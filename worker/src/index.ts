// The read-only feed endpoint (docs/FEED.md, #175). It serves the files the
// collector publishes under PREFIX in the rhythm-feed bucket and nothing else
// in that bucket. Requests carry no user data and the Worker logs nothing
// itself (#166 O16a).

export interface Env {
  FEED: R2Bucket;
  LIMITER: RateLimit;
  PREFIX: string;
}

// The rate limit's period in seconds (wrangler.toml).
const PERIOD = 60;

// A file the feed may name: the manifest and its signature, or one
// content-named file under segments/ or snapshots/ (FEED.md, Files).
const MUTABLE = new Map([
  ['manifest.json', 'application/json'],
  ['manifest.json.sig', 'application/json'],
]);
const IMMUTABLE = /^(segments|snapshots)\/[A-Za-z0-9_-][A-Za-z0-9._-]*\.(jsonl\.zst|vec)$/;

function contentType(path: string): string | undefined {
  const mutable = MUTABLE.get(path);
  if (mutable) return mutable;
  const match = IMMUTABLE.exec(path);
  if (!match || path.split('/').some((part) => part === '.' || part === '..')) return undefined;
  return match[2] === 'vec' ? 'application/octet-stream' : 'application/zstd';
}

// R2 also evaluates If-Match and If-Unmodified-Since from request headers; the
// feed answers only the two validators a client revalidates with. RFC 9110
// ignores If-Modified-Since when If-None-Match is present.
function conditional(request: Headers): Headers {
  const headers = new Headers();
  const etag = request.get('if-none-match');
  const since = request.get('if-modified-since');
  if (etag) headers.set('if-none-match', etag);
  else if (since) headers.set('if-modified-since', since);
  return headers;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      return new Response(null, { status: 405, headers: { allow: 'GET, HEAD' } });
    }
    const { success } = await env.LIMITER.limit({ key: request.headers.get('cf-connecting-ip') ?? '' });
    if (!success) return new Response(null, { status: 429, headers: { 'retry-after': String(PERIOD) } });

    // The URL parser has already resolved dot segments; percent-encoded
    // characters stay encoded and fail the path check.
    const path = new URL(request.url).pathname.slice(1);
    const type = contentType(path);
    if (!type) return new Response(null, { status: 404 });

    const object = await env.FEED.get(env.PREFIX + path, { onlyIf: conditional(request.headers) });
    if (!object) return new Response(null, { status: 404 });

    const headers = new Headers({
      'content-type': type,
      'cache-control': MUTABLE.has(path) ? 'no-cache' : 'public, max-age=31536000, immutable',
      etag: object.httpEtag,
      'last-modified': object.uploaded.toUTCString(),
    });
    // A failed precondition returns the object without a body.
    if (!('body' in object)) return new Response(null, { status: 304, headers });
    if (request.method === 'HEAD') {
      await object.body.cancel();
      headers.set('content-length', String(object.size));
      return new Response(null, { headers });
    }
    return new Response(object.body, { headers });
  },
} satisfies ExportedHandler<Env>;
