//! Reads Antigravity's account setup and quota answers.

use jiff::Timestamp;
use serde::Deserialize;
use stint_core::{Percentage, QuotaWindow, WindowKind};

use crate::error::ProviderError;

/// What `loadCodeAssist` says about the account.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The Google-managed project quota is read for.
    pub(crate) project: String,
    /// The subscription, such as "Pro", when the answer names one.
    pub(crate) plan: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct LoadResponse {
    cloudaicompanion_project: Option<Project>,
    current_tier: Option<Tier>,
    paid_tier: Option<Tier>,
    ineligible_tiers: Vec<Ineligible>,
}

/// A project id, sent as a string or as an object holding it.
#[derive(Deserialize)]
#[serde(untagged)]
enum Project {
    Id(String),
    Object {
        id: Option<String>,
        #[serde(rename = "projectId")]
        project_id: Option<String>,
    },
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Tier {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Ineligible {
    reason_code: Option<String>,
    reason_message: Option<String>,
}

/// Parses `loadCodeAssist`. An account without a project cannot report quota yet: it has
/// to finish setting up in Antigravity, or is not eligible where it is.
pub(crate) fn setup(bytes: &[u8]) -> Result<Setup, ProviderError> {
    let response: LoadResponse = serde_json::from_slice(bytes)?;
    let project = response
        .cloudaicompanion_project
        .and_then(|project| match project {
            Project::Id(id) => Some(id),
            Project::Object { id, project_id } => id.or(project_id),
        });
    let project = project.filter(|id| !id.is_empty()).ok_or_else(|| {
        let reason = response.ineligible_tiers.iter().find_map(|tier| {
            tier.reason_message
                .clone()
                .or_else(|| tier.reason_code.clone())
        });
        ProviderError::AccountSetup(
            reason.unwrap_or_else(|| "finish setting up the account in Antigravity".to_owned()),
        )
    })?;
    let plan = plan(response.paid_tier.as_ref()).or_else(|| plan(response.current_tier.as_ref()));
    Ok(Setup { project, plan })
}

/// "Ultra", "Pro", "Plus", or "Free", from a tier's id or name. Generic tiers such as
/// `standard-tier` say nothing about the subscription.
fn plan(tier: Option<&Tier>) -> Option<String> {
    let tier = tier?;
    let id = tier.id.as_deref().unwrap_or_default();
    let name = tier.name.as_deref().unwrap_or_default();
    let named = |word: &str| {
        name.split_whitespace()
            .any(|part| part.eq_ignore_ascii_case(word))
    };
    let plan = if id == "g1-ultra-tier" || named("Ultra") {
        "Ultra"
    } else if id == "g1-pro-tier" || named("Pro") {
        "Pro"
    } else if named("Plus") {
        "Plus"
    } else if id == "free-tier" {
        "Free"
    } else {
        return None;
    };
    Some(plan.to_owned())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Summary {
    groups: Vec<Group>,
    // Some answers wrap the summary.
    response: Option<Box<Summary>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Group {
    display_name: Option<String>,
    buckets: Vec<Bucket>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Bucket {
    #[serde(rename = "bucketId")]
    id: Option<String>,
    window: Option<String>,
    remaining_fraction: Option<f64>,
    reset_time: Option<String>,
    disabled: bool,
}

/// Parses `retrieveUserQuotaSummary`: each model group's five-hour and weekly windows,
/// scoped to the group, such as `gemini` or `3p` for Claude and GPT models.
///
/// Disabled buckets, buckets without a remaining fraction, and windows of other lengths
/// are left out. A window reported twice keeps its most used reading.
pub(crate) fn summary(bytes: &[u8]) -> Result<Vec<QuotaWindow>, ProviderError> {
    let mut summary: Summary = serde_json::from_slice(bytes)?;
    if summary.groups.is_empty()
        && let Some(inner) = summary.response.take()
    {
        summary = *inner;
    }
    let mut windows: Vec<QuotaWindow> = Vec::new();
    for group in &summary.groups {
        for bucket in group.buckets.iter().filter(|bucket| !bucket.disabled) {
            let Some(window) = window(group, bucket) else {
                continue;
            };
            match windows
                .iter_mut()
                .find(|known| known.kind == window.kind && known.scope == window.scope)
            {
                Some(known) if known.used.get() < window.used.get() => *known = window,
                Some(_) => {}
                None => windows.push(window),
            }
        }
    }
    Ok(windows)
}

fn window(group: &Group, bucket: &Bucket) -> Option<QuotaWindow> {
    let id = bucket.id.as_deref().unwrap_or_default();
    let (stem, kind_from_id) = if let Some(stem) = id.strip_suffix("-5h") {
        (stem, Some(WindowKind::FiveHour))
    } else if let Some(stem) = id.strip_suffix("-weekly") {
        (stem, Some(WindowKind::SevenDay))
    } else {
        (id, None)
    };
    let kind_from_window = match bucket.window.as_deref() {
        Some("5h") => Some(WindowKind::FiveHour),
        Some("weekly" | "7d") => Some(WindowKind::SevenDay),
        Some(_) => return None,
        None => None,
    };
    let kind = match (kind_from_window, kind_from_id) {
        (Some(a), Some(b)) if a != b => return None,
        (Some(kind), _) | (None, Some(kind)) => kind,
        (None, None) => return None,
    };
    let scope = if stem.is_empty() {
        group.display_name.as_deref()?.to_lowercase()
    } else {
        stem.to_owned()
    };
    let remaining = bucket
        .remaining_fraction
        .filter(|fraction| (0.0..=1.0).contains(fraction))?;
    Some(QuotaWindow {
        kind,
        scope: Some(scope),
        used: Percentage::new(100.0 * (1.0 - remaining)).ok()?,
        resets_at: bucket
            .reset_time
            .as_deref()
            .and_then(|at| at.parse::<Timestamp>().ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_reads_project_and_paid_plan() {
        let setup = setup(
            br#"{"cloudaicompanionProject":"managed-1",
                "currentTier":{"id":"standard-tier","name":"Gemini Code Assist"},
                "paidTier":{"id":"g1-ultra-tier","name":"Google AI Ultra"}}"#,
        )
        .unwrap();
        assert_eq!(
            setup,
            Setup {
                project: "managed-1".into(),
                plan: Some("Ultra".into())
            }
        );
    }

    #[test]
    fn setup_accepts_project_objects_and_free_tiers() {
        let setup = setup(
            br#"{"cloudaicompanionProject":{"id":"managed-2"},"currentTier":{"id":"free-tier"}}"#,
        )
        .unwrap();
        assert_eq!(setup.project, "managed-2");
        assert_eq!(setup.plan.as_deref(), Some("Free"));
        let pro =
            super::setup(br#"{"cloudaicompanionProject":"p","paidTier":{"name":"Google AI Pro"}}"#)
                .unwrap();
        assert_eq!(pro.plan.as_deref(), Some("Pro"));
        let generic = super::setup(
            br#"{"cloudaicompanionProject":"p","currentTier":{"id":"standard-tier"}}"#,
        )
        .unwrap();
        assert_eq!(generic.plan, None);
    }

    #[test]
    fn setup_without_project_says_why() {
        let err = setup(
            br#"{"ineligibleTiers":[{"reasonCode":"UNSUPPORTED_LOCATION",
                "reasonMessage":"Not available in your country"}]}"#,
        )
        .unwrap_err();
        assert!(
            matches!(err, ProviderError::AccountSetup(reason) if reason == "Not available in your country")
        );
        assert!(matches!(setup(b"{}"), Err(ProviderError::AccountSetup(_))));
    }

    #[test]
    fn summary_scopes_windows_to_groups() {
        let windows = summary(
            br#"{"groups":[
                {"displayName":"Gemini Models","buckets":[
                    {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.75,"resetTime":"2026-11-26T15:00:00Z"},
                    {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.4,"resetTime":"2026-11-29T10:00:00.123Z"}]},
                {"displayName":"Claude and GPT models","buckets":[
                    {"bucketId":"3p-5h","window":"5h","disabled":true},
                    {"bucketId":"3p-weekly","remainingFraction":0}]}]}"#,
        )
        .unwrap();
        let expected = [
            (WindowKind::FiveHour, "gemini", 25.0),
            (WindowKind::SevenDay, "gemini", 60.0),
            (WindowKind::SevenDay, "3p", 100.0),
        ];
        assert_eq!(windows.len(), expected.len());
        for (window, (kind, scope, used)) in windows.iter().zip(expected) {
            assert_eq!(window.kind, kind);
            assert_eq!(window.scope.as_deref(), Some(scope));
            assert!((window.used.get() - used).abs() < 1e-9);
        }
        assert!(windows[1].resets_at.is_some());
        assert!(windows[2].resets_at.is_none());
    }

    #[test]
    fn summary_skips_unclear_buckets_and_keeps_the_worst_duplicate() {
        let windows = summary(
            br#"{"response":{"groups":[{"buckets":[
                {"bucketId":"gemini-5h","window":"weekly","remainingFraction":0.5},
                {"bucketId":"daily-thing","window":"daily","remainingFraction":0.5},
                {"bucketId":"gemini-weekly","remainingFraction":1.5},
                {"bucketId":"x-5h","remainingFraction":0.9},
                {"bucketId":"x-5h","remainingFraction":0.2}]}]}}"#,
        )
        .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].scope.as_deref(), Some("x"));
        assert!((windows[0].used.get() - 80.0).abs() < 1e-9);
    }
}
