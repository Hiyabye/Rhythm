//! robots.txt parsing and the policy every automatic fetch consults.
use crate::transport::{Access, Fetch, FetchError, Transport};
use std::sync::atomic::AtomicBool;
use url::Url;

/// Serves its robots.txt as unrendered ASP source (see `without_asp`).
const ASP_SOURCE_HOST: &str = "scc.sogang.ac.kr";
/// Answers /robots.txt with its HTML "page not found" page (200 text/html):
/// there is no policy, so for this host only that page means no rules (user
/// decision, #80 step 4). Any other host's HTML answer stays an error.
const HTML_404_ROBOTS_HOST: &str = "admission.sogang.ac.kr";
/// Agents, rules and crawl delay; `None` is a delay that cannot be honored,
/// which fails the policy only when the group applies to Rhythm.
type RobotGroup = (Vec<String>, Vec<(String, bool)>, Option<f64>);
#[derive(Debug)]
pub struct Robots {
    pub origin: String,
    pub rules: Vec<(String, bool)>,
    pub delay: f64,
}
impl Robots {
    pub async fn load<T: Transport>(
        transport: &T,
        url: &Url,
        cancel: &AtomicBool,
        access: Access,
    ) -> Result<Self, FetchError> {
        let robots = url
            .join("/robots.txt")
            .map_err(|_| "정보원 주소를 확인해 주세요.")?;
        match transport
            .fetch(&robots, None, 64 * 1024, cancel, access)
            .await?
        {
            Fetch::Body { bytes, mime, .. } => {
                if matches!(mime.as_deref(), Some("text/html" | "application/xhtml+xml")) {
                    if robots.host_str() == Some(HTML_404_ROBOTS_HOST) {
                        return Ok(Self::parse(url, "")?);
                    }
                    return Err("학교의 수집 정책 대신 HTML 오류 페이지를 받았습니다.".into());
                }
                let body = std::str::from_utf8(&bytes)
                    .map_err(|_| "학교의 수집 정책을 해석하지 못했습니다.")?;
                Ok(if robots.host_str() == Some(ASP_SOURCE_HOST) {
                    Self::parse(url, &without_asp(body)?)?
                } else {
                    Self::parse(url, body)?
                })
            }
            _ => Err("학교의 수집 정책을 확인하지 못했습니다.".into()),
        }
    }
    pub fn parse(url: &Url, body: &str) -> Result<Self, String> {
        // An HTTP-200 error page is not an empty allow-all robots policy.
        let start = body.trim_start_matches('\u{feff}').trim_start();
        if start.starts_with('<') || start.contains('\0') {
            return Err("학교의 수집 정책 대신 해석할 수 없는 자료를 받았습니다.".into());
        }
        let mut groups: Vec<RobotGroup> = Vec::new();
        let mut agents = Vec::new();
        let mut rules = Vec::new();
        let mut delay = Some(0.0);
        let mut directives = false;
        for line in body.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "user-agent" => {
                    if directives {
                        groups.push((
                            std::mem::take(&mut agents),
                            std::mem::take(&mut rules),
                            delay,
                        ));
                        delay = Some(0.0);
                        directives = false;
                    }
                    agents.push(value.to_ascii_lowercase());
                }
                "allow" | "disallow" if !agents.is_empty() => {
                    directives = true;
                    if !value.is_empty() {
                        rules.push((value.into(), name.trim().eq_ignore_ascii_case("allow")));
                    }
                }
                "crawl-delay" if !agents.is_empty() => {
                    directives = true;
                    // Validate before max(): floating-point max can discard NaN.
                    delay = value
                        .parse::<f64>()
                        .ok()
                        .filter(|delay| delay.is_finite() && (0.0..=60.0).contains(delay));
                }
                _ => {}
            }
        }
        groups.push((agents, rules, delay));
        let specific = groups
            .iter()
            .any(|(a, _, _)| a.iter().any(|a| a == "rhythm"));
        let mut policy = Self {
            origin: url.origin().ascii_serialization(),
            rules: Vec::new(),
            delay: 0.65,
        };
        for (agents, rules, delay) in groups {
            if agents
                .iter()
                .any(|a| a == if specific { "rhythm" } else { "*" })
            {
                let delay = delay.ok_or("학교의 수집 간격 정책은 자동 처리할 수 없습니다.")?;
                policy.rules.extend(rules);
                policy.delay = policy.delay.max(delay);
            }
        }
        if !policy.delay.is_finite() || policy.delay > 60.0 || policy.delay < 0.0 {
            return Err("학교의 수집 간격 정책은 자동 처리할 수 없습니다.".into());
        }
        Ok(policy)
    }
    pub fn permits(&self, url: &Url) -> bool {
        // Automatic cross-origin resource fetching is not needed for verified boards.
        if url.origin().ascii_serialization() != self.origin {
            return false;
        }
        let path = normalize_escapes(&url[url::Position::BeforePath..url::Position::AfterQuery]);
        let mut decision = (0, true);
        for (rule, allow) in &self.rules {
            let rule = normalize_escapes(rule);
            let pattern = rule.strip_suffix('$').unwrap_or(&rule);
            let matched = wildcard_prefix(pattern.as_bytes(), path.as_bytes(), rule.ends_with('$'));
            let weight = pattern.bytes().filter(|byte| *byte != b'*').count();
            if matched && (weight > decision.0 || (weight == decision.0 && *allow)) {
                decision = (weight, *allow);
            }
        }
        decision.1
    }
}
/// Brings a rule and a URL path to one spelling: non-ASCII bytes are
/// percent-encoded (the `url` crate already does this to paths) and the hex
/// digits of every escape are upper case, so `/한글`, `/%ED%95%9C%EA%B8%80` and
/// `/%ed%95%9c%ea%b8%80` compare equal. `*` and `$` are left alone.
fn normalize_escapes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%'
            && bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            out.push('%');
            out.push(bytes[i + 1].to_ascii_uppercase() as char);
            out.push(bytes[i + 2].to_ascii_uppercase() as char);
            i += 3;
        } else if byte.is_ascii() {
            out.push(byte as char);
            i += 1;
        } else {
            out.push_str(&format!("%{byte:02X}"));
            i += 1;
        }
    }
    out
}
/// Removes the `<% … %>` code blocks of an unrendered ASP robots.txt. That host
/// chooses between per-host rule branches in code, so dropping the code keeps
/// the rules of every branch: the policy can only become stricter. An unclosed
/// block fails instead of silently dropping the rules after it.
fn without_asp(body: &str) -> Result<String, String> {
    let mut rest = body;
    let mut rules = String::new();
    while let Some(start) = rest.find("<%") {
        rules.push_str(&rest[..start]);
        let end = rest[start..]
            .find("%>")
            .ok_or("학교의 수집 정책을 해석하지 못했습니다.")?;
        rest = &rest[start + end + 2..];
    }
    rules.push_str(rest);
    Ok(rules)
}
// Iterative glob matching with a single remembered wildcard; no recursion or
// regex backtracking. Unanchored robots rules match any suffix of the URL path.
fn wildcard_prefix(pattern: &[u8], text: &[u8], anchored: bool) -> bool {
    let (mut p, mut t) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while t < text.len() {
        if p == pattern.len() && !anchored {
            return true;
        }
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = t;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some(index) = star {
            retry += 1;
            t = retry;
            p = index + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers every request with one body and MIME type.
    struct Fixed(&'static str, &'static str);
    impl Transport for Fixed {
        async fn fetch(
            &self,
            _: &Url,
            _: Option<(Option<String>, Option<String>)>,
            _: usize,
            _: &AtomicBool,
            _: Access,
        ) -> Result<Fetch, FetchError> {
            Ok(Fetch::Body {
                bytes: self.0.as_bytes().to_vec(),
                mime: Some(self.1.into()),
                etag: None,
                last_modified: None,
            })
        }
    }

    #[tokio::test]
    async fn only_the_admission_html_robots_page_means_no_rules() {
        let page = Fixed(
            "<html><body>페이지를 찾을 수 없습니다</body></html>",
            "text/html",
        );
        let cancel = AtomicBool::new(false);
        let admission =
            Url::parse("https://admission.sogang.ac.kr/enter/html/counsel/notice.asp?s_page=1")
                .unwrap();
        let policy = Robots::load(&page, &admission, &cancel, Access::Source)
            .await
            .unwrap();
        assert!(policy.rules.is_empty() && policy.permits(&admission));
        let other = Url::parse("https://scec.sogang.ac.kr/fro_end/html/dep_05/5100.php").unwrap();
        assert!(Robots::load(&page, &other, &cancel, Access::Source)
            .await
            .is_err());
        // A real policy on the admission host still applies.
        let rules = Fixed("User-agent: *\nDisallow: /enter/\n", "text/plain");
        let policy = Robots::load(&rules, &admission, &cancel, Access::Source)
            .await
            .unwrap();
        assert!(!policy.permits(&admission));
    }

    /// `https://scc.sogang.ac.kr/robots.txt` as served (text/plain) on 2026-11-08:
    /// unrendered ASP source choosing rules per host name.
    const SCC_ROBOTS: &str = "<%\r\nResponse.ContentType=\"text/plain\";\r\nif(Request.ServerVariables(\"HTTP_HOST\") == \"nbsm.sogang.ac.kr\"){\r\n%>\r\nUser-agent: *\r\nallow:/\r\n<%\r\n}else{\r\n%>\r\n\r\nUser-agent: *\r\nDisallow:/cgsi/\r\n\r\n<%\r\n<%\r\n}\r\n%>\r\n";

    #[test]
    fn asp_source_robots_keeps_the_rules_of_every_branch() {
        let url = Url::parse(
            "https://scc.sogang.ac.kr/Download3?pathStr=MTAx&fileName=a.pdf&gubun=cmsfile",
        )
        .unwrap();
        let policy = Robots::parse(&url, &without_asp(SCC_ROBOTS).unwrap()).unwrap();
        assert!(policy.permits(&url));
        assert!(!policy.permits(&url.join("/cgsi/a").unwrap()));
        assert!(without_asp("User-agent: *\n<% if(x){ \nDisallow: /").is_err());
        assert!(
            Robots::parse(&url, SCC_ROBOTS).is_err(),
            "other hosts stay strict"
        );
    }

    #[test]
    fn rules_match_percent_encoded_paths_whatever_their_spelling() {
        let url = Url::parse("https://www.sogang.ac.kr/x").unwrap();
        let blocked = Url::parse("https://www.sogang.ac.kr/한글/x").unwrap();
        for rule in ["/한글", "/%ED%95%9C%EA%B8%80", "/%ed%95%9c%ea%b8%80"] {
            let policy =
                Robots::parse(&url, &format!("User-agent: *\nDisallow: {rule}\n")).unwrap();
            assert!(!policy.permits(&blocked), "{rule}");
            assert!(policy.permits(&url), "{rule}");
        }
        let policy = Robots::parse(&url, "User-agent: *\nDisallow: /한*/x$\n").unwrap();
        assert!(!policy.permits(&blocked));
        assert!(policy.permits(&Url::parse("https://www.sogang.ac.kr/한글/xy").unwrap()));
    }
}
