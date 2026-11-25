//! Change records: one JSON object per line with its `seq` and `kind`,
//! compressed with zstd. A segment holds every seq of its range in order; a
//! snapshot holds the current upserts, each with the seq that last set it.
use crate::{Error, MAX_DECODED_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;

/// zstd level for published files: a snapshot is written once a day.
const LEVEL: i32 = 19;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Record {
    #[serde(rename = "notice.upsert")]
    Notice(Box<Notice>),
    #[serde(rename = "embedding.upsert")]
    Embedding(Embedding),
    #[serde(rename = "classification.upsert")]
    Classification(Classification),
    #[serde(rename = "notice.remove")]
    Remove(Removal),
    /// Format 1.2: an academic calendar event (#319).
    #[serde(rename = "event.upsert")]
    Event(Event),
    #[serde(rename = "event.remove")]
    EventRemove(EventRemoval),
    /// Format 1.3: one week of the dormitory menu (#318).
    #[serde(rename = "meal.week")]
    MealWeek(Box<MealWeek>),
}

/// A notice's current revision without its body: the first `EXCERPT_CHARS`
/// characters stand in for it (#166 D13, O2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub source_id: String,
    pub external_id: String,
    pub revision_hash: String,
    pub canonical_url: String,
    pub title: String,
    pub author: Option<String>,
    pub category: Option<String>,
    /// `YYYY-MM-DD`, as the board shows it.
    pub published_at: String,
    /// The site's own change time (RFC 3339 UTC), when it reports one.
    pub modified_at: Option<String>,
    pub body_state: BodyState,
    pub excerpt: String,
    /// `body_hash` of the full body text (format 1.1); `None` from a 1.0
    /// publisher. Tells a client whether the body changed beyond the excerpt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>,
    pub resources: Vec<Resource>,
    pub dates: Vec<NoticeDate>,
    /// `rhythm_sources::dates::EXTRACTOR_VERSION` of `dates`.
    pub dates_version: u32,
    /// Found by the history import rather than as a new notice: never notified.
    pub historical: bool,
    pub first_seen_at: String,
    /// When the server last saw a meaningful revision.
    pub revised_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyState {
    Normal,
    /// The body could not be read (an iframe or a scanned PDF); the notice is
    /// metadata only and the app may fetch the original when opened.
    Unsupported,
}

/// A reference only; the feed never carries the bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resource {
    pub url: String,
    pub filename: String,
    pub kind: ResourceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Attachment,
    Image,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoticeDate {
    pub day: String,
    pub time: Option<String>,
    pub end_day: Option<String>,
    pub end_time: Option<String>,
    pub label: String,
    pub kind: DateKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateKind {
    Deadline,
    Start,
    Date,
}

/// A notice revision's vector: `vector` indexes the vectors file published
/// with the same records file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Embedding {
    pub source_id: String,
    pub external_id: String,
    pub revision_hash: String,
    pub scope: String,
    pub vector: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub source_id: String,
    pub external_id: String,
    pub revision_hash: String,
    pub scope: String,
    pub category: String,
    pub score: f32,
    pub margin: f32,
    pub low_confidence: bool,
}

/// The notice was deleted at its source (#166 D24).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Removal {
    pub source_id: String,
    pub external_id: String,
    pub removed_at: String,
}

/// A calendar event's current state (format 1.2). Dates only: the source
/// calendar is all-day, and times appear only inside titles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// The calendar, `rhythm_sources::calendar::ACADEMIC_CALENDAR_ID`.
    pub calendar_id: String,
    /// The event's identity within its calendar (the ICS `UID`).
    pub uid: String,
    pub title: String,
    /// `YYYY-MM-DD`.
    pub start_day: String,
    /// `YYYY-MM-DD`, inclusive; equal to `start_day` for a one-day event.
    pub end_day: String,
}

/// The event was dropped from its calendar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRemoval {
    pub calendar_id: String,
    pub uid: String,
    pub removed_at: String,
}

/// One week of the Bellarmine dormitory menu, read by OCR from the image of
/// the menu notice (format 1.3, #318). It replaces everything a client holds
/// for that week, including a week it read from an older post.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MealWeek {
    /// The week's Monday, `YYYY-MM-DD` (KST).
    pub week: String,
    /// The menu notice the week was read from.
    pub source_id: String,
    pub external_id: String,
    /// The original menu image; a client always links it.
    pub image_url: String,
    pub status: MealStatus,
    /// Monday to Sunday when `ok`; empty when `unreadable`.
    pub days: Vec<MealDay>,
    /// The rows printed once for the whole week; `None` when `unreadable`.
    pub breakfast_common: Option<Meal>,
    pub drinks: Option<Meal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MealStatus {
    Ok,
    /// The image could not be read with certainty: show only its link.
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MealDay {
    /// `YYYY-MM-DD`.
    pub day: String,
    /// 한식 breakfast.
    pub breakfast_korean: Meal,
    /// 간편식 breakfast.
    pub breakfast_simple: Meal,
    /// 컵밥.
    pub cup_rice: Meal,
    /// 석식.
    pub dinner: Meal,
}

/// One cell of the menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meal {
    pub status: MealCell,
    /// Items as printed (source typos kept); empty unless `available`.
    pub items: Vec<String>,
    /// `HH:MM` when the cell prints one time, such as a 컵밥 serving time.
    pub time: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MealCell {
    Available,
    /// The menu says the meal is not served, such as `토요일석식미운영`.
    Closed,
    /// Blank, merged across days, or not readable.
    Unknown,
}

/// One decoded line. `record` is `None` for a kind this version doesn't know,
/// which a client skips (a minor format addition).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub record: Option<Record>,
}

#[derive(Serialize)]
struct Line<'a> {
    seq: u64,
    #[serde(flatten)]
    record: &'a Record,
}

/// JSONL, zstd-compressed.
pub fn encode(entries: &[(u64, Record)]) -> Result<Vec<u8>, Error> {
    let mut text = Vec::new();
    for (seq, record) in entries {
        serde_json::to_writer(&mut text, &Line { seq: *seq, record })
            .map_err(|e| Error::Invalid(e.to_string()))?;
        text.push(b'\n');
    }
    zstd::encode_all(text.as_slice(), LEVEL).map_err(|e| Error::Invalid(e.to_string()))
}

/// A segment: exactly the seqs `from..=to`, in order.
pub fn decode_segment(bytes: &[u8], from: u64, to: u64) -> Result<Vec<Entry>, Error> {
    let entries = decode(bytes, MAX_DECODED_BYTES)?;
    let expected = (from..=to).map(Some);
    if !entries.iter().map(|e| Some(e.seq)).eq(expected) {
        return Err(Error::Invalid(format!(
            "segment does not hold exactly seq {from} to {to}"
        )));
    }
    Ok(entries)
}

/// A snapshot: upserts only, seqs increasing and at most `through`.
pub fn decode_snapshot(bytes: &[u8], through: u64) -> Result<Vec<Entry>, Error> {
    let entries = decode(bytes, MAX_DECODED_BYTES)?;
    if entries.windows(2).any(|pair| pair[0].seq >= pair[1].seq)
        || entries.last().is_some_and(|last| last.seq > through)
    {
        return Err(Error::Invalid(format!(
            "snapshot seqs are not increasing up to {through}"
        )));
    }
    if entries.iter().any(|entry| {
        matches!(
            entry.record,
            Some(Record::Remove(_) | Record::EventRemove(_))
        )
    }) {
        return Err(Error::Invalid("snapshot holds a removal".into()));
    }
    Ok(entries)
}

pub(crate) fn decode(bytes: &[u8], limit: u64) -> Result<Vec<Entry>, Error> {
    let mut text = Vec::new();
    zstd::stream::read::Decoder::new(bytes)
        .map_err(|e| Error::Invalid(e.to_string()))?
        .take(limit + 1)
        .read_to_end(&mut text)
        .map_err(|e| Error::Invalid(format!("records do not decompress: {e}")))?;
    if text.len() as u64 > limit {
        return Err(Error::Invalid("records are too large".into()));
    }
    let text =
        std::str::from_utf8(&text).map_err(|_| Error::Invalid("records are not UTF-8".into()))?;
    text.lines()
        .enumerate()
        .map(|(number, line)| {
            line_entry(line).map_err(|e| Error::Invalid(format!("record line {}: {e}", number + 1)))
        })
        .collect()
}

fn line_entry(line: &str) -> Result<Entry, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let seq = value["seq"].as_u64().ok_or("no seq")?;
    let kind = value["kind"].as_str().ok_or("no kind")?;
    let known = matches!(
        kind,
        "notice.upsert"
            | "embedding.upsert"
            | "classification.upsert"
            | "notice.remove"
            | "event.upsert"
            | "event.remove"
            | "meal.week"
    );
    let record = if known {
        Some(serde_json::from_value(value).map_err(|e| e.to_string())?)
    } else {
        None
    };
    Ok(Entry { seq, record })
}
