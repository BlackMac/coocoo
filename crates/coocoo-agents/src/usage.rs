//! `agents.usage`: a provider's subscription quota, read with `omp usage --json`. omp knows how
//! each provider meters (5-hour and weekly windows, monthly credits, extra usage in USD, model
//! tiers with their own pool, several accounts); this module only normalizes its report.

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
    #[serde(default)]
    capacity: HashMap<String, Vec<OmpCapacity>>,
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
struct OmpScope {
    tier: Option<String>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OmpCapacity {
    window: String,
    duration_ms: Option<f64>,
    meter: Option<String>,
    accounts: u32,
    used_accounts: f64,
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

fn normalize(provider: &str, mut usage: OmpUsage, now_ms: i64) -> ProviderUsage {
    let capacity = usage.capacity.remove(provider).unwrap_or_default();
    let windows = capacity
        .into_iter()
        .map(|c| {
            let duration_ms = c.duration_ms.map(|d| d as u64);
            // The earliest reset of the limits omp summed into this window.
            let resets_at = duration_ms.and_then(|duration| {
                usage
                    .reports
                    .iter()
                    .flat_map(|r| r.limits.iter())
                    .filter(|l| {
                        l.window.as_ref().and_then(|w| w.duration_ms).map(|d| d as u64) == Some(duration)
                            && meter(provider, l).as_deref() == c.meter.as_deref()
                    })
                    .filter_map(|l| l.window.as_ref()?.resets_at.map(|t| t as i64))
                    .filter(|&t| t > now_ms)
                    .min()
            });
            UsageCapacity { window: c.window, duration_ms, meter: c.meter, accounts: c.accounts, used: c.used_accounts, resets_at }
        })
        .collect();
    let accounts = usage
        .reports
        .into_iter()
        .map(|r| {
            let text = |key: &str| r.metadata.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
            let org = text("orgName");
            let label = text("email").or_else(|| org.clone()).or_else(|| text("accountId")).unwrap_or_else(|| provider.to_owned());
            UsageAccount {
                org: org.filter(|o| *o != label),
                plan: text("planType").or_else(|| text("plan")),
                label,
                limits: r.limits.into_iter().map(limit).collect(),
            }
        })
        .collect();
    ProviderUsage { provider: provider.to_owned(), fetched_at: now_ms, windows, accounts }
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

/// omp's `meterForLimit`: the pool a limit belongs to inside its window, matching the `meter`
/// of the capacity entries. Only Anthropic and Codex keep separate pools.
fn meter(provider: &str, l: &OmpLimit) -> Option<String> {
    if provider != "anthropic" && provider != "openai-codex" {
        return None;
    }
    if let Some(tier) = l.scope.tier.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        return Some(tier.to_lowercase());
    }
    if provider != "openai-codex" {
        return None;
    }
    let id = l.id.to_lowercase();
    match id.split(':').nth(1) {
        Some(slug) if !slug.is_empty() && slug != "primary" && slug != "secondary" => Some(slug.to_owned()),
        _ => Some("chat".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;

    /// Two Anthropic accounts as `omp usage --json` reports them (trimmed): the umbrella 7-day
    /// window and a model-tier pool share a duration but are separate capacity entries.
    fn sample() -> OmpUsage {
        serde_json::from_value(serde_json::json!({
            "reports": [
                {
                    "provider": "anthropic",
                    "limits": [
                        { "id": "anthropic:5h", "label": "Claude 5 Hour", "scope": { "provider": "anthropic", "shared": true },
                          "window": { "id": "5h", "label": "5 Hour", "durationMs": 18000000 },
                          "amount": { "used": 0, "limit": 100, "usedFraction": 0, "unit": "percent" }, "status": "ok" },
                        { "id": "anthropic:7d", "label": "Claude 7 Day", "scope": { "provider": "anthropic", "shared": true },
                          "window": { "id": "7d", "label": "7 Day", "durationMs": 604800000, "resetsAt": 3_000_000 },
                          "amount": { "used": 100, "limit": 100, "usedFraction": 1, "unit": "percent" }, "status": "exhausted" },
                        { "id": "anthropic:7d:fable", "label": "Claude 7 Day (Fable)", "scope": { "provider": "anthropic", "tier": "fable" },
                          "window": { "id": "7d", "label": "7 Day", "durationMs": 604800000, "resetsAt": 9_000_000 },
                          "amount": { "used": 1, "limit": 100, "usedFraction": 0.01, "unit": "percent" }, "status": "ok" },
                        { "id": "anthropic:extra", "label": "Claude Extra Usage", "scope": { "provider": "anthropic" },
                          "amount": { "used": 1.92, "limit": 5, "unit": "usd" }, "status": "ok" }
                    ],
                    "metadata": { "email": "a@example.com", "orgName": "Team A" }
                },
                {
                    "provider": "anthropic",
                    "limits": [
                        { "id": "anthropic:7d", "label": "Claude 7 Day", "scope": { "provider": "anthropic", "shared": true },
                          "window": { "id": "7d", "label": "7 Day", "durationMs": 604800000, "resetsAt": 2_000_000 },
                          "amount": { "remainingFraction": 0.05, "unit": "percent" }, "status": "warning" }
                    ],
                    "metadata": { "accountId": "acct-b" }
                }
            ],
            "capacity": {
                "anthropic": [
                    { "window": "5h", "durationMs": 18000000, "accounts": 1, "usedAccounts": 0, "remainingAccounts": 1 },
                    { "window": "7d", "durationMs": 604800000, "accounts": 2, "usedAccounts": 1.95, "remainingAccounts": 0.05 },
                    { "window": "7d", "durationMs": 604800000, "meter": "fable", "accounts": 1, "usedAccounts": 0.01, "remainingAccounts": 0.99 },
                    { "window": "extra", "accounts": 1, "usedAccounts": 0.384, "remainingAccounts": 0.616 }
                ]
            }
        }))
        .unwrap()
    }

    #[test]
    fn window_resets_come_from_its_own_pool() {
        let usage = normalize("anthropic", sample(), NOW);
        let resets: Vec<_> = usage.windows.iter().map(|w| (w.window.as_str(), w.meter.as_deref(), w.resets_at)).collect();
        assert_eq!(
            resets,
            [("5h", None, None), ("7d", None, Some(2_000_000)), ("7d", Some("fable"), Some(9_000_000)), ("extra", None, None)]
        );
    }

    #[test]
    fn past_resets_are_ignored() {
        let usage = normalize("anthropic", sample(), 2_500_000);
        assert_eq!(usage.windows[1].resets_at, Some(3_000_000));
    }

    #[test]
    fn accounts_and_fractions() {
        let usage = normalize("anthropic", sample(), NOW);
        let a = &usage.accounts[0];
        assert_eq!((a.label.as_str(), a.org.as_deref()), ("a@example.com", Some("Team A")));
        let extra = a.limits.iter().find(|l| l.id == "anthropic:extra").unwrap();
        assert!((extra.used_fraction.unwrap() - 0.384).abs() < 1e-9);
        assert_eq!(extra.window, None);
        let b = &usage.accounts[1];
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
