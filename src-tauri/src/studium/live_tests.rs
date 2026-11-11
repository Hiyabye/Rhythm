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

/// Explicit feature validation only: one robots request, one common page and
/// seven details through the production pinned client. No source is persisted.
#[tokio::test]
#[ignore = "explicit school-server layout validation: at most nine requests, no retries or source capture"]
async fn live_all_room_layouts_use_the_same_validated_detail_response() {
    use crate::archive::{
        network::{Access, Fetch},
        sync::Transport,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use url::Url;

    struct BoundedClient {
        client: Client,
        requests: AtomicUsize,
    }
    impl Transport for BoundedClient {
        async fn fetch(
            &self,
            url: &Url,
            validators: Option<(Option<String>, Option<String>)>,
            limit: usize,
            cancel: &AtomicBool,
            access: Access,
        ) -> Result<Fetch, String> {
            assert_eq!(access, Access::Studium);
            assert!(self.requests.fetch_add(1, Ordering::AcqRel) < 9);
            self.client
                .get(url, validators, limit, cancel, access)
                .await
        }
    }
    let transport = BoundedClient {
        client: Client::new().unwrap(),
        requests: AtomicUsize::new(0),
    };
    let cancel = AtomicBool::new(false);
    let mut session = Session::new(&transport, &cancel).await.expect("robots");
    let common = session
        .html(rooms::STATUS_URL, &cancel)
        .await
        .expect("common");
    let summaries = parser::parse_status(&common, chrono::Utc::now().timestamp()).unwrap();
    drop(common);
    for room in rooms::all() {
        assert!(summaries[room.id].is_ok());
        let detail = session.html(&room.url(), &cancel).await.expect("detail");
        let data = parser::parse_seats(&detail, room.id, chrono::Utc::now().timestamp())
            .unwrap_or_else(|error| panic!("room {}: {error}", room.id));
        let layout = data
            .layout
            .as_ref()
            .unwrap_or_else(|| panic!("room {} layout: {:?}", room.id, data.layout_error));
        assert_eq!(layout.points.len(), room.capacity as usize);
        assert!(layout.width > 0 && layout.height > 0);
        assert_eq!(
            data.seat_numbers.as_ref().unwrap().len(),
            data.remaining as usize
        );
        println!(
            "Validated room {}: points={}, total={}, remaining={}, source_time={}",
            room.id,
            layout.points.len(),
            data.total,
            data.remaining,
            data.source_time
        );
    }
    assert_eq!(transport.requests.load(Ordering::Acquire), 9);
}
