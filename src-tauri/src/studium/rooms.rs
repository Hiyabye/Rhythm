use super::model::Room;

pub const STATUS_URL: &str = "https://libseat.sogang.ac.kr:444/SEAT/domian5.asp";

// Library room identities/capacities observed in the Studium handoff. A changed
// school capacity must fail validation rather than silently renumbering seats.
pub const ROOMS: [Room; 7] = [
    Room {
        id: "19",
        name: "111 로욜라 인벤툼Ⅰ",
        label: "로욜라 인벤툼 I",
        capacity: 113,
    },
    Room {
        id: "5",
        name: "133 로욜라 인벤툼 II",
        label: "로욜라 인벤툼 II",
        capacity: 102,
    },
    Room {
        id: "6",
        name: "K관 열람실",
        label: "K관 열람실",
        capacity: 311,
    },
    Room {
        id: "11",
        name: "PA관 열람실",
        label: "PA관 열람실",
        capacity: 101,
    },
    Room {
        id: "15",
        name: "J관 일반열람실",
        label: "J관 일반열람실",
        capacity: 207,
    },
    Room {
        id: "16",
        name: "J관 노트북전용실",
        label: "J관 노트북전용실",
        capacity: 92,
    },
    Room {
        id: "18",
        name: "MA관 열람실",
        label: "MA관 열람실",
        capacity: 174,
    },
];

pub fn get(id: &str) -> Result<&'static Room, String> {
    ROOMS
        .iter()
        .find(|room| room.id == id)
        .ok_or_else(|| "지원하지 않는 열람실입니다.".into())
}

pub fn all() -> Vec<Room> {
    ROOMS.to_vec()
}
