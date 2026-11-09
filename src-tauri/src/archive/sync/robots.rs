//! robots.txt parsing and the policy every automatic fetch consults.
use super::*;

/// Serves its robots.txt as unrendered ASP source (see `without_asp`).
const ASP_SOURCE_HOST: &str = "scc.sogang.ac.kr";
/// Agents, rules and crawl delay; `None` is a delay that cannot be honored,
/// which fails the policy only when the group applies to Rhythm.
pub(super) type RobotGroup = (Vec<String>, Vec<(String, bool)>, Option<f64>);
#[derive(Debug)]
pub(crate) struct Robots {
    pub(crate) origin: String,
    pub(crate) rules: Vec<(String, bool)>,
    pub(crate) delay: f64,
}
impl Robots {
    pub(crate) async fn load<T: Transport>(
        transport: &T,
        url: &Url,
        cancel: &AtomicBool,
        access: network::Access,
    ) -> Result<Self, String> {
        let robots = url
            .join("/robots.txt")
            .map_err(|_| "정보원 주소를 확인해 주세요.")?;
        match transport
            .fetch(&robots, None, 64 * 1024, cancel, access)
            .await?
        {
            network::Fetch::Body { bytes, mime, .. } => {
                if matches!(mime.as_deref(), Some("text/html" | "application/xhtml+xml")) {
                    return Err("학교의 수집 정책 대신 HTML 오류 페이지를 받았습니다.".into());
                }
                let body = std::str::from_utf8(&bytes)
                    .map_err(|_| "학교의 수집 정책을 해석하지 못했습니다.")?;
                if robots.host_str() == Some(ASP_SOURCE_HOST) {
                    Self::parse(url, &without_asp(body)?)
                } else {
                    Self::parse(url, body)
                }
            }
            _ => Err("학교의 수집 정책을 확인하지 못했습니다.".into()),
        }
    }
    pub(crate) fn parse(url: &Url, body: &str) -> Result<Self, String> {
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
    pub(crate) fn permits(&self, url: &Url) -> bool {
        // Automatic cross-origin resource fetching is not needed for verified boards.
        if url.origin().ascii_serialization() != self.origin {
            return false;
        }
        let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let mut decision = (0, true);
        for (rule, allow) in &self.rules {
            let pattern = rule.strip_suffix('$').unwrap_or(rule);
            let matched = wildcard_prefix(pattern.as_bytes(), path.as_bytes(), rule.ends_with('$'));
            let weight = pattern.bytes().filter(|byte| *byte != b'*').count();
            if matched && (weight > decision.0 || (weight == decision.0 && *allow)) {
                decision = (weight, *allow);
            }
        }
        decision.1
    }
}
/// Removes the `<% … %>` code blocks of an unrendered ASP robots.txt. That host
/// chooses between per-host rule branches in code, so dropping the code keeps
/// the rules of every branch: the policy can only become stricter. An unclosed
/// block fails instead of silently dropping the rules after it.
pub(super) fn without_asp(body: &str) -> Result<String, String> {
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
pub(super) fn wildcard_prefix(pattern: &[u8], text: &[u8], anchored: bool) -> bool {
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
