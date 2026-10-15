//! A subscription's plan usage as meters: the replies of Claude's
//! `/api/oauth/usage` and Codex's `/wham/usage` read into windows
//! ("Session", "Week", "Month") and the live plan label. Shared by every
//! reader: the engine probing a CLI login, and Cloud's logins, whose usage
//! only the vault can fetch (the desktop and the phone read its reply). Pure.

use chrono::{DateTime, Utc};

use crate::{AgentUsageWindow, HarnessId};

/// `harness`'s usage reply as windows and the live plan label; `None` when
/// it carries no window.
pub fn from_reply(
    harness: HarnessId,
    body: &serde_json::Value,
) -> Option<(Vec<AgentUsageWindow>, Option<String>)> {
    match harness {
        HarnessId::ClaudeCode => claude(body)
            .filter(|windows| !windows.is_empty())
            .map(|windows| (windows, None)),
        HarnessId::Codex => codex(body),
        _ => None,
    }
}

/// Claude's `/api/oauth/usage`: the 5-hour session, weekly and per-model
/// weekly buckets, each a 0-100 utilization with a reset time. A bucket
/// reported under two names keeps its fuller reading. `None` when the reply
/// names no bucket; a bucket known only by its reset contributes no window.
pub fn claude(body: &serde_json::Value) -> Option<Vec<AgentUsageWindow>> {
    let buckets = claude_buckets(body);
    let mut windows: Vec<AgentUsageWindow> = Vec::new();
    for (label, used, resets_at) in &buckets {
        let Some(used_fraction) = used else { continue };
        let window = AgentUsageWindow {
            label: label.clone(),
            used_fraction: *used_fraction,
            resets_at: *resets_at,
        };
        if let Some(previous) = windows.iter_mut().find(|w| w.label == *label) {
            if window.used_fraction > previous.used_fraction {
                *previous = window;
            }
        } else {
            windows.push(window);
        }
    }
    (!buckets.is_empty()).then_some(windows)
}

/// One Claude usage bucket: label, used fraction, reset time.
pub type ClaudeBucket = (String, Option<f32>, Option<DateTime<Utc>>);

/// Claude's usage buckets, keeping a missing utilization apart from a known
/// reset: full probes and rate-limit headers can both know when a window
/// resets without measuring it. A `rejected` bucket reads as full.
pub fn claude_buckets(body: &serde_json::Value) -> Vec<ClaudeBucket> {
    let mut buckets = Vec::new();
    for (key, label) in [
        ("five_hour", "Session"),
        ("seven_day", "Week"),
        ("seven_day_opus", "Week (opus)"),
        ("seven_day_sonnet", "Week (sonnet)"),
        ("seven_day_fable", "Week (fable)"),
        ("seven_day_haiku", "Week (haiku)"),
    ] {
        if let Some(w) = body.get(key) {
            let utilization = ["used_percentage", "utilization", "usedPercentage"]
                .into_iter()
                .find_map(|key| w.get(key).and_then(quota_number));
            let used = if w["status"] == "rejected" {
                Some(1.0)
            } else {
                utilization.map(|u| (u / 100.0).clamp(0.0, 1.0) as f32)
            };
            let reset = parse_quota_reset(
                w.get("resets_at")
                    .or_else(|| w.get("reset_at"))
                    .or_else(|| w.get("resetAt")),
            );
            if used.is_some() || reset.is_some() {
                buckets.push((label.to_string(), used, reset));
            }
        }
    }
    for limit in body["limits"].as_array().into_iter().flatten() {
        if limit["kind"] != "weekly_scoped" && limit["group"] != "weekly" {
            continue;
        }
        let Some(model) = limit
            .pointer("/scope/model/display_name")
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let family = crate::model_family(model);
        if family == "other" {
            continue;
        }
        let used = limit.get("percent").and_then(quota_number);
        let used = if limit["status"] == "rejected" {
            Some(1.0)
        } else {
            used.map(|u| (u / 100.0).clamp(0.0, 1.0) as f32)
        };
        let reset = parse_quota_reset(limit.get("resets_at"));
        if used.is_some() || reset.is_some() {
            buckets.push((format!("Week ({family})"), used, reset));
        }
    }
    buckets
}

/// A finite number sent as a JSON number or a numeric string.
pub fn quota_number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| v.is_finite())
}

/// A reset time as Unix seconds, Unix millis, or RFC 3339.
fn parse_quota_reset(value: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    let value = value?;
    if let Some(number) = quota_number(value) {
        let ms = if number < 1_000_000_000_000.0 {
            number * 1000.0
        } else {
            number
        };
        DateTime::from_timestamp_millis(ms as i64)
    } else {
        parse_when(Some(value))
    }
}

/// Codex `/wham/usage`: primary/secondary windows + the live plan ("free"/
/// "plus"/"pro"…), which beats the login-time JWT claim.
pub fn codex(body: &serde_json::Value) -> Option<(Vec<AgentUsageWindow>, Option<String>)> {
    codex_labeled(body, &std::collections::HashMap::new())
}

/// [`codex`], naming a window that reports no span from `labels` (keyed
/// `primary` / `secondary`, as learned from an earlier reply), else
/// "Primary" / "Secondary".
pub fn codex_labeled(
    body: &serde_json::Value,
    labels: &std::collections::HashMap<String, String>,
) -> Option<(Vec<AgentUsageWindow>, Option<String>)> {
    let rl = body.get("rate_limit")?;
    let mut windows = Vec::new();
    for (key, bucket, fallback) in [
        ("primary_window", "primary", "Primary"),
        ("secondary_window", "secondary", "Secondary"),
    ] {
        if let Some(w) = rl.get(key)
            && let Some(used) = w.get("used_percent").and_then(|v| v.as_f64())
        {
            let span = w
                .get("limit_window_seconds")
                .and_then(|v| v.as_i64())
                .filter(|span| *span > 0);
            windows.push(AgentUsageWindow {
                label: span
                    .map(codex_window_label)
                    .map(str::to_owned)
                    .or_else(|| labels.get(bucket).cloned())
                    .unwrap_or_else(|| fallback.into()),
                used_fraction: (used / 100.0) as f32,
                resets_at: parse_when(w.get("reset_at")),
            });
        }
    }
    if windows.is_empty() {
        return None;
    }
    let plan = body
        .get("plan_type")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    Some((windows, codex_plan(plan)))
}

/// "ChatGPT Plus" from Codex's `plan_type`.
pub fn codex_plan(plan: Option<&str>) -> Option<String> {
    let plan = plan?;
    let mut chars = plan.chars();
    let first = chars.next()?;
    Some(format!(
        "ChatGPT {}{}",
        first.to_uppercase(),
        chars.as_str()
    ))
}

/// Meter label for a Codex rate-limit window from its `limit_window_seconds`:
/// the free tier's window is a 30-day month (2_592_000s), Plus runs a 5-hour
/// primary (~18_000s) with a weekly secondary (604_800s). A bare "> 1 day =
/// week" rule mislabeled the monthly window "Week"; thresholds in seconds
/// leave the middle gaps to the nearest label rather than guessing a plan.
pub fn codex_window_label(span_seconds: i64) -> &'static str {
    const DAY: i64 = 86_400;
    if span_seconds >= 28 * DAY {
        "Month"
    } else if span_seconds >= 5 * DAY {
        "Week"
    } else {
        "Session"
    }
}

/// A reset time as Unix seconds or RFC 3339.
pub fn parse_when(value: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    match value? {
        serde_json::Value::Number(n) => DateTime::<Utc>::from_timestamp(n.as_i64()?, 0),
        serde_json::Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}
