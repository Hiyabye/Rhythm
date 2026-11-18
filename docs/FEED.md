# Notice feed format

The v2 collector publishes Sogang's public notices as a signed feed, and the app mirrors it into its own SQLite database (#166). This document is the contract between the two. `crates/feed` (`rhythm-feed`) implements it, and both sides depend on that crate.

The feed is format **1.0**. It carries notice metadata, a 320-character body excerpt, extracted dates, resource links, embedding vectors and server categories. It never carries full bodies, original HTML or resource bytes (#166 D13). Nothing in the feed depends on who reads it, and requests carry no user data.

## Files

The Worker (`https://rhythm-feed.hiyabye0465.workers.dev`) serves these files from the public R2 bucket `rhythm-feed`, under the prefix the collector publishes to (`--publish`; `feed/` in production):

| Path | Contents | Mutable |
| --- | --- | --- |
| `manifest.json` | The current file list, board health and embedding identity | replaced each publish |
| `manifest.json.sig` | The manifest's Ed25519 signature | replaced with the manifest |
| `segments/…jsonl.zst` | Change records for one seq range | never |
| `segments/…vec` | The vectors those records reference | never |
| `snapshots/…jsonl.zst`, `snapshots/…vec` | The full current state through a seq | never |

The manifest gives each file's path, SHA-256 and size. Clients treat paths as opaque. A path is relative to the manifest. Its components are non-empty, contain only ASCII letters, digits, `.`, `_` and `-`, and are never `.` or `..`; the crate rejects any other path. The publisher names files after their role, seq range and content hash (`segments/run-…`, `segments/day-…`, `snapshots/…`), so a file name never refers to two different contents.

The Worker (`worker/`) serves exactly these files and answers 404 for any other path, including other prefixes of the bucket and percent-encoded or non-canonical paths:

- **Methods:** `GET` and `HEAD`; anything else is 405.
- **Caching:** `manifest.json` and `manifest.json.sig` are `cache-control: no-cache`; segments and snapshots are `public, max-age=31536000, immutable`. Every response carries `etag` and `last-modified`, and `If-None-Match` (or `If-Modified-Since` without it) returns 304 when the file is unchanged. Other preconditions are ignored.
- **Types:** `application/json` for the manifest and its signature, `application/zstd` for `.jsonl.zst`, `application/octet-stream` for `.vec`.
- **Rate limit:** a nominal 60 requests a minute per client address and Cloudflare location, counted approximately (several hundred passed in a minute in a live check); beyond it the Worker answers 429 with `retry-after: 60`. A client must handle 429 but cannot rely on a fixed threshold.
- No CORS headers: the feed is for the app, not for browser pages.

## Versioning

`format` is `major.minor`. A client reads every minor version of its own major and refuses another major (`Error::UnsupportedFormat`). The app then shows "앱을 업데이트하세요" and keeps its mirror as it is.

A minor version may add:

- fields to any object, which older clients ignore;
- record kinds, which older clients skip. LLM analysis (`analysis.upsert`, #166 D4) is meant to arrive this way.

Anything else is a major change: removing or renaming a field, changing a field's meaning or type, or adding a value to a closed set (`body_state`, resource `kind`, date `kind`).

## Manifest

[`crates/feed/src/fixtures/manifest.json`](../crates/feed/src/fixtures/manifest.json) is a complete example, and the crate's tests parse it.

| Field | Meaning |
| --- | --- |
| `format` | `"1.0"` |
| `generated_at` | When this manifest was published (RFC 3339 UTC) |
| `moved_to` | `null`, or the HTTPS base URL the feed moved to (#166 D22). The manifest there must verify like this one |
| `embedding.model`, `embedding.scope` | The Workers AI model and the scope (model + `rhythm_embed::config_hash`) of new vectors |
| `embedding.dimensions` | `1024` |
| `embedding.canary` | The latest drift check (#166 D27): `recorded`, `normal`, `warning`, `stopped`, or `null` before the first one |
| `boards[]` | Per board: `source_id`, `last_success_at`, `consecutive_failures`, `history_complete` (#166 D19) |
| `snapshot` | `through_seq`, `records` and `vectors` (`{path, sha256, size}`; `vectors` may be `null`) |
| `segments[]` | `from_seq`, `to_seq`, `records`, `vectors`, in seq order |

The crate also checks the manifest's structure:

- Segments are contiguous: each `from_seq` is the previous segment's `to_seq + 1`, and `from_seq ≤ to_seq`.
- When segments are listed, `snapshot.through_seq` lies between the first segment's `from_seq - 1` and the last segment's `to_seq`, so a client starting from the snapshot can continue with the segments.
- Every SHA-256 is 64 lowercase hex characters.

## Signature

`manifest.json.sig` is JSON with two fields:

```json
{"key_id":"3f1c9a0d5e7b2468","signature":"<128 hex characters>"}
```

- **`signature`** is a standard Ed25519 signature (RFC 8032) over the exact bytes of `manifest.json`.
- **`key_id`** is the first 8 bytes of the public key's SHA-256, in hex.
- **Trusted keys:** the app compiles in a list of them and accepts a manifest only when the key named by `key_id` is on that list and the signature verifies. Shipping a new key one release before the publisher switches to it rotates keys without breaking clients.
- **Key handling:** the maintainer generates the signing key as an Ed25519 PKCS#8 document (v1 from OpenSSL, or v2) and backs it up. It reaches the collector only as a secret of the `collector` environment. Losing it means clients accept no new manifests until an app update ships a new key.

The signature covers the manifest. The manifest covers every other file through its size and SHA-256, so a leaked R2 token cannot inject content.

## Records

A records file is UTF-8 JSON Lines compressed with zstd. Each line is one object with a `seq`, a `kind` and the kind's fields. A file may not decompress beyond 256 MiB.

**`notice.upsert`** carries the current revision of one notice. It replaces every field the client holds for that notice.

| Field | Type | Meaning |
| --- | --- | --- |
| `source_id`, `external_id` | string | The notice identity (as in v1) |
| `revision_hash` | string | Changes with every meaningful revision: `rhythm_feed::revision_hash`, the SHA-256 (lowercase hex) of the compact JSON array `[canonical_url, title, author, category, published_at, body_text, [[url, filename, kind], …]]` of the parsed page, so a client can tell whether a body it fetched is this revision |
| `canonical_url` | string | The original page; the app fetches the body from it when the notice is opened (#166 D14) |
| `title`, `author`, `category` | string, string?, string? | As the board shows them |
| `published_at` | string | `YYYY-MM-DD` |
| `modified_at` | string? | The site's own change time (RFC 3339 UTC), when it reports one |
| `body_state` | `normal` \| `unsupported` | `unsupported`: the body could not be read (an iframe, or a scanned or link-only PDF); the notice is metadata only |
| `excerpt` | string | The first 320 characters (Unicode scalar values) of the body text |
| `resources[]` | `{url, filename, kind}` | `kind` is `attachment` or `image`. These are links only; the app fetches them under its own policy |
| `dates[]` | `{day, time?, end_day?, end_time?, label, kind}` | Dates extracted from the full body (#166 O1); `kind` is `deadline`, `start` or `date` |
| `dates_version` | integer | The `rhythm_sources::dates::EXTRACTOR_VERSION` that produced `dates` |
| `historical` | bool | Found by the history import, not as a new notice: never notified |
| `first_seen_at`, `revised_at` | string | Server times (RFC 3339 UTC) of the first sighting and of the last meaningful revision |

**`embedding.upsert`** has `source_id`, `external_id`, `revision_hash`, `scope` and `vector`. `vector` is the index of the notice's vector in the vectors file published with the same records file.

**`classification.upsert`** has `source_id`, `external_id`, `revision_hash`, `scope`, `category` (one of `rhythm_embed::CATEGORIES`), `score`, `margin` and `low_confidence` (#166 O9).

**`notice.remove`** has `source_id`, `external_id` and `removed_at`. It is sent once the collector has confirmed the notice was deleted at its source (#166 D24).

The embedding and classification of a revision follow its `notice.upsert`, either in the same file or in a later one. A client applies them only while `revision_hash` matches the notice revision it holds.

## Vectors

A vectors file holds whole vectors and nothing else (#166 O19a). Each vector is 1,028 bytes: a little-endian f32 `scale`, then 1,024 int8 values `q`.

- **Writing:** the publisher sets `scale = max|x| / 127` and `q = round(x / scale)` (half away from zero).
- **Reading:** the client computes `q × scale` and renormalizes to unit length.
- **Accuracy:** #170 measured this encoding within the local CPU-vs-Metal baseline.
- **Validity:** a vector with a non-finite, zero or negative scale, or all-zero values, is invalid.

## Segments, snapshots and retention

`seq` increases by one for every record the publisher writes.

- **A segment** holds exactly the seqs from `from_seq` to `to_seq`, in order.
- **A snapshot** holds the upserts of every notice not removed, each with the seq that last set it, up to `through_seq`. Seqs in a snapshot increase but have gaps, and a snapshot never holds a `notice.remove`.
- **When files are written** (user decisions, 2026-11-18):
  - Each collector run that changes something publishes one run segment.
  - At the first publish of a new UTC day, the previous day's run segments are merged into one daily segment with the same seq range, so no seq is lost. A snapshot is written at the same time.
  - The manifest then lists the daily segment in place of the run segments. The run segment files are deleted a day later, so a client that read the previous manifest can still download them.
- **Retention:** daily segments stay listed for 30 days. A replaced snapshot, like a compacted run segment, is deleted a day after it leaves the manifest (user decision, 2026-11-18), so the manifest lists only the newest snapshot. A day without changes keeps the same snapshot file.
- **Publish order** (user decision, 2026-11-18): a run uploads its new files, records their seqs in its state, uploads the state, and only then uploads the manifest and its signature. A failed state upload therefore leaves only unlisted files, which the next run deletes before it assigns the same seqs again. A run refuses to publish when the live manifest lists a seq its state has not assigned (a rolled-back state), when the live manifest is not older than its clock, or when the live manifest does not verify with its key.
- **Restart** (user decisions, 2026-11-18): after a state rollback the maintainer runs the collector once with `--reset-feed`. The state forgets what it published, and the run assigns every current record a new seq after the last seq the live manifest lists. It publishes no segment: the new manifest lists only a snapshot of those records, whose `through_seq` lies beyond every seq of the old feed, so any cursor a client holds falls before it and the client applies the snapshot (step 4 below), which brings it to the rolled-back state. Later runs continue the seqs as usual, so a seq never names two records. The old feed's files are retired as the restart publishes and deleted a day later. A restart needs a live manifest that verifies and at least one notice to snapshot.

## Applying the feed

1. Download `manifest.json` and `manifest.json.sig`, then verify them (`rhythm_feed::verify`).
2. Refuse a manifest whose `generated_at` is older than the last one accepted, so an old signed manifest cannot roll the mirror back.
3. Follow `moved_to`, if set.
4. Choose where to start:
   - **A cursor `c` with `c + 1` inside the listed segments:** apply the segments from the one that holds `c + 1`.
   - **A cursor equal to the newest listed seq** (the last segment's `to_seq`, or `through_seq` when no segment is listed): nothing to apply.
   - **Any other cursor, or none:** apply the snapshot. Notices the client holds but the snapshot lacks were removed meanwhile; delete them as for `notice.remove`. Embeddings and classifications the snapshot lacks are dropped too, since it carries every current one. Then apply the segments after `through_seq`.
5. Check each downloaded file's size and SHA-256 against the manifest (`FileRef::check`) before decoding it.
6. Apply a file's records in one transaction, then store its last seq as the cursor.

Further rules:

- **Notifications:** a first sync, and any notice marked `historical`, never notifies.
- **Removals:** a removal deletes the notice and everything derived from it. A saved notice keeps its row with a "원문 삭제됨" note (#166 client behavior).
- **Plain text:** all text in the feed is plain text. The app escapes it and never renders it as HTML.
