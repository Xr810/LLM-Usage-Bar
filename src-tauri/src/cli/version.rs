//! Pure version parsing and release-channel selection; no process or network I/O.
use once_cell::sync::Lazy;
use regex::Regex;

fn parse_semver(v: &str) -> Option<([u64; 3], Vec<String>)> {
    let core_and_pre = v.trim().split('+').next().unwrap_or("");
    let (core, pre) = match core_and_pre.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (core_and_pre, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let pre_segments = pre
        .map(|p| p.split('.').map(|s| s.to_string()).collect())
        .unwrap_or_default();
    Some(([major, minor, patch], pre_segments))
}

pub(super) fn compare_semver(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let (ac, ap) = parse_semver(a)?;
    let (bc, bp) = parse_semver(b)?;
    for i in 0..3 {
        match ac[i].cmp(&bc[i]) {
            Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    match (ap.is_empty(), bp.is_empty()) {
        (true, true) => return Some(Ordering::Equal),
        (true, false) => return Some(Ordering::Greater),
        (false, true) => return Some(Ordering::Less),
        (false, false) => {}
    }
    for (x, y) in ap.iter().zip(bp.iter()) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(xv), Ok(yv)) => xv.cmp(&yv),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.as_str().cmp(y.as_str()),
        };
        if ord != Ordering::Equal {
            return Some(ord);
        }
    }
    Some(ap.len().cmp(&bp.len()))
}

/// Stable users stay on latest. Only an already-ahead local installation may
/// see a newer prerelease from its explicitly allowed channel.
pub(super) fn pick_latest_version(
    dist_tags: &serde_json::Map<String, serde_json::Value>,
    prerelease_tags: &[&str],
    local_version: Option<&str>,
) -> Option<String> {
    use std::cmp::Ordering;
    let latest = dist_tags.get("latest").and_then(|v| v.as_str())?;
    let local_ahead = local_version
        .and_then(|local| compare_semver(local, latest))
        .map(|ord| ord == Ordering::Greater)
        .unwrap_or(false);
    if prerelease_tags.is_empty() || !local_ahead {
        return Some(latest.to_string());
    }
    let mut best = latest.to_string();
    for tag in prerelease_tags {
        if let Some(candidate) = dist_tags.get(*tag).and_then(|v| v.as_str()) {
            if compare_semver(candidate, &best) == Some(Ordering::Greater) {
                best = candidate.to_string();
            }
        }
    }
    Some(best)
}

static VERSION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\d+\.\d+\.\d+(-[\w.]+)?").expect("Invalid version regex"));

pub(super) fn extract_version(raw: &str) -> String {
    VERSION_RE
        .find(raw)
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_numeric_segments_and_build_metadata_preserve_order() {
        use std::cmp::Ordering;
        assert_eq!(
            compare_semver("1.2.3-beta.10", "1.2.3-beta.2"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare_semver("1.2.3+one", "1.2.3+two"),
            Some(Ordering::Equal)
        );
        assert_eq!(compare_semver("1.2.3.4", "1.2.3"), None);
        assert_eq!(extract_version("codex 0.1.2505172116"), "0.1.2505172116");
    }

    #[test]
    fn invalid_local_or_tags_never_promote_a_stable_user() {
        let tags = serde_json::json!({"latest":"2.1.154", "next":"2.1.160", "bad":false});
        let tags = tags.as_object().unwrap();
        assert_eq!(
            pick_latest_version(tags, &["next"], Some("invalid")).as_deref(),
            Some("2.1.154")
        );
        assert_eq!(
            pick_latest_version(tags, &["bad"], Some("2.1.156")).as_deref(),
            Some("2.1.154")
        );
    }
}
