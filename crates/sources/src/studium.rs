//! The exact routes of the reading-room seat server (`Access::Studium`).
use url::Url;

pub const STATUS_URL: &str = "https://libseat.sogang.ac.kr:444/SEAT/domian5.asp";

/// The fixed room IDs a detail request may name. The app's room catalog
/// (names, capacities) must list exactly these.
pub const ROOM_IDS: [&str; 7] = ["19", "5", "6", "11", "15", "16", "18"];

pub fn validate_url(url: &Url) -> Result<(), String> {
    let denied = || "허용된 열람실 조회 주소가 아닙니다.".to_string();
    if url.scheme() != "https"
        || url.host_str() != Some("libseat.sogang.ac.kr")
        || url.port_or_known_default() != Some(444)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(denied());
    }
    match url.path() {
        "/robots.txt" | "/SEAT/domian5.asp" if url.query().is_none() => Ok(()),
        "/SEAT/roomview5.asp" => {
            let query = url.query().ok_or_else(denied)?;
            let id = query.strip_prefix("room_no=").ok_or_else(denied)?;
            ROOM_IDS.contains(&id).then_some(()).ok_or_else(denied)
        }
        _ => Err(denied()),
    }
}
