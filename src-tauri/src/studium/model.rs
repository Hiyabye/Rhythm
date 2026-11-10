use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct Room {
    pub id: &'static str,
    pub name: &'static str,
    pub label: &'static str,
    pub capacity: u32,
}

impl Room {
    pub fn url(&self) -> String {
        format!(
            "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no={}",
            self.id
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub room_ids: Vec<String>,
    pub view_room: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            room_ids: vec!["5".into()],
            view_room: "5".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SeatData {
    pub total: u32,
    pub used: u32,
    pub remaining: u32,
    pub source_time: String,
    /// None means numbers could not be validated; Some([]) is verified full.
    pub seat_numbers: Option<Vec<u32>>,
    pub source_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RoomState {
    pub room_id: String,
    pub status: String,
    pub result: Option<SeatData>,
    pub error: Option<String>,
    pub checked_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutomaticStatus {
    pub active: bool,
    pub interval_minutes: u32,
    pub next_check_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub rooms: Vec<Room>,
    pub preferences: Preferences,
    pub job_id: Option<u64>,
    pub running: bool,
    pub results: Vec<RoomState>,
    pub automatic: AutomaticStatus,
    pub favorites: std::collections::BTreeMap<String, Vec<u32>>,
}
