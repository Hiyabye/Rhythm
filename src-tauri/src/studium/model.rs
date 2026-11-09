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
