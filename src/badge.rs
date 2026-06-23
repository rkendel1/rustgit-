use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    detect_overlay_repository_context, executions_start_endpoint, hash_key, BadgeGenerateRequest,
    ExecutionStartRequest, OverlayRepositoryContext,
};

pub(crate) fn parse_badge_repository_context(repo_url: &str) -> Option<OverlayRepositoryContext> {
    let trimmed = repo_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let without_suffix = without_query.trim_end_matches('/').trim_end_matches(".git");
    let without_scheme = without_suffix
        .strip_prefix("https://")
        .or_else(|| without_suffix.strip_prefix("http://"))
        .unwrap_or(without_suffix);
    let (host, path) = without_scheme
        .split_once('/')
        .unwrap_or((without_scheme, ""));
    let host = host.to_ascii_lowercase();
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    let normalized = format!("https://github.com/{}", path.trim_start_matches('/'));
    detect_overlay_repository_context(&normalized)
}

fn normalize_badge_mode(mode: Option<&str>) -> &'static str {
    match mode
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("wasm") => "wasm",
        Some("docker") => "docker",
        _ => "auto",
    }
}

fn normalize_badge_visibility(visibility: Option<&str>) -> &'static str {
    match visibility
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("private") => "private",
        _ => "public",
    }
}

pub fn badge_generate_endpoint(request: &BadgeGenerateRequest) -> (String, String) {
    let endpoint = "/api/badges/generate".to_string();
    let Some(context) = parse_badge_repository_context(&request.repo_url) else {
        return (
            endpoint,
            json!({
                "error": "invalid_github_repo_url",
                "message": "Expected a GitHub repository URL like https://github.com/owner/repo"
            })
            .to_string(),
        );
    };

    let branch = request
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(context.branch.as_str());
    let mode = normalize_badge_mode(request.mode.as_deref());
    let visibility = normalize_badge_visibility(request.visibility.as_deref());

    let owner = context.owner;
    let repo = context.repo;
    let canonical_repo_url = format!("https://github.com/{owner}/{repo}");
    let execution_profile_id = hash_key(&canonical_repo_url);
    let badge_url = format!("https://api.trythissoftware.com/badge/{owner}/{repo}.svg");
    let seed_url = format!("https://trythissoftware.com/seed/{owner}/{repo}");
    let alt_text = format!("{owner}/{repo} execution status badge");
    let markdown_embed = format!("[<img src=\"{badge_url}\" alt=\"{alt_text}\">]({seed_url})");
    let html_embed =
        format!("<a href=\"{seed_url}\">\n  <img src=\"{badge_url}\" alt=\"{alt_text}\">\n</a>");

    (
        endpoint,
        json!({
            "repo": {
                "owner": owner,
                "name": repo,
                "url": canonical_repo_url,
                "branch": branch
            },
            "config": {
                "mode": mode,
                "visibility": visibility
            },
            "badge_url": badge_url,
            "seed_url": seed_url,
            "embed_snippets": {
                "markdown": markdown_embed,
                "html": html_embed,
                "raw_badge_url": badge_url,
                "seed_link": seed_url
            },
            "execution_profile": {
                "repo_id": execution_profile_id,
                "repo_url": canonical_repo_url,
                "runtime_preference": mode,
                "analysis_status": "pending",
                "analyze_endpoint": "/api/v1/repositories/analyze"
            },
            "legacy_generate_api": "/api/badge/generate",
            "config_variants": {
                "runtime_preference": ["auto", "wasm", "docker"],
                "visibility_mode": ["public", "private"]
            },
            "badge_types": ["default", "execution_ready", "broken", "healing", "verified"],
            "auto_update_notice": "This badge updates automatically based on repository execution health.",
            "state_model": "Badge is a pointer, not state. The SVG resolves from live execution state."
        })
        .to_string(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeRuntimeState {
    Runnable,
    Verified,
    Healed,
    Untested,
    ProductionReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    Unverified,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    pub id: String,
    pub github_owner: String,
    pub github_repo: String,
    pub default_branch: String,
    pub first_seen_at: u64,
    pub last_seen_at: u64,
    pub repository_fingerprint: String,
    pub health_score: f32,
    pub execution_score: f32,
    pub healing_score: f32,
    pub verification_state: VerificationState,
    pub badge_state: BadgeRuntimeState,
    pub current_workspace_id: Option<String>,
    pub latest_execution_id: Option<String>,
    pub latest_successful_execution_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BadgeExecutionSnapshot {
    pub health_score: f32,
    pub execution_readiness: f32,
    pub last_run_status: String,
    pub has_execution_history: bool,
    pub healed_artifact_available: bool,
}

fn badge_state_label(state: BadgeRuntimeState) -> (&'static str, &'static str, &'static str) {
    match state {
        BadgeRuntimeState::Runnable => ("Runnable", "#facc15", "🟡"),
        BadgeRuntimeState::Verified => ("Verified", "#22c55e", "🟢"),
        BadgeRuntimeState::Healed => ("Healed", "#38bdf8", "🔵"),
        BadgeRuntimeState::Untested => ("Untested", "#94a3b8", "⚪"),
        BadgeRuntimeState::ProductionReady => ("Production Ready", "#16a34a", "🟢"),
    }
}

fn escape_svg_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn derive_badge_runtime_state(snapshot: &BadgeExecutionSnapshot) -> BadgeRuntimeState {
    if snapshot.healed_artifact_available {
        return BadgeRuntimeState::Healed;
    }
    if !snapshot.has_execution_history {
        return BadgeRuntimeState::Untested;
    }
    if snapshot.last_run_status.eq_ignore_ascii_case("success")
        && snapshot.health_score >= 95.0
        && snapshot.execution_readiness >= 0.9
    {
        return BadgeRuntimeState::ProductionReady;
    }
    if snapshot.last_run_status.eq_ignore_ascii_case("success") {
        return BadgeRuntimeState::Verified;
    }
    BadgeRuntimeState::Runnable
}

const BADGE_PADDING_WIDTH: i32 = 32;
const BADGE_CHAR_WIDTH: i32 = 6;
const BADGE_MIN_LEFT_WIDTH: i32 = 48;
const BADGE_MIN_RIGHT_WIDTH: i32 = 78;

pub fn badge_svg_endpoint(
    owner: &str,
    repo: &str,
    snapshot: &BadgeExecutionSnapshot,
) -> (String, String) {
    let state = derive_badge_runtime_state(snapshot);
    let (label, color, emoji) = badge_state_label(state);
    let repo_name = escape_svg_text(&format!("{owner}/{repo}"));
    let status_text = escape_svg_text(&format!("{emoji} {label}"));
    let health = snapshot.health_score.clamp(0.0, 100.0);
    let left_width =
        BADGE_PADDING_WIDTH + (repo_name.chars().count() as i32 * BADGE_CHAR_WIDTH).max(BADGE_MIN_LEFT_WIDTH);
    let right_width =
        BADGE_PADDING_WIDTH + (status_text.chars().count() as i32 * BADGE_CHAR_WIDTH).max(BADGE_MIN_RIGHT_WIDTH);
    let total_width = left_width + right_width;

    (
        format!("/badge/{owner}/{repo}.svg"),
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_width}" height="20" role="img" aria-label="{repo_name}: {label}">
  <linearGradient id="badge-fill" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <rect rx="3" width="{total_width}" height="20" fill="#1f2937"/>
  <rect rx="3" x="{left_width}" width="{right_width}" height="20" fill="{color}"/>
  <path fill="{color}" d="M{left_width} 0h4v20h-4z"/>
  <rect rx="3" width="{total_width}" height="20" fill="url(#badge-fill)"/>
  <g fill="#fff" text-anchor="middle" font-family="DejaVu Sans,Verdana,Geneva,sans-serif" text-rendering="geometricPrecision" font-size="11">
    <text x="{left_text_x}" y="15" fill="#fff">{repo_name}</text>
    <text x="{right_text_x}" y="15" fill="#fff">{status_text}</text>
  </g>
  <title>{repo_name} - {label} ({health:.1}% health)</title>
</svg>"##,
            left_text_x = left_width / 2,
            right_text_x = left_width + (right_width / 2),
        ),
    )
}

pub fn healed_badge_svg_endpoint(owner: &str, repo: &str) -> (String, String) {
    badge_svg_endpoint(
        owner,
        repo,
        &BadgeExecutionSnapshot {
            health_score: 100.0,
            execution_readiness: 1.0,
            last_run_status: "success".to_string(),
            has_execution_history: true,
            healed_artifact_available: true,
        },
    )
}

pub fn badge_seed_launch_endpoint(
    owner: &str,
    repo: &str,
    branch: Option<&str>,
) -> (String, String) {
    let normalized_branch = branch.unwrap_or("main");
    let repo_url = format!("https://github.com/{owner}/{repo}");
    let (execution_path, execution_body) = executions_start_endpoint(&ExecutionStartRequest {
        org_id: None,
        user_id: None,
        anon_user_id: Some(format!("anon-seed-{}", &hash_key(&repo_url)[..12])),
        anon_session_id: Some(format!(
            "seed-{}",
            &hash_key(&format!("{repo_url}:{normalized_branch}"))[..12]
        )),
        device_fingerprint: Some("readme-badge-seed".to_string()),
        repo_url: repo_url.clone(),
        branch: Some(normalized_branch.to_string()),
        commit: None,
    });

    let execution_payload: Value = match serde_json::from_str(&execution_body) {
        Ok(payload) => payload,
        Err(error) => json!({
            "warning": format!("failed_to_parse_execution_payload: {error}"),
            "raw": execution_body
        }),
    };

    (
        format!("/seed/{owner}/{repo}"),
        json!({
            "entrypoint": "readme_badge",
            "repo": {
                "owner": owner,
                "name": repo,
                "url": repo_url,
                "branch": normalized_branch
            },
            "pipeline": {
                "analyze_endpoint": "/api/v1/repositories/analyze",
                "execution_plan_endpoint": "/api/v1/execution/plan",
                "execution_start_endpoint": execution_path,
                "execution_graph": "generated",
                "healing_enabled": true
            },
            "session": {
                "identity_type": "anonymous",
                "ownership_transfer": ["fork_pr_back", "user_adoption_fork", "hosted_variant"]
            },
            "execution": execution_payload
        })
        .to_string(),
    )
}

pub fn healed_badge_variant_endpoint(owner: &str, repo: &str) -> (String, String) {
    let (_, body) = healed_badge_svg_endpoint(owner, repo);
    (format!("/badge/healed/{owner}/{repo}.svg"), body)
}
