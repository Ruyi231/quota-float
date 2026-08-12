use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, RETRY_AFTER};
use serde::Serialize;
use serde_json::Value;
use std::{
    cmp::Reverse,
    env,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

const MAX_FILES_TO_SCAN: usize = 32;
const MAX_TAIL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_AUTH_BYTES: u64 = 256 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub label: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub resets_at: Option<i64>,
    pub window_minutes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub status: String,
    pub source: String,
    pub plan_type: Option<String>,
    pub primary: Option<UsageWindow>,
    pub secondary: Option<UsageWindow>,
    pub reset_credits: Option<ResetCreditSnapshot>,
    pub observed_at: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCreditSnapshot {
    pub status: String,
    pub count: Option<u64>,
    pub expires_at: Vec<String>,
    pub message: Option<String>,
}

struct Auth {
    access_token: String,
    account_id: Option<String>,
}

impl QuotaSnapshot {
    pub(crate) fn missing(message: impl Into<String>) -> Self {
        Self {
            status: "missing".into(),
            source: "local".into(),
            plan_type: None,
            primary: None,
            secondary: None,
            reset_credits: None,
            observed_at: None,
            message: Some(message.into()),
        }
    }
}

pub fn read_latest_snapshot() -> QuotaSnapshot {
    let Some(session_root) = codex_session_root() else {
        return QuotaSnapshot::missing("无法定位用户目录。");
    };
    if !session_root.is_dir() {
        return QuotaSnapshot::missing("尚未找到 Codex 会话目录，请先运行一次 Codex 任务。");
    }

    let mut files = Vec::new();
    collect_rollout_files(&session_root, &mut files);
    files.sort_by_key(|(modified, _)| Reverse(*modified));

    for (_, path) in files.into_iter().take(MAX_FILES_TO_SCAN) {
        if let Some(snapshot) = read_snapshot_from_file(&path) {
            return snapshot;
        }
    }

    QuotaSnapshot::missing("会话日志中尚无额度事件，请在 Codex 中运行一次任务后刷新。")
}

fn codex_session_root() -> Option<PathBuf> {
    if let Some(root) = env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(root).join("sessions"));
    }
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|root| root.join(".codex").join("sessions"))
}

fn auth_path() -> Option<PathBuf> {
    if let Some(root) = env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(root).join("auth.json"));
    }
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|root| root.join(".codex").join("auth.json"))
}

fn pick_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| value.get(*key)?.as_str())
}

fn account_id_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    pick_string(
        &value,
        &[
            "https://api.openai.com/auth.chatgpt_account_id",
            "chatgpt_account_id",
        ],
    )
    .map(str::to_owned)
}

fn load_auth() -> Result<Auth, String> {
    let path = auth_path().ok_or_else(|| "无法定位 Codex 登录信息。".to_string())?;
    let metadata = fs::metadata(&path).map_err(|_| "请先在 Codex Desktop 中登录。".to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_AUTH_BYTES {
        return Err("Codex 登录信息不可用。".into());
    }
    let raw = fs::read_to_string(path).map_err(|_| "无法读取 Codex 登录信息。".to_string())?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|_| "Codex 登录格式已经变化。".to_string())?;
    let tokens = value.get("tokens").unwrap_or(&value);
    let access_token = pick_string(tokens, &["access_token", "accessToken"])
        .ok_or_else(|| "Codex 登录已过期，请重新登录。".to_string())?
        .to_owned();
    let account_id = pick_string(tokens, &["account_id", "accountId"])
        .map(str::to_owned)
        .or_else(|| account_id_from_jwt(&access_token));
    Ok(Auth {
        access_token,
        account_id,
    })
}

fn auth_headers(auth: &Auth) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    let mut bearer = HeaderValue::from_str(&format!("Bearer {}", auth.access_token))
        .map_err(|_| "Codex 登录信息无效。".to_string())?;
    bearer.set_sensitive(true);
    headers.insert(AUTHORIZATION, bearer);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
    headers.insert("OAI-Product-Sku", HeaderValue::from_static("CODEX"));
    if let Some(account_id) = &auth.account_id {
        let mut value =
            HeaderValue::from_str(account_id).map_err(|_| "账户标识无效。".to_string())?;
        value.set_sensitive(true);
        headers.insert("ChatGPT-Account-Id", value);
    }
    Ok(headers)
}

fn rate_limited_message(headers: &HeaderMap, subject: &str) -> String {
    let retry_after = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    match retry_after {
        Some(seconds) => format!("{subject}，请 {seconds} 秒后重试。"),
        None => format!("{subject}，请至少等待 5 分钟后重试。"),
    }
}

fn online_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("CodexLocalQuotaWidget/1.3.3")
        .build()
        .map_err(|_| "无法初始化在线查询。".to_string())
}

async fn limited_json(mut response: reqwest::Response) -> Result<Value, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("额度服务响应异常过大。".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "无法读取额度服务响应。".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("额度服务响应异常过大。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "额度服务响应格式已经变化。".to_string())
}

fn direct_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
    })
}

fn timestamp_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        if let Some(text) = value.as_str() {
            return Some(text.to_owned());
        }
        value.as_i64().map(|seconds| seconds.to_string())
    })
}

fn timestamp_epoch(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
            .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
            .or_else(|| {
                value
                    .as_str()
                    .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                    .map(|time| time.timestamp())
            })
    })
}

fn number_with_key(value: &Value, keys: &[&str]) -> Option<(usize, f64)> {
    keys.iter()
        .enumerate()
        .find_map(|(index, key)| value.get(*key)?.as_f64().map(|number| (index, number)))
}

fn scale_ratio_field(key: &str, value: f64) -> bool {
    matches!(
        key,
        "remaining_ratio" | "remainingRatio" | "used_ratio" | "usedRatio" | "utilization"
    ) || (!key.contains("percent") && !key.contains("pct") && value <= 1.0)
}

fn parse_online_window(value: Option<&Value>) -> Option<UsageWindow> {
    let value = value?;
    let remaining_keys = [
        "remaining_percent",
        "remainingPercent",
        "remaining_pct",
        "remainingPct",
        "remaining_ratio",
        "remainingRatio",
        "remaining",
    ];
    let used_keys = [
        "used_percent",
        "usedPercent",
        "used_pct",
        "usedPct",
        "used_ratio",
        "usedRatio",
        "utilization",
        "used",
    ];
    let remaining_percent =
        if let Some((index, remaining)) = number_with_key(value, &remaining_keys) {
            if scale_ratio_field(remaining_keys[index], remaining) {
                remaining * 100.0
            } else {
                remaining
            }
        } else {
            let (index, used) = number_with_key(value, &used_keys)?;
            let used_percent = if scale_ratio_field(used_keys[index], used) {
                used * 100.0
            } else {
                used
            };
            100.0 - used_percent
        }
        .clamp(0.0, 100.0);

    let window_minutes = direct_u64(
        value,
        &[
            "window_minutes",
            "windowMinutes",
            "duration_minutes",
            "durationMinutes",
        ],
    )
    .and_then(|minutes| i64::try_from(minutes).ok())
    .or_else(|| {
        direct_u64(
            value,
            &[
                "limit_window_seconds",
                "limitWindowSeconds",
                "window_seconds",
                "windowSeconds",
                "duration_seconds",
                "durationSeconds",
                "period_seconds",
                "periodSeconds",
            ],
        )
        .and_then(|seconds| i64::try_from(seconds / 60).ok())
    });

    Some(UsageWindow {
        label: window_label(window_minutes),
        used_percent: (100.0 - remaining_percent).clamp(0.0, 100.0),
        remaining_percent,
        resets_at: timestamp_epoch(
            value,
            &[
                "reset_at",
                "resetAt",
                "resets_at",
                "resetsAt",
                "reset_time",
                "resetTime",
            ],
        ),
        window_minutes,
    })
}

fn find_online_window<'a>(
    rate_limit: &'a Value,
    names: &[&str],
    expected_seconds: u64,
) -> Option<&'a Value> {
    for name in names {
        if let Some(value) = rate_limit.get(*name) {
            let Some(window) = parse_online_window(Some(value)) else {
                continue;
            };
            let actual_seconds = window
                .window_minutes
                .and_then(|minutes| u64::try_from(minutes).ok())
                .unwrap_or(0)
                .saturating_mul(60);
            if actual_seconds == 0 || actual_seconds.abs_diff(expected_seconds) <= 60 {
                return Some(value);
            }
        }
    }
    for key in [
        "windows",
        "limit_windows",
        "limitWindows",
        "limits",
        "buckets",
    ] {
        let Some(items) = rate_limit.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(window) = parse_online_window(Some(item)) else {
                continue;
            };
            let actual_seconds = window
                .window_minutes
                .and_then(|minutes| u64::try_from(minutes).ok())
                .unwrap_or(0)
                .saturating_mul(60);
            let matches_duration =
                actual_seconds > 0 && actual_seconds.abs_diff(expected_seconds) <= 60;
            let matches_name = pick_string(item, &["name", "type", "id", "window", "label"])
                .map(|text| {
                    let lower = text.to_ascii_lowercase();
                    names.iter().any(|name| {
                        let candidate = name.to_ascii_lowercase();
                        lower == candidate || lower.contains(&candidate)
                    })
                })
                .unwrap_or(false);
            if matches_duration || matches_name {
                return Some(item);
            }
        }
    }
    None
}

fn parse_usage_response(value: &Value) -> Result<QuotaSnapshot, String> {
    let rate_limit = value
        .get("rate_limit")
        .or_else(|| value.get("rateLimit"))
        .unwrap_or(value);
    let short_window = parse_online_window(find_online_window(
        rate_limit,
        &[
            "primary_window",
            "primaryWindow",
            "short_window",
            "shortWindow",
            "five_hour_window",
            "fiveHourWindow",
            "5h",
            "primary",
        ],
        18_000,
    ));
    let mut weekly_window = parse_online_window(find_online_window(
        rate_limit,
        &[
            "secondary_window",
            "secondaryWindow",
            "weekly_window",
            "weeklyWindow",
            "week_window",
            "weekWindow",
            "weekly",
            "secondary",
            "primary_window",
            "primaryWindow",
            "primary",
        ],
        604_800,
    ));
    if short_window == weekly_window {
        weekly_window = None;
    }
    let (primary, secondary) = match (short_window, weekly_window) {
        (Some(short), weekly) => (Some(short), weekly),
        (None, Some(weekly)) => (Some(weekly), None),
        (None, None) => {
            return Err("在线额度响应中没有可识别的额度窗口。".into());
        }
    };
    Ok(QuotaSnapshot {
        status: "ok".into(),
        source: "online".into(),
        plan_type: pick_string(value, &["plan_type", "planType"])
            .or_else(|| pick_string(rate_limit, &["plan_type", "planType"]))
            .map(str::to_owned),
        primary,
        secondary,
        reset_credits: value
            .get("rate_limit_reset_credits")
            .or_else(|| value.get("rateLimitResetCredits"))
            .or_else(|| rate_limit.get("rate_limit_reset_credits"))
            .or_else(|| rate_limit.get("rateLimitResetCredits"))
            .and_then(parse_meaningful_reset_credit_response),
        observed_at: Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        message: None,
    })
}

fn collect_reset_credit_expirations(value: &Value) -> Vec<String> {
    fn visit(value: &Value, output: &mut Vec<String>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    visit(item, output);
                }
            }
            Value::Object(map) => {
                if let Some(time) = timestamp_text(
                    value,
                    &[
                        "expires_at",
                        "expiresAt",
                        "expiration_time",
                        "expirationTime",
                        "expires",
                    ],
                ) {
                    output.push(time);
                }
                for key in [
                    "credits",
                    "reset_credits",
                    "resetCredits",
                    "available",
                    "items",
                    "grants",
                ] {
                    if let Some(child) = map.get(key) {
                        visit(child, output);
                    }
                }
            }
            _ => {}
        }
    }

    let mut expirations = Vec::new();
    visit(value, &mut expirations);
    expirations.sort();
    expirations.dedup();
    expirations
}

fn parse_reset_credit_response(value: &Value) -> ResetCreditSnapshot {
    let expirations = collect_reset_credit_expirations(value);
    let count = direct_u64(
        value,
        &[
            "available_count",
            "availableCount",
            "remaining",
            "count",
            "quantity",
        ],
    )
    .or_else(|| (!expirations.is_empty()).then_some(expirations.len() as u64));
    ResetCreditSnapshot {
        status: "ok".into(),
        count,
        expires_at: expirations,
        message: None,
    }
}

fn parse_meaningful_reset_credit_response(value: &Value) -> Option<ResetCreditSnapshot> {
    let snapshot = parse_reset_credit_response(value);
    (snapshot.count.is_some() || !snapshot.expires_at.is_empty()).then_some(snapshot)
}

pub async fn read_quota_online() -> Result<QuotaSnapshot, String> {
    let auth = load_auth()?;
    let headers = auth_headers(&auth)?;
    let response = online_client()?
        .get(USAGE_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|_| "在线额度刷新失败，请检查网络。".to_string())?;
    if !response.status().is_success() {
        let rate_limited = rate_limited_message(response.headers(), "额度查询过于频繁");
        return Err(match response.status().as_u16() {
            401 | 403 => "Codex 登录已过期或没有额度查询权限。".into(),
            429 => rate_limited,
            _ => format!("在线额度服务暂不可用（{}）。", response.status().as_u16()),
        });
    }
    parse_usage_response(&limited_json(response).await?)
}

pub async fn read_reset_credits_online() -> Result<ResetCreditSnapshot, String> {
    let auth = load_auth()?;
    let headers = auth_headers(&auth)?;
    let response = online_client()?
        .get(RESET_CREDITS_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|_| "重置卡查询失败，请检查网络。".to_string())?;
    if !response.status().is_success() {
        let rate_limited = rate_limited_message(response.headers(), "重置卡查询过于频繁");
        return Err(match response.status().as_u16() {
            401 | 403 => "Codex 登录已过期或没有查询权限。".into(),
            429 => rate_limited,
            _ => format!("重置卡服务暂不可用（{}）。", response.status().as_u16()),
        });
    }
    let value = limited_json(response).await?;
    Ok(parse_reset_credit_response(&value))
}

fn collect_rollout_files(directory: &Path, output: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_rollout_files(&path, output);
            continue;
        }
        let is_rollout = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
            .unwrap_or(false);
        if !is_rollout {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        output.push((modified, path));
    }
}

fn read_snapshot_from_file(path: &Path) -> Option<QuotaSnapshot> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(MAX_TAIL_BYTES);
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }
    let mut bytes = Vec::with_capacity((length - start).min(MAX_TAIL_BYTES) as usize);
    file.read_to_end(&mut bytes).ok()?;
    if start > 0 {
        let newline = bytes.iter().position(|byte| *byte == b'\n')?;
        bytes.drain(..=newline);
    }
    let text = String::from_utf8_lossy(&bytes);
    text.lines().rev().find_map(parse_event_line)
}

fn parse_event_line(line: &str) -> Option<QuotaSnapshot> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let rate_limits = payload.get("rate_limits")?.as_object()?;
    let primary = rate_limits.get("primary").and_then(parse_window);
    let secondary = rate_limits.get("secondary").and_then(parse_window);
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(QuotaSnapshot {
        status: "ok".into(),
        source: "local".into(),
        plan_type: rate_limits
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        primary,
        secondary,
        reset_credits: None,
        observed_at: value
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_owned),
        message: None,
    })
}

fn parse_window(value: &Value) -> Option<UsageWindow> {
    let object = value.as_object()?;
    let used_percent = object.get("used_percent")?.as_f64()?.clamp(0.0, 100.0);
    let window_minutes = object.get("window_minutes").and_then(number_as_i64);
    Some(UsageWindow {
        label: window_label(window_minutes),
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        resets_at: object.get("resets_at").and_then(number_as_i64),
        window_minutes,
    })
}

fn number_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_f64().map(|number| number.round() as i64))
}

fn window_label(minutes: Option<i64>) -> String {
    match minutes {
        Some(value) if value <= 360 => "5 小时额度".into(),
        Some(value) if value <= 1_440 => "每日额度".into(),
        Some(value) if value <= 10_080 => "7 天额度".into(),
        Some(value) if value > 0 && value % 1_440 == 0 => {
            format!("{} 天额度", value / 1_440)
        }
        Some(value) if value > 0 && value % 60 == 0 => {
            format!("{} 小时额度", value / 60)
        }
        _ => "额度窗口".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_weekly_rate_limit_event() {
        let line = r#"{"timestamp":"2026-07-28T03:19:01.559Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":2.0,"window_minutes":10080,"resets_at":1785813382},"secondary":null,"credits":null,"plan_type":"plus"}}}"#;
        let snapshot = parse_event_line(line).expect("a valid rate limit event");

        assert_eq!(snapshot.status, "ok");
        assert_eq!(snapshot.source, "local");
        assert_eq!(snapshot.plan_type.as_deref(), Some("plus"));
        assert_eq!(
            snapshot.observed_at.as_deref(),
            Some("2026-07-28T03:19:01.559Z")
        );
        assert_eq!(
            snapshot.primary,
            Some(UsageWindow {
                label: "7 天额度".into(),
                used_percent: 2.0,
                remaining_percent: 98.0,
                resets_at: Some(1_785_813_382),
                window_minutes: Some(10_080),
            })
        );
        assert!(snapshot.secondary.is_none());
    }

    #[test]
    fn parses_two_windows_and_clamps_percentages() {
        let line = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":125,"window_minutes":300,"resets_at":12},"secondary":{"used_percent":42.5,"window_minutes":10080,"resets_at":34},"plan_type":"pro"}}}"#;
        let snapshot = parse_event_line(line).expect("a valid rate limit event");

        assert_eq!(snapshot.primary.unwrap().remaining_percent, 0.0);
        assert_eq!(snapshot.secondary.unwrap().remaining_percent, 57.5);
    }

    #[test]
    fn ignores_unrelated_or_incomplete_events() {
        assert!(
            parse_event_line(r#"{"type":"event_msg","payload":{"type":"task_started"}}"#).is_none()
        );
        assert!(parse_event_line(
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":null}}"#
        )
        .is_none());
        assert!(parse_event_line("not-json").is_none());
    }

    #[test]
    fn chooses_human_window_labels() {
        assert_eq!(window_label(Some(300)), "5 小时额度");
        assert_eq!(window_label(Some(1_440)), "每日额度");
        assert_eq!(window_label(Some(10_080)), "7 天额度");
        assert_eq!(window_label(Some(20_160)), "14 天额度");
        assert_eq!(window_label(None), "额度窗口");
    }

    #[test]
    fn parses_reset_credit_count_and_expirations() {
        let value = serde_json::json!({
            "available_count": 2,
            "credits": [
                { "expires_at": "2026-08-01T00:00:00Z" },
                { "expirationTime": 1_785_715_200_i64 }
            ]
        });
        let snapshot = parse_reset_credit_response(&value);

        assert_eq!(snapshot.count, Some(2));
        assert_eq!(snapshot.expires_at.len(), 2);
        assert_eq!(snapshot.status, "ok");
    }

    #[test]
    fn infers_reset_credit_count_from_grants() {
        let value = serde_json::json!({
            "grants": [{ "expiresAt": "2026-08-01T00:00:00Z" }]
        });
        let snapshot = parse_reset_credit_response(&value);

        assert_eq!(snapshot.count, Some(1));
        assert_eq!(snapshot.expires_at, vec!["2026-08-01T00:00:00Z"]);
    }

    #[test]
    fn parses_online_short_and_weekly_windows() {
        let value = serde_json::json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 3.0,
                    "reset_at": 1_785_895_158_i64,
                    "limit_window_seconds": 18_000
                },
                "secondary_window": {
                    "remainingPercent": 82.5,
                    "resetsAt": "2026-08-05T01:59:18Z",
                    "windowSeconds": 604_800
                }
            }
        });

        let snapshot = parse_usage_response(&value).expect("valid online usage");

        assert_eq!(snapshot.status, "ok");
        assert_eq!(snapshot.source, "online");
        assert_eq!(snapshot.plan_type.as_deref(), Some("plus"));
        assert_eq!(snapshot.primary.unwrap().remaining_percent, 97.0);
        let weekly = snapshot.secondary.unwrap();
        assert_eq!(weekly.remaining_percent, 82.5);
        assert_eq!(weekly.window_minutes, Some(10_080));
        assert_eq!(weekly.resets_at, Some(1_785_895_158));
    }

    #[test]
    fn uses_online_weekly_window_as_the_primary_fallback() {
        let value = serde_json::json!({
            "rateLimit": {
                "primary": {
                    "remaining": 0.75,
                    "resetAt": 1_785_895_158_i64,
                    "periodSeconds": 604_800
                }
            }
        });

        let snapshot = parse_usage_response(&value).expect("weekly fallback");

        assert_eq!(snapshot.primary.unwrap().remaining_percent, 75.0);
        assert!(snapshot.secondary.is_none());
    }

    #[test]
    fn parses_reset_credits_embedded_in_usage_response() {
        let value = serde_json::json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary": {
                    "remainingPercent": 80,
                    "periodSeconds": 604_800
                }
            },
            "rate_limit_reset_credits": {
                "available_count": 2,
                "credits": [
                    { "expires_at": "2026-09-01T00:00:00Z" },
                    { "expires_at": "2026-10-01T00:00:00Z" }
                ]
            }
        });

        let snapshot = parse_usage_response(&value).expect("usage with reset credits");
        let credits = snapshot.reset_credits.expect("embedded reset credits");

        assert_eq!(credits.count, Some(2));
        assert_eq!(credits.expires_at.len(), 2);
    }

    #[test]
    fn includes_retry_after_seconds_in_rate_limit_message() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("90"));

        assert_eq!(
            rate_limited_message(&headers, "重置卡查询过于频繁"),
            "重置卡查询过于频繁，请 90 秒后重试。"
        );
    }
}
