//! Fetching the body of a feed notice when it is opened (#177). The feed
//! carries only an excerpt; the reader shows it at once and this fetches the
//! article from the school under the same rules as the crawler (approved
//! routes, robots.txt, the client's per-host pacing), then fills the notice's
//! revision in place. Main-board notices whose body is the school's PDF
//! viewer download that PDF and use its text, as the crawler does.
use super::*;
use std::time::{Duration, Instant};

/// How long a host's robots.txt is reused between opened notices.
const ROBOTS_TTL: Duration = Duration::from_secs(60 * 60);

/// Body fetches of opened notices. They run beside each other and beside a
/// sync job; maintenance, exit and turning the notices extension off cancel
/// them.
#[derive(Default)]
pub struct Bodies {
    active: Mutex<Vec<Arc<AtomicBool>>>,
    robots: Mutex<HashMap<String, (Instant, Arc<Robots>)>>,
    downloading: Mutex<HashSet<i64>>,
}

struct Active<'a>(&'a Bodies, Arc<AtomicBool>);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.active.lock() {
            active.retain(|cancel| !Arc::ptr_eq(cancel, &self.1));
        }
    }
}

/// The notice whose files one call is downloading.
struct Downloading<'a>(&'a Bodies, i64);
impl<'a> Downloading<'a> {
    fn claim(bodies: &'a Bodies, id: i64) -> Result<Option<Self>, String> {
        let mut downloading = bodies
            .downloading
            .lock()
            .map_err(|_| "첨부 자료를 내려받지 못했습니다.")?;
        Ok(downloading.insert(id).then_some(Self(bodies, id)))
    }
}
impl Drop for Downloading<'_> {
    fn drop(&mut self) {
        if let Ok(mut downloading) = self.0.downloading.lock() {
            downloading.remove(&self.1);
        }
    }
}

impl Bodies {
    pub fn cancel(&self) {
        if let Ok(active) = self.active.lock() {
            for cancel in active.iter() {
                cancel.store(true, Ordering::Release);
            }
        }
    }

    pub async fn wait(&self) {
        while self.active.lock().map(|a| !a.is_empty()).unwrap_or(false) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    /// Fetches and stores the body of notice `id` unless it is already here
    /// or the source deleted it. Returns whether the school's text differs
    /// from the revision the feed described. A failure keeps the excerpt.
    pub async fn fetch<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        id: i64,
    ) -> Result<bool, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.active
            .lock()
            .map_err(|_| "공지 본문을 가져오지 못했습니다.")?
            .push(cancel.clone());
        let _active = Active(self, cancel.clone());
        let Some((definition, entry)) = with_store(store, |s| {
            let Some((source, entry)) = s.pending_body(id)? else {
                return Ok(None);
            };
            enabled(s, &source)?;
            Ok(Some((s.source(&source)?.definition, entry)))
        })?
        else {
            return Ok(false);
        };
        let url = parser::detail_url(&definition, &entry)?;
        let policy = self.robots(transport, &url, &cancel).await?;
        let network::Fetch::Body {
            bytes,
            etag,
            last_modified,
            mime,
        } = fetch(
            transport,
            &url,
            None,
            ARTICLE_LIMIT,
            &cancel,
            &policy,
            network::Access::Source,
        )
        .await?
        else {
            return Err("학교 서버가 공지 본문을 보내지 않았습니다.".into());
        };
        check_document_mime(&definition, mime.as_deref())?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "공지 본문 문자 인코딩을 지원하지 않습니다.")?;
        let mut article = parser::article(&definition, &entry, text)?;
        article.etag = etag;
        article.last_modified = last_modified;
        let viewer: Vec<String> = parser::viewer_pdf_resources(&article)
            .into_iter()
            .map(|r| r.url.clone())
            .collect();
        if !viewer.is_empty() {
            self.viewer_pdfs(store, root, transport, &cancel, id, &article, &viewer)
                .await?;
            local_pdf_text(store, root, id, &mut article).await?;
        }
        let filled = with_store(store, |s| {
            ensure_active(&cancel)?;
            enabled(s, &definition.id)?;
            s.fill_feed_body(id, &article, viewer.is_empty())
        })?;
        // The rows are gone, so a file left behind is only unused disk space.
        for path in filled.orphaned {
            let _ = tokio::fs::remove_file(root.join(path)).await;
        }
        Ok(filled.revised)
    }

    /// Downloads the images and attachments of opened notice `id` that were
    /// not tried yet, under the crawler's rules: the board's robots.txt,
    /// another image host's own, the per-file caps and one job's budget.
    /// Each failure is recorded on its file, which then waits for an explicit
    /// retry. Cybercampus files stay with its session sync. A second call for
    /// the same notice waits for the first.
    pub async fn resources<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        id: i64,
    ) -> Result<(), String> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.active
            .lock()
            .map_err(|_| "첨부 자료를 내려받지 못했습니다.")?
            .push(cancel.clone());
        let _active = Active(self, cancel.clone());
        let _downloading = loop {
            if let Some(guard) = Downloading::claim(self, id)? {
                break guard;
            }
            active(&cancel)?;
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let Some((definition, entry, resources)) =
            with_store(store, |s| {
                let Some((source, entry, resources)) = s.pending_downloads(id)? else {
                    return Ok(None);
                };
                enabled(s, &source)?;
                let definition = s.source(&source)?.definition;
                Ok((definition.family != cybercampus::FAMILY)
                    .then_some((definition, entry, resources)))
            })?
        else {
            return Ok(());
        };
        let url = parser::detail_url(&definition, &entry)?;
        let policy = self.robots(transport, &url, &cancel).await?;
        let image_robots = RobotsCache::default();
        let total = AtomicU64::new(0);
        for resource in resources {
            with_store(store, |s| enabled(s, &definition.id))?;
            download_resource(
                store,
                root,
                transport,
                &cancel,
                &policy,
                &image_robots,
                &resource,
                &total,
            )
            .await?;
        }
        Ok(())
    }

    /// Downloads the notice's viewer PDFs it does not hold yet. A failed
    /// download fails the fetch, so opening the notice again retries it.
    #[allow(clippy::too_many_arguments)]
    async fn viewer_pdfs<T: Transport>(
        &self,
        store: &SharedStore,
        root: &Path,
        transport: &T,
        cancel: &AtomicBool,
        id: i64,
        article: &ArticleInput,
        viewer: &[String],
    ) -> Result<(), String> {
        let missing = with_store(store, |s| {
            ensure_active(cancel)?;
            s.attach_resources(id, &article.resources)?;
            Ok(s.pending_resources(id)?
                .into_iter()
                .filter(|r| viewer.contains(&r.url))
                .collect::<Vec<_>>())
        })?;
        let total = AtomicU64::new(0);
        for resource in missing {
            let url =
                Url::parse(&resource.url).map_err(|_| "첨부 자료 주소가 올바르지 않습니다.")?;
            let policy = self.robots(transport, &url, cancel).await?;
            download_resource(
                store,
                root,
                transport,
                cancel,
                &policy,
                &RobotsCache::default(),
                &resource,
                &total,
            )
            .await?
            .into_result()?;
        }
        Ok(())
    }

    /// The robots.txt of `url`'s host, loaded at most hourly. Failures are
    /// not kept, so the next open asks again.
    async fn robots<T: Transport>(
        &self,
        transport: &T,
        url: &Url,
        cancel: &AtomicBool,
    ) -> Result<Arc<Robots>, String> {
        let origin = url.origin().ascii_serialization();
        if let Some((loaded, policy)) = self
            .robots
            .lock()
            .ok()
            .and_then(|c| c.get(&origin).cloned())
        {
            if loaded.elapsed() < ROBOTS_TTL {
                return Ok(policy);
            }
        }
        let policy = Arc::new(Robots::load(transport, url, cancel, network::Access::Source).await?);
        if let Ok(mut cache) = self.robots.lock() {
            cache.insert(origin, (Instant::now(), policy.clone()));
        }
        Ok(policy)
    }
}

fn enabled(store: &Store, source: &str) -> Result<(), String> {
    if store.source_extension_enabled(source)? {
        Ok(())
    } else {
        Err("이 공지의 확장 기능을 먼저 켜 주세요.".into())
    }
}
