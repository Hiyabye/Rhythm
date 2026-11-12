use super::{cancelled, parser, rooms};
use crate::archive::{
    network::{Access, Fetch, FetchError},
    sync::{robots::Robots, Transport},
};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use url::Url;

pub(crate) const RESPONSE_LIMIT: usize = 1_000_000;

pub(crate) fn validate_url(url: &Url) -> Result<(), String> {
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
            rooms::get(id).map(|_| ()).map_err(|_| denied())
        }
        _ => Err(denied()),
    }
}

/// One manual cycle shares its policy. Transport keeps cross-cycle pacing.
pub(crate) struct Session<'a, T> {
    transport: &'a T,
    policy: Robots,
    last: Instant,
}

impl<'a, T: Transport> Session<'a, T> {
    pub(crate) async fn new(transport: &'a T, cancel: &AtomicBool) -> Result<Self, FetchError> {
        let url = Url::parse(rooms::STATUS_URL).map_err(|_| "열람실 주소 오류")?;
        let policy = Robots::load(transport, &url, cancel, Access::Studium).await?;
        Ok(Self {
            transport,
            policy,
            last: Instant::now(),
        })
    }

    pub(crate) async fn html(
        &mut self,
        value: &str,
        cancel: &AtomicBool,
    ) -> Result<String, FetchError> {
        let url = Url::parse(value).map_err(|_| "열람실 주소 오류")?;
        validate_url(&url)?;
        if !self.policy.permits(&url) {
            return Err("학교의 수집 정책에서 좌석 조회를 허용하지 않습니다.".into());
        }
        let delay = Duration::from_secs_f64(self.policy.delay).saturating_sub(self.last.elapsed());
        tokio::select! {
            _ = tokio::time::sleep(delay) => {},
            _ = cancelled(cancel) => return Err("좌석 확인을 취소했습니다.".into()),
        }
        self.last = Instant::now();
        match self
            .transport
            .fetch(&url, None, RESPONSE_LIMIT, cancel, Access::Studium)
            .await?
        {
            Fetch::Body { bytes, mime, .. } => {
                if bytes.len() > RESPONSE_LIMIT {
                    return Err("좌석 응답 크기 제한을 초과했습니다.".into());
                }
                if !matches!(
                    mime.as_deref(),
                    None | Some("text/html" | "application/xhtml+xml")
                ) {
                    return Err("좌석 서버가 HTML이 아닌 응답을 보냈습니다.".into());
                }
                Ok(parser::decode_page(&bytes)?)
            }
            _ => Err("좌석 서버의 응답을 확인하지 못했습니다.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seat_access_does_not_expand_existing_archive_access() {
        for value in [
            rooms::STATUS_URL,
            "https://libseat.sogang.ac.kr:444/robots.txt",
            "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=5",
        ] {
            let url = Url::parse(value).unwrap();
            assert!(validate_url(&url).is_ok());
            assert!(crate::archive::network::validate(&url, Access::Source).is_err());
            assert!(crate::archive::network::validate(&url, Access::EmbeddedImage).is_err());
        }
        for value in [
            "http://libseat.sogang.ac.kr:444/SEAT/domian5.asp",
            "https://libseat.sogang.ac.kr/SEAT/domian5.asp",
            "https://libseat.sogang.ac.kr:445/SEAT/domian5.asp",
            "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=5&room_no=6",
            "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=%35",
            "https://libseat.sogang.ac.kr:444/SEAT/roomview5.asp?room_no=999",
            "https://user@libseat.sogang.ac.kr:444/SEAT/domian5.asp",
            "https://libseat.sogang.ac.kr:444/SEAT/domian5.asp#x",
            "https://libseat.sogang.ac.kr:444/robots.txt?x=1",
            "https://www.sogang.ac.kr/",
        ] {
            assert!(
                validate_url(&Url::parse(value).unwrap()).is_err(),
                "{value}"
            );
        }
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod session_tests;
