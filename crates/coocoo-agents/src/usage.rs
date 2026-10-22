//! `agents.usage`: a provider's subscription quota, read with `omp usage --json`. omp knows how
//! each provider meters (5-hour and weekly windows, monthly credits, extra usage in USD, several
//! accounts); this module groups its limits into per-window quota pools and marks the pools the
//! asking agent's model draws from.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use coocoo_proto::{ApiError, ApiResult, ProviderUsage, UsageAccount, UsageCapacity, UsageLimit};
use serde::Deserialize;
use serde_json::{Map, Value};

/// How long one fetch is reused: quota moves slowly and every fetch spawns omp, which calls the
/// provider's usage endpoint for each account.
pub(crate) const USAGE_TTL: Duration = Duration::from_secs(60);
/// Below the 30 s peer request timeout, so a slow provider fails with a clear message.
const USAGE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
struct OmpUsage {
    #[serde(default)]
    reports: Vec<OmpReport>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OmpReport {
    #[serde(default)]
    limits: Vec<OmpLimit>,
    #[serde(default)]
    metadata: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OmpLimit {
    id: String,
    label: String,
    #[serde(default)]
    scope: OmpScope,
    window: Option<OmpWindow>,
    amount: OmpAmount,
    status: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct OmpScope {
    tier: Option<String>,
    window_id: Option<String>,
    /// Identity of one upstream quota reported once per routing family (Antigravity's shared
    /// Claude & GPT counter).
    shared_group: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OmpWindow {
    label: String,
    duration_ms: Option<f64>,
    resets_at: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OmpAmount {
    used: Option<f64>,
    limit: Option<f64>,
    used_fraction: Option<f64>,
    remaining_fraction: Option<f64>,
    #[serde(default = "unknown_unit")]
    unit: String,
}

fn unknown_unit() -> String {
    "unknown".to_owned()
}

/// Provider ids are passed to omp as an argument value: keep them to id characters.
pub(crate) fn valid_provider(provider: &str) -> bool {
    !provider.is_empty()
        && !provider.starts_with('-')
        && provider.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Run `omp usage --json --provider <provider>` and normalize its report.
pub(crate) async fn fetch(omp: &Path, cwd: &Path, provider: &str, now_ms: i64) -> ApiResult<ProviderUsage> {
    tokio::fs::create_dir_all(cwd).await?;
    let mut command = tokio::process::Command::new(omp);
    command
        .args(["usage", "--json", "--provider", provider])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(USAGE_TIMEOUT, command.output())
        .await
        .map_err(|_| ApiError::unavailable("omp usage timed out"))?
        .map_err(|e| ApiError::unavailable(format!("cannot run omp usage: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("no output");
        return Err(ApiError::unavailable(format!("omp usage failed: {reason}")));
    }
    let report: OmpUsage = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::internal(format!("unexpected omp usage output: {e}")))?;
    Ok(normalize(provider, report, now_ms))
}

/// Mark the windows that gate `model` (the model id within the provider): provider-wide pools
/// always, a separate pool when the model draws from it.
pub(crate) fn mark_gating(usage: &mut ProviderUsage, model: Option<&str>) {
    let model = model.map(str::to_lowercase);
    let own = model.as_deref().and_then(|m| model_pool(&usage.provider, m));
    for w in &mut usage.windows {
        w.gating = w.pools.is_empty()
            || model.as_deref().is_some_and(|m| w.pools.iter().any(|p| Some(p.as_str()) == own || m.contains(p.as_str())));
    }
}

/// One window × pool while summing accounts.
struct Bucket {
    capacity: UsageCapacity,
    fractions: Vec<f64>,
}

fn normalize(provider: &str, usage: OmpUsage, now_ms: i64) -> ProviderUsage {
    let mut buckets: Vec<Bucket> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for report in &usage.reports {
        // Each account adds its binding (highest) used fraction per window and pool.
        let mut account_max: HashMap<usize, f64> = HashMap::new();
        for l in &report.limits {
            let Some(fraction) = used_fraction(&l.amount) else { continue };
            let duration_ms = l.window.as_ref().and_then(|w| w.duration_ms).map(|d| d as u64);
            let window = match duration_ms {
                Some(ms) => short_duration(ms),
                None => l.window.as_ref().map(|w| w.label.clone()).or_else(|| l.scope.window_id.clone()).unwrap_or_else(|| l.label.clone()),
            };
            let pool = pool(provider, l);
            let group = l.scope.shared_group.clone().or_else(|| pool.clone()).unwrap_or_default();
            let i = *index.entry((window.clone(), group)).or_insert_with(|| {
                buckets.push(Bucket {
                    capacity: UsageCapacity {
                        window,
                        duration_ms,
                        name: pool_name(provider, l, pool.as_deref()),
                        pools: Vec::new(),
                        gating: false,
                        accounts: 0,
                        used: 0.0,
                        resets_at: None,
                    },
                    fractions: Vec::new(),
                });
                buckets.len() - 1
            });
            let capacity = &mut buckets[i].capacity;
            if let Some(pool) = pool
                && !capacity.pools.contains(&pool)
            {
                capacity.pools.push(pool);
            }
            if let Some(reset) = l.window.as_ref().and_then(|w| w.resets_at).map(|t| t as i64)
                && reset > now_ms
            {
                capacity.resets_at = Some(capacity.resets_at.map_or(reset, |r| r.min(reset)));
            }
            let max = account_max.entry(i).or_insert(fraction);
            *max = max.max(fraction);
        }
        for (i, fraction) in account_max {
            buckets[i].fractions.push(fraction);
        }
    }
    let mut windows: Vec<UsageCapacity> = buckets
        .into_iter()
        .map(|b| UsageCapacity { accounts: b.fractions.len() as u32, used: b.fractions.iter().sum(), ..b.capacity })
        .collect();
    windows.sort_by(|a, b| a.duration_ms.unwrap_or(u64::MAX).cmp(&b.duration_ms.unwrap_or(u64::MAX)).then_with(|| a.name.cmp(&b.name)));

    let accounts = usage
        .reports
        .into_iter()
        .map(|r| {
            let text = |key: &str| r.metadata.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            let org = text("orgName");
            let label = text("email").or_else(|| org.clone()).or_else(|| text("accountId")).unwrap_or_else(|| provider.to_owned());
            let plan = text("planType").or_else(|| text("plan"));
            // A shared quota reported once per routing family is one row.
            let mut seen = Vec::new();
            let limits = r
                .limits
                .into_iter()
                .filter(|l| match &l.scope.shared_group {
                    Some(group) if seen.contains(group) => false,
                    Some(group) => {
                        seen.push(group.clone());
                        true
                    }
                    None => true,
                })
                .map(limit)
                .collect();
            UsageAccount { org: org.filter(|o| *o != label), plan, label, limits }
        })
        .collect();
    ProviderUsage { provider: provider.to_owned(), fetched_at: now_ms, windows, accounts }
}

/// `5h`, `7d`, `30d`, as omp labels window durations.
fn short_duration(ms: u64) -> String {
    const MIN: u64 = 60_000;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    if ms >= DAY && ms.is_multiple_of(DAY) {
        format!("{}d", ms / DAY)
    } else if ms >= HOUR && ms.is_multiple_of(HOUR) {
        format!("{}h", ms / HOUR)
    } else {
        format!("{}m", ms / MIN)
    }
}

/// The separate quota pool a limit draws from inside its window, as omp's catalog
/// (`quota-tiers`) scopes them: Anthropic model tiers, Codex chat versus Spark, Antigravity's
/// per-family backend counters (the second segment of its limit ids). `None` = provider-wide.
fn pool(provider: &str, l: &OmpLimit) -> Option<String> {
    let tier = l.scope.tier.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_lowercase);
    let segment = || l.id.split(':').nth(1).map(str::to_lowercase).filter(|s| !s.is_empty());
    match provider {
        "anthropic" => tier,
        "openai-codex" => tier.or_else(|| Some(segment().filter(|s| s != "primary" && s != "secondary").unwrap_or_else(|| "chat".to_owned()))),
        "google-antigravity" => segment().filter(|s| s != "default"),
        _ => None,
    }
}

/// The pool `model` draws from where the model id alone does not name it (omp's catalog
/// `quota-tiers`); tier pools such as Anthropic's are matched by name in the id.
fn model_pool(provider: &str, model: &str) -> Option<&'static str> {
    match provider {
        "openai-codex" => Some(if model.contains("-spark") { "spark" } else { "chat" }),
        "google-antigravity" => {
            if model.contains("claude-") {
                Some("anthropic")
            } else if ["gemini-", "gemma-", "tab_"].iter().any(|p| model.contains(p)) {
                Some("google")
            } else if model.contains("gpt-") || model.starts_with("openai/") {
                Some("openai")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Display name of a separate pool: Antigravity labels its counters (`Gemini`, `Claude & GPT`);
/// elsewhere the pool id is the name, except Codex's default `chat` pool.
fn pool_name(provider: &str, l: &OmpLimit, pool: Option<&str>) -> Option<String> {
    let pool = pool?;
    if provider == "google-antigravity" {
        return Some(l.label.trim_end_matches(" (shared)").to_owned());
    }
    (pool != "chat").then(|| pool.to_owned())
}

fn limit(l: OmpLimit) -> UsageLimit {
    let a = &l.amount;
    UsageLimit {
        used_fraction: used_fraction(a),
        window: l.window.as_ref().map(|w| w.label.clone()),
        resets_at: l.window.as_ref().and_then(|w| w.resets_at).map(|t| t as i64),
        unit: a.unit.clone(),
        used: a.used,
        limit: a.limit,
        status: l.status,
        id: l.id,
        label: l.label,
    }
}

/// omp's `resolveUsedFraction`: explicit fraction, then used/limit, then a percent `used`,
/// then the inverted remaining fraction.
fn used_fraction(a: &OmpAmount) -> Option<f64> {
    if a.used_fraction.is_some() {
        return a.used_fraction;
    }
    if let (Some(used), Some(limit)) = (a.used, a.limit)
        && limit > 0.0
    {
        return Some(used / limit);
    }
    if a.unit == "percent"
        && let Some(used) = a.used
    {
        return Some(used / 100.0);
    }
    a.remaining_fraction.map(|r| (1.0 - r).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;
    const H5: u64 = 18_000_000;
    const D7: u64 = 604_800_000;

    fn usage(value: Value) -> OmpUsage {
        serde_json::from_value(value).unwrap()
    }

    fn limit(id: &str, label: &str, scope: Value, window: Option<(u64, i64)>, fraction: f64) -> Value {
        let window = window.map(|(d, r)| serde_json::json!({ "id": "w", "label": format!("{d}"), "durationMs": d, "resetsAt": r }));
        serde_json::json!({ "id": id, "label": label, "scope": scope, "window": window,
            "amount": { "usedFraction": fraction, "unit": "percent" } })
    }

    /// Two Anthropic accounts: the umbrella 7-day window and the Fable tier's own 7-day pool.
    fn anthropic() -> OmpUsage {
        usage(serde_json::json!({ "reports": [
            { "limits": [
                limit("anthropic:5h", "Claude 5 Hour", serde_json::json!({}), Some((H5, 0)), 0.0),
                limit("anthropic:7d", "Claude 7 Day", serde_json::json!({}), Some((D7, 3_000_000)), 1.0),
                limit("anthropic:7d:fable", "Claude 7 Day (Fable)", serde_json::json!({ "tier": "fable" }), Some((D7, 9_000_000)), 0.01),
                { "id": "anthropic:extra", "label": "Claude Extra Usage", "scope": { "windowId": "extra" },
                  "amount": { "used": 1.92, "limit": 5, "unit": "usd" } }
              ], "metadata": { "email": "a@example.com", "orgName": "Team A" } },
            { "limits": [
                { "id": "anthropic:7d", "label": "Claude 7 Day", "window": { "label": "7 Day", "durationMs": D7, "resetsAt": 2_000_000 },
                  "amount": { "remainingFraction": 0.05, "unit": "percent" } }
              ], "metadata": { "accountId": "acct-b" } }
        ]}))
    }

    /// One Antigravity account: a Gemini counter and the Claude & GPT counter, which omp reports
    /// once per family under one shared group.
    fn antigravity() -> OmpUsage {
        let shared = |family: &str, w: &str| serde_json::json!({ "sharedGroup": format!("3p-{w}"), "shared": true, "tier": family });
        usage(serde_json::json!({ "reports": [{ "limits": [
            limit("google-antigravity:google:default:gemini-weekly", "Gemini", serde_json::json!({}), Some((D7, 5_000_000)), 0.33),
            limit("google-antigravity:google:default:gemini-5h", "Gemini", serde_json::json!({}), Some((H5, 2_000_000)), 0.0),
            limit("google-antigravity:anthropic:default:3p-weekly", "Claude & GPT (shared)", shared("anthropic", "weekly"), Some((D7, 7_000_000)), 0.1),
            limit("google-antigravity:openai:default:3p-weekly", "Claude & GPT (shared)", shared("openai", "weekly"), Some((D7, 7_000_000)), 0.1),
            limit("google-antigravity:anthropic:default:3p-5h", "Claude & GPT (shared)", shared("anthropic", "5h"), Some((H5, 2_000_000)), 0.0),
            limit("google-antigravity:openai:default:3p-5h", "Claude & GPT (shared)", shared("openai", "5h"), Some((H5, 2_000_000)), 0.0)
        ], "metadata": { "email": "g@example.com" } }] }))
    }

    fn windows(u: &ProviderUsage) -> Vec<(&str, Option<&str>, bool, f64)> {
        u.windows.iter().map(|w| (w.window.as_str(), w.name.as_deref(), w.gating, (w.used * 100.0).round() / 100.0)).collect()
    }

    #[test]
    fn anthropic_tier_pool_gates_only_its_model() {
        let mut u = normalize("anthropic", anthropic(), NOW);
        mark_gating(&mut u, Some("claude-opus-5-5"));
        assert_eq!(
            windows(&u),
            [("5h", None, true, 0.0), ("7d", None, true, 1.95), ("7d", Some("fable"), false, 0.01), ("extra", None, true, 0.38)]
        );
        mark_gating(&mut u, Some("claude-fable-5"));
        assert!(u.windows[2].gating);
    }

    #[test]
    fn antigravity_counters_stay_apart_and_follow_the_model() {
        let mut u = normalize("google-antigravity", antigravity(), NOW);
        mark_gating(&mut u, Some("claude-opus-4-6"));
        assert_eq!(
            windows(&u),
            [
                ("5h", Some("Claude & GPT"), true, 0.0),
                ("5h", Some("Gemini"), false, 0.0),
                ("7d", Some("Claude & GPT"), true, 0.1),
                ("7d", Some("Gemini"), false, 0.33)
            ]
        );
        mark_gating(&mut u, Some("gemini-3-pro"));
        assert_eq!(u.windows.iter().filter(|w| w.gating).map(|w| w.name.as_deref()).collect::<Vec<_>>(), [Some("Gemini"); 2]);
        mark_gating(&mut u, Some("gpt-oss-120b"));
        assert_eq!(u.windows.iter().filter(|w| w.gating).map(|w| w.name.as_deref()).collect::<Vec<_>>(), [Some("Claude & GPT"); 2]);
    }

    #[test]
    fn shared_quota_is_one_account_row() {
        let u = normalize("google-antigravity", antigravity(), NOW);
        let ids: Vec<_> = u.accounts[0].limits.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "google-antigravity:google:default:gemini-weekly",
                "google-antigravity:google:default:gemini-5h",
                "google-antigravity:anthropic:default:3p-weekly",
                "google-antigravity:anthropic:default:3p-5h"
            ]
        );
    }

    #[test]
    fn window_resets_are_the_earliest_future_one() {
        let u = normalize("anthropic", anthropic(), NOW);
        assert_eq!(u.windows[1].resets_at, Some(2_000_000));
        let u = normalize("anthropic", anthropic(), 2_500_000);
        assert_eq!(u.windows[1].resets_at, Some(3_000_000));
    }

    #[test]
    fn accounts_and_fractions() {
        let u = normalize("anthropic", anthropic(), NOW);
        let a = &u.accounts[0];
        assert_eq!((a.label.as_str(), a.org.as_deref()), ("a@example.com", Some("Team A")));
        let extra = a.limits.iter().find(|l| l.id == "anthropic:extra").unwrap();
        assert!((extra.used_fraction.unwrap() - 0.384).abs() < 1e-9);
        assert_eq!(extra.window, None);
        let b = &u.accounts[1];
        assert_eq!((b.label.as_str(), b.org.as_deref()), ("acct-b", None));
        assert!((b.limits[0].used_fraction.unwrap() - 0.95).abs() < 1e-9);
    }

    #[test]
    fn provider_ids() {
        assert!(valid_provider("openai-codex"));
        assert!(valid_provider("google-antigravity"));
        assert!(!valid_provider(""));
        assert!(!valid_provider("--json"));
        assert!(!valid_provider("a b"));
    }
}
