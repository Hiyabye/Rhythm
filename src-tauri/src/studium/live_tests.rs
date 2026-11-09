//! Explicit opt-in checks only. No captured HTML or results are persisted.
use super::{network::Session, parser, rooms};
use crate::archive::Client;
use std::sync::atomic::AtomicBool;

#[tokio::test]
#[ignore = "explicit school-server validation: robots + common + one detail, no retries"]
async fn live_default_room_uses_production_tls_transport_and_parser() {
    let client = Client::new().unwrap();
    let cancel = AtomicBool::new(false);
    let mut session = Session::new(&client, &cancel).await.expect("robots policy");
    let common = session
        .html(rooms::STATUS_URL, &cancel)
        .await
        .expect("common HTML");
    let summaries =
        parser::parse_status(&common, chrono::Utc::now().timestamp()).expect("common schema/time");
    for room in rooms::all() {
        summaries[room.id]
            .as_ref()
            .unwrap_or_else(|error| panic!("room {}: {error}", room.id));
    }
    let detail = session
        .html(&rooms::get("5").unwrap().url(), &cancel)
        .await
        .expect("detail HTML");
    let data = parser::parse_seats(&detail, "5", chrono::Utc::now().timestamp())
        .expect("detail schema/time/numbers");
    assert_eq!(data.total, 102);
    assert_eq!(
        data.seat_numbers.as_ref().unwrap().len(),
        data.remaining as usize
    );
    println!("Validated all 7 common rows and room 5 detail: total={}, used={}, remaining={}, source_time={}", data.total, data.used, data.remaining, data.source_time);
}
