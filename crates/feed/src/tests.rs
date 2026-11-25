use super::*;
use manifest::{key_id, sha256_hex, Signature};
use record::{
    decode_segment, decode_snapshot, BodyState, Classification, DateKind, Embedding, Event,
    EventRemoval, Meal, MealCell, MealDay, MealStatus, MealWeek, Notice, NoticeDate, Removal,
    Resource, ResourceKind,
};
use ring::{rand::SystemRandom, signature::Ed25519KeyPair};
use test_keys::unhex;

const EXAMPLE: &str = include_str!("fixtures/manifest.json");

fn notice(id: &str) -> Notice {
    Notice {
        source_id: "sogang-academic".into(),
        external_id: id.into(),
        revision_hash: format!("rev-{id}"),
        canonical_url: format!("https://www.sogang.ac.kr/ko/detail/{id}"),
        title: "2026학년도 2학기 \"전공\" 신청 안내".into(),
        author: Some("학사지원팀".into()),
        category: None,
        published_at: "2026-11-02".into(),
        modified_at: Some("2026-11-03T01:02:03.000Z".into()),
        body_state: BodyState::Normal,
        excerpt: excerpt("신청 기간은 11월 10일부터\n11월 14일까지입니다."),
        body_hash: Some(body_hash("신청 기간은 11월 10일부터\n11월 14일까지입니다.")),
        resources: vec![Resource {
            url: "https://www.sogang.ac.kr/file/1".into(),
            filename: "안내문.pdf".into(),
            kind: ResourceKind::Attachment,
        }],
        dates: vec![NoticeDate {
            day: "2026-11-14".into(),
            time: Some("17:00".into()),
            end_day: None,
            end_time: None,
            label: "신청 마감".into(),
            kind: DateKind::Deadline,
        }],
        dates_version: 3,
        historical: false,
        first_seen_at: "2026-11-02T05:00:00.000Z".into(),
        revised_at: "2026-11-03T05:00:00.000Z".into(),
    }
}

fn every_kind() -> Vec<(u64, Record)> {
    vec![
        (7, Record::Notice(Box::new(notice("1")))),
        (
            8,
            Record::Embedding(Embedding {
                source_id: "sogang-academic".into(),
                external_id: "1".into(),
                revision_hash: "rev-1".into(),
                scope: "model#hash".into(),
                vector: 0,
            }),
        ),
        (
            9,
            Record::Classification(Classification {
                source_id: "sogang-academic".into(),
                external_id: "1".into(),
                revision_hash: "rev-1".into(),
                scope: "model#hash".into(),
                category: "학사".into(),
                score: 0.875,
                margin: 0.0625,
                low_confidence: false,
            }),
        ),
        (
            10,
            Record::Remove(Removal {
                source_id: "sogang-academic".into(),
                external_id: "2".into(),
                removed_at: "2026-11-18T00:00:00.000Z".into(),
            }),
        ),
    ]
}

fn records(entries: &[Entry]) -> Vec<(u64, Record)> {
    entries
        .iter()
        .map(|entry| (entry.seq, entry.record.clone().unwrap()))
        .collect()
}

/// JSONL lines, compressed as the publisher would.
fn compressed(lines: &[&str]) -> Vec<u8> {
    zstd::encode_all(format!("{}\n", lines.join("\n")).as_bytes(), 3).unwrap()
}

#[test]
fn records_round_trip_through_a_segment() {
    let entries = every_kind();
    let bytes = record::encode(&entries).unwrap();
    assert_eq!(records(&decode_segment(&bytes, 7, 10).unwrap()), entries);
    // Each line is plain JSON with the seq and kind beside the fields.
    let text = String::from_utf8(zstd::decode_all(bytes.as_slice()).unwrap()).unwrap();
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["seq"], 7);
    assert_eq!(first["kind"], "notice.upsert");
    assert_eq!(first["body_state"], "normal");
    assert_eq!(first["dates"][0]["kind"], "deadline");
    assert_eq!(first["resources"][0]["kind"], "attachment");
    assert_eq!(first["body_hash"].as_str().map(str::len), Some(64));
}

#[test]
fn a_segment_must_hold_exactly_its_seqs() {
    let bytes = record::encode(&every_kind()).unwrap();
    for (from, to) in [(6, 10), (7, 11), (8, 10), (7, 9)] {
        assert!(decode_segment(&bytes, from, to).is_err(), "{from}..{to}");
    }
    let mut gap = every_kind();
    gap[2].0 = 11;
    let bytes = record::encode(&gap).unwrap();
    assert!(decode_segment(&bytes, 7, 11).is_err());
}

#[test]
fn a_snapshot_holds_increasing_upserts_up_to_its_seq() {
    let upserts: Vec<_> = every_kind().into_iter().take(3).collect();
    let bytes = record::encode(&upserts).unwrap();
    assert_eq!(records(&decode_snapshot(&bytes, 9).unwrap()), upserts);
    assert_eq!(
        decode_snapshot(&bytes, 100).unwrap().len(),
        3,
        "gaps are allowed"
    );
    assert!(decode_snapshot(&bytes, 8).is_err(), "beyond its seq");
    let mut repeated = upserts.clone();
    repeated[1].0 = 7;
    assert!(decode_snapshot(&record::encode(&repeated).unwrap(), 9).is_err());
    assert!(
        decode_snapshot(&record::encode(&every_kind()).unwrap(), 10).is_err(),
        "a removal"
    );
}

#[test]
fn unknown_kinds_and_fields_are_skipped_but_known_ones_must_parse() {
    let mut line: serde_json::Value =
        serde_json::to_value(Record::Notice(Box::new(notice("1")))).unwrap();
    line["seq"] = 1.into();
    line["summary"] = "a field from a later minor version".into();
    let bytes = compressed(&[
        &line.to_string(),
        r#"{"seq":2,"kind":"analysis.upsert","summary":"later"}"#,
    ]);
    let entries = decode_segment(&bytes, 1, 2).unwrap();
    assert_eq!(
        entries[0].record,
        Some(Record::Notice(Box::new(notice("1"))))
    );
    assert_eq!(
        entries[1],
        Entry {
            seq: 2,
            record: None
        }
    );

    for bad in [
        r#"{"seq":1,"kind":"notice.upsert","title":"missing fields"}"#,
        r#"{"kind":"notice.remove","source_id":"a","external_id":"1","removed_at":"x"}"#,
        r#"{"seq":1,"source_id":"a"}"#,
        r#"{"seq":1,"kind":"notice.upsert","#,
        r#"{"seq":-1,"kind":"x"}"#,
    ] {
        assert!(decode_segment(&compressed(&[bad]), 1, 1).is_err(), "{bad}");
    }
    // A 1.0 publisher sends no `body_hash`, and `None` is not written.
    let mut old = line.clone();
    old.as_object_mut().unwrap().remove("summary");
    old.as_object_mut().unwrap().remove("body_hash");
    let entries = decode_segment(&compressed(&[&old.to_string()]), 1, 1).unwrap();
    let Some(Record::Notice(decoded)) = &entries[0].record else {
        panic!("a notice");
    };
    assert_eq!(decoded.body_hash, None);
    assert_eq!(
        serde_json::to_value(Record::Notice(decoded.clone())).unwrap(),
        {
            let mut written = old.clone();
            written.as_object_mut().unwrap().remove("seq");
            written
        }
    );

    let mut state: serde_json::Value = line.clone();
    state["body_state"] = "iframe".into();
    assert!(decode_segment(&compressed(&[&state.to_string()]), 1, 1).is_err());
}

#[test]
fn decoding_stops_at_the_size_limit() {
    let bytes = record::encode(&every_kind()).unwrap();
    let size = zstd::decode_all(bytes.as_slice()).unwrap().len() as u64;
    assert_eq!(record::decode(&bytes, size).unwrap().len(), 4);
    assert!(record::decode(&bytes, size - 1).is_err());
    assert!(record::decode(b"not zstd", 1_000).is_err());
}

#[test]
fn excerpts_count_characters() {
    let body = "가".repeat(EXCERPT_CHARS + 5);
    assert_eq!(excerpt(&body).chars().count(), EXCERPT_CHARS);
    assert_eq!(excerpt("짧은 본문"), "짧은 본문");
}

fn unit(seed: u32) -> Vec<f32> {
    // A deterministic, irregular vector.
    let values: Vec<f32> = (0..DIMENSIONS as u32)
        .map(|i| ((i * 7919 + seed * 104_729) % 2003) as f32 - 1001.0)
        .collect();
    let norm = values.iter().map(|x| x * x).sum::<f32>().sqrt();
    values.iter().map(|x| x / norm).collect()
}

#[test]
fn vectors_are_symmetric_int8_with_a_scale() {
    let mut vector = vec![0f32; DIMENSIONS];
    vector[0] = 1.0;
    vector[1] = -0.5;
    vector[2] = 0.25;
    let bytes = vector::quantize(&vector).unwrap();
    assert_eq!(
        f32::from_le_bytes(bytes[..4].try_into().unwrap()),
        1.0 / 127.0
    );
    // -63.5 rounds away from zero; 31.75 rounds to 32.
    assert_eq!(&bytes[4..8], &[127, (-64i8) as u8, 32, 0]);
    assert_eq!(bytes.len(), VECTOR_BYTES);

    let originals = [unit(1), unit(2), unit(3)];
    let file = vector::encode(
        &originals
            .iter()
            .map(|v| vector::quantize(v).unwrap())
            .collect::<Vec<_>>(),
    );
    assert_eq!(file.len(), 3 * VECTOR_BYTES);
    let decoded = vector::decode(&file).unwrap();
    for (original, decoded) in originals.iter().zip(&decoded) {
        let cosine: f32 = original.iter().zip(decoded).map(|(a, b)| a * b).sum();
        let norm: f32 = decoded.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(cosine > 0.9998, "{cosine}");
        assert!((norm - 1.0).abs() < 1e-5);
    }
}

#[test]
fn malformed_vectors_are_rejected() {
    assert!(vector::quantize(&[0.0; DIMENSIONS]).is_err());
    assert!(vector::quantize(&[1.0; 512]).is_err());
    let mut nan = unit(1);
    nan[3] = f32::NAN;
    assert!(vector::quantize(&nan).is_err());

    let good = vector::quantize(&unit(1)).unwrap();
    assert!(vector::decode(&good[..VECTOR_BYTES - 1]).is_err());
    for scale in [f32::NAN, 0.0, -1.0, f32::INFINITY] {
        let mut bad = good;
        bad[..4].copy_from_slice(&scale.to_le_bytes());
        assert!(vector::decode(&bad).is_err(), "{scale}");
    }
    let mut zero = [0u8; VECTOR_BYTES];
    zero[..4].copy_from_slice(&1f32.to_le_bytes());
    assert!(vector::decode(&zero).is_err());
    assert!(vector::decode(&[]).unwrap().is_empty());
}

fn event(uid: &str) -> Event {
    Event {
        calendar_id: "sogang-academic-calendar".into(),
        uid: uid.into(),
        title: "학기말성적 확인".into(),
        start_day: "2025-12-30".into(),
        end_day: "2026-01-02".into(),
    }
}

#[test]
fn calendar_events_are_records_and_snapshots_hold_only_their_upserts() {
    let entries = vec![
        (3, Record::Event(event("a"))),
        (
            4,
            Record::EventRemove(EventRemoval {
                calendar_id: "sogang-academic-calendar".into(),
                uid: "b".into(),
                removed_at: "2026-11-25T00:00:00.000Z".into(),
            }),
        ),
    ];
    let bytes = record::encode(&entries).unwrap();
    assert_eq!(records(&decode_segment(&bytes, 3, 4).unwrap()), entries);
    let text = String::from_utf8(zstd::decode_all(bytes.as_slice()).unwrap()).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["kind"], "event.upsert");
    assert_eq!(lines[0]["end_day"], "2026-01-02");
    assert_eq!(lines[1]["kind"], "event.remove");
    let upsert = record::encode(&entries[..1]).unwrap();
    assert_eq!(decode_snapshot(&upsert, 3).unwrap().len(), 1);
    assert!(decode_snapshot(&bytes, 4).is_err(), "an event removal");
}

fn meal(status: MealCell, items: &[&str], time: Option<&str>) -> Meal {
    Meal {
        status,
        items: items.iter().map(|item| item.to_string()).collect(),
        time: time.map(str::to_string),
    }
}

#[test]
fn meal_weeks_are_records_in_segments_and_snapshots() {
    let unknown = meal(MealCell::Unknown, &[], None);
    let day = MealDay {
        day: "2026-11-24".into(),
        breakfast_korean: meal(MealCell::Available, &["전복죽", "동치미"], None),
        breakfast_simple: unknown.clone(),
        cup_rice: meal(MealCell::Available, &["돈까스덮밥"], Some("11:40")),
        dinner: meal(MealCell::Closed, &[], None),
    };
    let week = MealWeek {
        week: "2026-11-23".into(),
        source_id: "dorm-menu".into(),
        external_id: "313348".into(),
        image_url: "https://new-cms.sogang.ac.kr/CrossEditor/binary/images/000047/a.jpg".into(),
        status: MealStatus::Ok,
        low_resolution: false,
        days: vec![day; 7],
        breakfast_common: Some(meal(MealCell::Available, &["그린샐러드"], None)),
        drinks: Some(unknown),
    };
    let entries = vec![(5, Record::MealWeek(Box::new(week)))];
    let bytes = record::encode(&entries).unwrap();
    assert_eq!(records(&decode_segment(&bytes, 5, 5).unwrap()), entries);
    // A week is an upsert: snapshots carry it.
    assert_eq!(records(&decode_snapshot(&bytes, 5).unwrap()), entries);
    let text = String::from_utf8(zstd::decode_all(bytes.as_slice()).unwrap()).unwrap();
    let line: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(line["kind"], "meal.week");
    assert_eq!(line["status"], "ok");
    assert_eq!(line["low_resolution"], false);
    assert_eq!(line["days"][0]["cup_rice"]["time"], "11:40");
    assert_eq!(line["days"][0]["dinner"]["status"], "closed");
    assert_eq!(line["days"][0]["breakfast_simple"]["status"], "unknown");
    assert_eq!(line["breakfast_common"]["items"][0], "그린샐러드");
}

#[test]
fn a_1_2_manifest_without_meals_still_parses() {
    let mut value: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
    value["format"] = "1.2".into();
    value.as_object_mut().unwrap().remove("meals");
    let manifest = Manifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(manifest.meals.is_empty());
    assert_eq!(example().meals[0].source_id, "dorm-menu");
}

#[test]
fn a_1_1_manifest_without_calendars_still_parses() {
    let mut value: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
    value["format"] = "1.1".into();
    value.as_object_mut().unwrap().remove("calendars");
    let manifest = Manifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(manifest.calendars.is_empty());
    assert_eq!(
        example().calendars[0].calendar_id,
        "sogang-academic-calendar"
    );
}

fn signer() -> Signer {
    let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Signer::from_pkcs8(document.as_ref()).unwrap()
}

fn example() -> Manifest {
    Manifest::parse(EXAMPLE.as_bytes()).unwrap()
}

#[test]
fn the_documented_example_manifest_parses() {
    let manifest = example();
    assert_eq!(manifest.format, FORMAT);
    assert_eq!(manifest.snapshot.through_seq, 51210);
    assert_eq!(manifest.segments.len(), 2);
    let vectors = manifest.snapshot.vectors.unwrap();
    assert_eq!(vectors.size % VECTOR_BYTES as u64, 0);
}

#[test]
fn a_signed_manifest_verifies_only_unchanged_and_with_a_trusted_key() {
    let other = signer().public_key();
    let signer = signer();
    let manifest = EXAMPLE.as_bytes();
    let signature = signer.sign(manifest);
    let trusted = [other, signer.public_key()];
    assert_eq!(verify(manifest, &signature, &trusted).unwrap(), example());

    assert_eq!(
        verify(manifest, &signature, &[other]),
        Err(Error::BadSignature)
    );
    let mut changed = manifest.to_vec();
    let at = EXAMPLE.find("51214").unwrap();
    changed[at + 4] = b'5';
    assert_eq!(
        verify(&changed, &signature, &trusted),
        Err(Error::BadSignature)
    );

    let parsed: Signature = serde_json::from_slice(&signature).unwrap();
    assert_eq!(parsed.key_id, key_id(&signer.public_key()));
    assert_eq!(parsed.key_id.len(), 16);
    let mut flipped = parsed.clone();
    let last = if flipped.signature.ends_with('0') {
        "1"
    } else {
        "0"
    };
    flipped.signature.replace_range(127.., last);
    let mut odd = parsed.clone();
    odd.signature.pop();
    let mut wrong_key = parsed.clone();
    wrong_key.key_id = key_id(&other);
    for bad in [flipped, odd, wrong_key] {
        let bytes = serde_json::to_vec(&bad).unwrap();
        assert_eq!(verify(manifest, &bytes, &trusted), Err(Error::BadSignature));
    }
    assert_eq!(
        verify(manifest, b"not json", &trusted),
        Err(Error::BadSignature)
    );
}

#[test]
fn standard_ed25519_keys_and_signatures_interoperate() {
    // RFC 8032, section 7.1, test 1 (empty message).
    let seed = unhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
    let public: [u8; 32] =
        unhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .try_into()
            .unwrap();
    let signature = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
                     5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
    let signer = Signer::from_pkcs8(&test_keys::pkcs8(&seed)).unwrap();
    assert_eq!(signer.public_key(), public);
    let sig = serde_json::to_vec(&Signature {
        key_id: key_id(&public),
        signature: signature.into(),
    })
    .unwrap();
    // The signature passes; the empty "manifest" then fails to parse.
    assert!(matches!(
        verify(b"", &sig, &[public]),
        Err(Error::Invalid(_))
    ));
    assert_eq!(signer.sign(b""), sig);
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// The example with one JSON edit, parsed.
fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> Result<Manifest, Error> {
    let mut value: serde_json::Value = serde_json::from_str(EXAMPLE).unwrap();
    edit(&mut value);
    Manifest::parse(value.to_string().as_bytes())
}

#[test]
fn the_major_version_decides_whether_a_manifest_is_readable() {
    assert!(edited(|m| m["format"] = "1.7".into()).is_ok());
    let newer = edited(|m| {
        m["format"] = "2.0".into();
        m["segments"] = "a shape this version doesn't know".into();
    });
    assert_eq!(newer, Err(Error::UnsupportedFormat("2.0".into())));
    for format in ["1", "1.", "x.0", "01.0a", "", "1.0.1"] {
        let result = edited(|m| m["format"] = format.into());
        assert!(matches!(result, Err(Error::Invalid(_))), "{format:?}");
    }
    // A later minor may add fields.
    assert!(edited(|m| m["analysis"] = serde_json::json!({"prompt": 1})).is_ok());
}

#[test]
fn manifests_with_unsafe_files_or_gaps_are_rejected() {
    for path in [
        "../x",
        "/abs",
        "a//b",
        "a/./b",
        "a b",
        "a\\b",
        "",
        "x?y",
        "세그먼트",
    ] {
        let result = edited(|m| m["segments"][0]["records"]["path"] = path.into());
        assert!(matches!(result, Err(Error::Invalid(_))), "{path:?}");
    }
    let result = edited(|m| m["snapshot"]["records"]["sha256"] = "ABC".into());
    assert!(matches!(result, Err(Error::Invalid(_))));
    let gap = edited(|m| m["segments"][1]["from_seq"] = 51212.into());
    assert!(matches!(gap, Err(Error::Invalid(_))));
    let backwards = edited(|m| m["segments"][1]["to_seq"] = 51210.into());
    assert!(matches!(backwards, Err(Error::Invalid(_))));
    for through in [50899, 51215] {
        let outside = edited(|m| m["snapshot"]["through_seq"] = through.into());
        assert!(matches!(outside, Err(Error::Invalid(_))), "{through}");
    }
    assert!(edited(|m| m["snapshot"]["through_seq"] = 50900.into()).is_ok());
    assert!(edited(|m| m["segments"] = serde_json::json!([])).is_ok());
    let http = edited(|m| m["moved_to"] = "http://example.com/feed/".into());
    assert!(matches!(http, Err(Error::Invalid(_))));
    assert!(edited(|m| m["moved_to"] = "https://feed.example/".into()).is_ok());
}

#[test]
fn downloaded_files_must_match_their_reference() {
    let bytes = b"segment bytes";
    let file = FileRef::new("segments/1-2.jsonl.zst".into(), bytes);
    assert!(file.check(bytes).is_ok());
    assert!(file.check(b"segment bytez").is_err());
    assert!(file.check(b"segment byte").is_err());
}

#[test]
fn revision_hash_is_the_sha256_of_the_json_fields() {
    // Computed independently: SHA-256 of the compact JSON array.
    let hash = revision_hash(&Revision {
        canonical_url: "https://www.sogang.ac.kr/ko/detail/7",
        title: "장학 공지 \"안내\"",
        author: None,
        category: Some("학사"),
        published_at: "2026-11-02",
        body_text: "본문\n둘째 줄",
        resources: &[("https://www.sogang.ac.kr/a.pdf", "신청서.pdf", "attachment")],
    });
    assert_eq!(
        hash,
        "f9000578634c7565ceb9b888f6f24630f9e3990843f3b0cd165efea00f425b9e"
    );
}

#[test]
fn body_hash_is_the_sha256_of_the_body_text() {
    // Computed independently with `shasum -a 256`.
    assert_eq!(
        body_hash("본문\n둘째 줄"),
        "101421e941d699012a0b4f42eb5747e6d59b214c3b3bb6a92c832188631e289c"
    );
}
