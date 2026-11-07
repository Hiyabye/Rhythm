//! robots.txt parsing and the policy every automatic fetch consults.
use super::*;

pub(super) type RobotGroup = (Vec<String>, Vec<(String, bool)>, f64);
#[derive(Debug)]
pub(super) struct Robots {
    pub(super) origin: String,
    pub(super) rules: Vec<(String, bool)>,
    pub(super) delay: f64,
}
impl Robots {
    pub(super) async fn load<T: Transport>(
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
                Self::parse(
                    url,
                    std::str::from_utf8(&bytes)
                        .map_err(|_| "학교의 수집 정책을 해석하지 못했습니다.")?,
                )
            }
            _ => Err("학교의 수집 정책을 확인하지 못했습니다.".into()),
        }
    }
    pub(super) fn parse(url: &Url, body: &str) -> Result<Self, String> {
        // An HTTP-200 error page is not an empty allow-all robots policy.
        let start = body.trim_start_matches('\u{feff}').trim_start();
        if start.starts_with('<') || start.contains('\0') {
            return Err("학교의 수집 정책 대신 해석할 수 없는 자료를 받았습니다.".into());
        }
        let mut groups: Vec<RobotGroup> = Vec::new();
        let mut agents = Vec::new();
        let mut rules = Vec::new();
        let mut delay = 0.0;
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
                        delay = 0.0;
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
                    delay = value
                        .parse::<f64>()
                        .map_err(|_| "학교의 요청 간격 정책을 해석하지 못했습니다.")?;
                    // Validate before max(): floating-point max can discard NaN.
                    if !delay.is_finite() || !(0.0..=60.0).contains(&delay) {
                        return Err("학교의 수집 간격 정책은 자동 처리할 수 없습니다.".into());
                    }
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
                policy.rules.extend(rules);
                policy.delay = policy.delay.max(delay);
            }
        }
        if !policy.delay.is_finite() || policy.delay > 60.0 || policy.delay < 0.0 {
            return Err("학교의 수집 간격 정책은 자동 처리할 수 없습니다.".into());
        }
        Ok(policy)
    }
    pub(super) fn permits(&self, url: &Url) -> bool {
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
