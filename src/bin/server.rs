use std::collections::HashMap;
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::{
    extract::{Path, State},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use rustgit_wasm_runtime::{
    analyze::{AnalyzeCache, AnalyzeEngine, AnalyzeEngineRequest},
    badge_generate_endpoint, badge_seed_launch_endpoint, badge_svg_endpoint,
    healed_badge_variant_endpoint, BadgeExecutionSnapshot, BadgeGenerateRequest, LaunchOverrides,
    RuntimeError, WasmWorkspace, Workspace, WorkspaceManager,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

type SharedManager = Arc<WorkspaceManager>;
type FingerprintIndex = Arc<Mutex<HashMap<String, Value>>>;

#[derive(Clone)]
struct AppState {
    manager: SharedManager,
    analyze_cache: Arc<AnalyzeCache>,
    fingerprint_index: FingerprintIndex,
}

#[derive(Deserialize)]
struct LaunchRequest {
    repo_url: String,
}

#[derive(Deserialize)]
struct BadgeGeneratePayload {
    repo_url: String,
    branch: Option<String>,
    mode: Option<String>,
    visibility: Option<String>,
}

#[derive(Deserialize)]
struct ExecutionRequest {
    owner: Option<String>,
    repo: Option<String>,
    repo_url: Option<String>,
    branch: Option<String>,
    start_command: Option<String>,
    environment: Option<HashMap<String, String>>,
    versions: Option<HashMap<String, String>>,
}

impl ExecutionRequest {
    fn launch_overrides(&self) -> LaunchOverrides {
        let start_command = self
            .start_command
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let environment = self
            .environment
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        let versions = self
            .versions
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        let branch = self
            .branch
            .as_ref()
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty());
        LaunchOverrides {
            branch,
            start_command,
            environment,
            versions,
        }
    }
}

#[derive(Default, Deserialize)]
struct RestartRequest {
    branch: Option<String>,
    start_command: Option<String>,
    environment: Option<HashMap<String, String>>,
    versions: Option<HashMap<String, String>>,
}

impl RestartRequest {
    fn launch_overrides(self) -> LaunchOverrides {
        let start_command = self
            .start_command
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let environment = self
            .environment
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        let versions = self
            .versions
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key.trim().to_string(), value))
            .filter(|(key, _)| !key.is_empty())
            .collect();
        let branch = self
            .branch
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty());
        LaunchOverrides {
            branch,
            start_command,
            environment,
            versions,
        }
    }
}

#[derive(Serialize)]
struct ExecutionResponse {
    execution_id: String,
    workspace_url: String,
    status: String,
}

#[derive(Deserialize)]
struct AnalyzeRequest {
    owner: Option<String>,
    repo: Option<String>,
    url: Option<String>,
    repo_url: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
    include_repository_summary: Option<bool>,
    ask_question: Option<String>,
}

fn err_response(err: RuntimeError) -> (StatusCode, Json<Value>) {
    let status = match &err {
        RuntimeError::WorkspaceMissing(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": err.to_string() })))
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
}

fn base_url() -> String {
    std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

fn github_url(owner: &str, repo: &str) -> String {
    format!("https://github.com/{owner}/{repo}.git")
}

fn resolve_repo_url(
    repo_url: Option<String>,
    url: Option<String>,
    owner: Option<String>,
    repo: Option<String>,
) -> Result<String, (StatusCode, Json<Value>)> {
    if let Some(candidate) = repo_url.or(url).map(|value| value.trim().to_string()) {
        if !candidate.is_empty() {
            return Ok(candidate);
        }
    }
    match (owner, repo) {
        (Some(owner), Some(repo)) if !owner.trim().is_empty() && !repo.trim().is_empty() => {
            Ok(github_url(owner.trim(), repo.trim()))
        }
        _ => Err(bad_request(
            "missing repository reference; provide repo_url/url or owner and repo",
        )),
    }
}

fn prepare_repository_for_analysis(repo_url: &str, branch: &str, commit: &str) -> rustgit_wasm_runtime::Result<PathBuf> {
    if FsPath::new(repo_url).exists() {
        return Ok(PathBuf::from(repo_url));
    }

    let workspace = std::env::temp_dir()
        .join("rustgit-analyze")
        .join(hash_key(repo_url))
        .join(hash_key(&format!("{repo_url}:{branch}:{commit}")));
    if workspace.exists() {
        fs::remove_dir_all(&workspace)?;
    }
    fs::create_dir_all(&workspace)?;

    let build_clone_command = |with_branch: bool| -> Command {
        let mut clone = Command::new("git");
        clone
            .arg("-c")
            .arg("credential.helper=")
            .arg("-c")
            .arg("credential.username=")
            .arg("clone")
            .arg("--depth")
            .arg("1");
        if with_branch && !branch.is_empty() {
            clone.arg("--branch").arg(branch);
        }
        clone.arg(repo_url).arg(&workspace);
        clone.env("GIT_TERMINAL_PROMPT", "0");
        clone
    };

    let output = build_clone_command(true)
        .output()
        .map_err(|e| RuntimeError::CommandFailed(format!("git clone failed: {e}")))?;

    let output = if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let branch_not_found = stderr.contains("Remote branch") && stderr.contains("not found");
        if branch_not_found && !branch.is_empty() {
            // Branch doesn't exist on remote — retry without --branch to use the default branch
            if workspace.exists() {
                fs::remove_dir_all(&workspace)?;
            }
            fs::create_dir_all(&workspace)?;
            let fallback_output = build_clone_command(false)
                .output()
                .map_err(|e| RuntimeError::CommandFailed(format!("git clone failed: {e}")))?;
            fallback_output
        } else {
            return Err(RuntimeError::CommandFailed(format!(
                "git clone exited with status {}: {}",
                output.status,
                stderr.trim()
            )));
        }
    } else {
        output
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(RuntimeError::CommandFailed(format!(
            "git clone exited with status {}: {}",
            output.status, stderr
        )));
    }

    if !commit.is_empty() {
        let output = Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .arg("checkout")
            .arg(commit)
            .output()
            .map_err(|e| RuntimeError::CommandFailed(format!("git checkout failed: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(RuntimeError::CommandFailed(format!(
                "git checkout exited with status {}: {}",
                output.status, stderr
            )));
        }
    }

    Ok(workspace)
}

fn resolve_repository_commit(root: &FsPath) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!commit.is_empty()).then_some(commit)
}

fn hash_key(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn launch_workspace(
    State(state): State<AppState>,
    Json(body): Json<LaunchRequest>,
) -> Result<(StatusCode, Json<Workspace>), (StatusCode, Json<Value>)> {
    let repo_url = body.repo_url;
    let manager = state.manager;
    tokio::task::spawn_blocking(move || manager.launch(&repo_url))
        .await
        .expect("task panicked")
        .map(|ws| (StatusCode::CREATED, Json(ws)))
        .map_err(err_response)
}

async fn launch_execution(
    State(state): State<AppState>,
    Json(body): Json<ExecutionRequest>,
) -> Result<(StatusCode, Json<ExecutionResponse>), (StatusCode, Json<Value>)> {
    let overrides = body.launch_overrides();
    let repo_url = resolve_repo_url(body.repo_url.clone(), None, body.owner.clone(), body.repo.clone())?;
    let manager = state.manager;

    // Allocate the workspace ID synchronously (fast — just inserts a pending record)
    let id = manager.begin_launch_with_overrides(&repo_url, overrides.clone());
    let workspace_url = format!("{}/workspaces/{}", base_url(), id);

    // Do the heavy work (git clone, npm install, process spawn) in a background thread.
    // The UI polls /workspaces/:id for status updates while this runs.
    let id_bg = id.clone();
    tokio::task::spawn_blocking(move || {
        manager.complete_launch_with_overrides(&id_bg, &repo_url, overrides)
    });

    Ok((
        StatusCode::CREATED,
        Json(ExecutionResponse {
            execution_id: id,
            workspace_url,
            status: "created".to_string(),
        }),
    ))
}

async fn analyze_repository(
    State(state): State<AppState>,
    Json(body): Json<AnalyzeRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let started = Instant::now();
    let include_repository_summary = body.include_repository_summary.unwrap_or(false);
    let ask_question = body.ask_question.as_deref();
    let repo_url = resolve_repo_url(body.repo_url, body.url, body.owner, body.repo)?;
    let branch = body
        .branch
        .unwrap_or_else(|| "main".to_string())
        .trim()
        .to_string();
    let requested_commit = body.commit.unwrap_or_default().trim().to_string();
    let request_cache_commit = if requested_commit.is_empty() {
        "latest".to_string()
    } else {
        requested_commit.clone()
    };
    let request_cache_key = AnalyzeCache::key(&repo_url, &branch, &request_cache_commit);

    if let Some(cached) = state.analyze_cache.get(&request_cache_key) {
        let mut payload = cached.payload;
        payload["cached"] = json!(true);
        payload["durationMs"] = json!(started.elapsed().as_millis() as u64);
        if include_repository_summary {
            enrich_analyze_payload(&mut payload, ask_question);
        }
        if let Some(fingerprint_id) = payload.get("fingerprint_id").and_then(|v| v.as_str()) {
            if let Ok(mut idx) = state.fingerprint_index.lock() {
                idx.entry(fingerprint_id.to_string()).or_insert(payload.clone());
            }
        }
        return Ok((StatusCode::OK, Json(payload)));
    }

    let repo_root = prepare_repository_for_analysis(&repo_url, &branch, &requested_commit)
        .map_err(err_response)?;
    let resolved_commit = resolve_repository_commit(&repo_root).unwrap_or_else(|| {
        if requested_commit.is_empty() {
            "unknown".to_string()
        } else {
            requested_commit.clone()
        }
    });
    let resolved_cache_key = AnalyzeCache::key(&repo_url, &branch, &resolved_commit);

    if let Some(cached) = state.analyze_cache.get(&resolved_cache_key) {
        let mut payload = cached.payload;
        payload["cached"] = json!(true);
        payload["durationMs"] = json!(started.elapsed().as_millis() as u64);
        if include_repository_summary {
            enrich_analyze_payload(&mut payload, ask_question);
        }
        if let Some(fingerprint_id) = payload.get("fingerprint_id").and_then(|v| v.as_str()) {
            if let Ok(mut idx) = state.fingerprint_index.lock() {
                idx.entry(fingerprint_id.to_string()).or_insert(payload.clone());
            }
        }
        return Ok((StatusCode::OK, Json(payload)));
    }

    let request = AnalyzeEngineRequest {
        repo: repo_url.clone(),
        branch,
        commit: resolved_commit.clone(),
    };
    let result = AnalyzeEngine::analyze(&repo_root, &request).map_err(err_response)?;
    let mut payload = result.payload;
    payload["cached"] = json!(false);
    payload["durationMs"] = json!(started.elapsed().as_millis() as u64);
    if include_repository_summary {
        enrich_analyze_payload(&mut payload, ask_question);
    }
    state
        .analyze_cache
        .put(resolved_cache_key.clone(), payload.clone());
    if request_cache_key != resolved_cache_key {
        state.analyze_cache.put(request_cache_key, payload.clone());
    }
    if let Some(fingerprint_id) = payload.get("fingerprint_id").and_then(|v| v.as_str()) {
        if let Ok(mut idx) = state.fingerprint_index.lock() {
            idx.insert(fingerprint_id.to_string(), payload.clone());
        }
    }
    Ok((StatusCode::OK, Json(payload)))
}

fn repository_intelligence_from_payload(fingerprint_id: &str, payload: &Value) -> Value {
    let confidence = payload.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let health_score = confidence / 100.0;

    let preferred_provider = payload
        .get("execution")
        .and_then(|e| e.get("preferredProvider"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let execution_score = if preferred_provider.is_empty() || preferred_provider == "unknown" {
        0.0_f64
    } else {
        0.7
    };

    let runtime = payload
        .get("runtime")
        .and_then(|r| r.get("framework"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "unknown")
        .or_else(|| {
            payload
                .get("frameworks")
                .and_then(|f| f.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty() && *s != "unknown")
        })
        .map(|s| s.to_string());

    json!({
        "repository_id": fingerprint_id,
        "health_score": health_score,
        "execution_score": execution_score,
        "healing_score": null,
        "runtime": runtime,
        "repository_identity": null
    })
}

fn repository_ask_from_payload(payload: &Value, question: Option<&str>) -> Value {
    let repo_url = payload
        .get("repo_url")
        .or_else(|| payload.get("repo"))
        .and_then(|v| v.as_str())
        .unwrap_or("this repository");

    let frameworks: Vec<&str> = payload
        .get("frameworks")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| *s != "unknown")
                .collect()
        })
        .unwrap_or_default();

    let runtime_lang = payload
        .get("runtime")
        .and_then(|r| r.get("language"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "unknown");

    let runtime_framework = payload
        .get("runtime")
        .and_then(|r| r.get("framework"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "unknown");

    let preferred_provider = payload
        .get("execution")
        .and_then(|e| e.get("preferredProvider"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "unknown");

    let confidence = payload.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let answer_confidence = (confidence / 100.0).min(1.0) as f32;

    let question = question.unwrap_or("Summarize this repository.");

    let mut parts: Vec<String> = Vec::new();

    if let Some(fw) = runtime_framework {
        parts.push(format!("This repository uses the {} framework", fw));
    } else if !frameworks.is_empty() {
        parts.push(format!("This repository uses {}", frameworks.join(", ")));
    } else {
        parts.push(format!(
            "This repository at {} could not be automatically classified",
            repo_url
        ));
    }

    if let Some(lang) = runtime_lang {
        parts.push(format!("written in {}", lang));
    }

    parts.push(".".to_string());

    if let Some(provider) = preferred_provider {
        parts.push(format!(
            " The recommended way to run it is via {}.",
            provider
        ));
    }

    let answer = if question.to_lowercase().contains("run") || question.to_lowercase().contains("summar") {
        format!(
            "{} {}",
            parts.join(" ").trim(),
            preferred_provider
                .map(|p| format!("Run using: {p}"))
                .unwrap_or_else(|| "No specific run instructions could be determined automatically.".to_string())
        )
    } else {
        parts.join(" ").trim().to_string()
    };

    let mut evidence: Vec<&str> = Vec::new();
    if let Some(fw) = runtime_framework {
        evidence.push(fw);
    }
    if let Some(lang) = runtime_lang {
        evidence.push(lang);
    }
    if let Some(p) = preferred_provider {
        evidence.push(p);
    }

    json!({
        "answer": answer,
        "confidence": answer_confidence,
        "evidence": evidence
    })
}

fn enrich_analyze_payload(payload: &mut Value, question: Option<&str>) {
    if let Some(fingerprint_id) = payload.get("fingerprint_id").and_then(|v| v.as_str()) {
        payload["repository_intelligence"] =
            repository_intelligence_from_payload(fingerprint_id, payload);
        payload["repository_ask"] = repository_ask_from_payload(payload, question);
    }
}

async fn repository_intelligence(
    State(state): State<AppState>,
    Path(fingerprint_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let payload = state
        .fingerprint_index
        .lock()
        .ok()
        .and_then(|idx| idx.get(&fingerprint_id).cloned());

    let Some(payload) = payload else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "repository not found; run analyze first" })),
        ));
    };

    Ok(Json(repository_intelligence_from_payload(
        &fingerprint_id,
        &payload,
    )))
}

#[derive(Deserialize)]
struct AskRequest {
    question: Option<String>,
}

async fn repository_ask(
    State(state): State<AppState>,
    Path(fingerprint_id): Path<String>,
    Json(body): Json<AskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let payload = state
        .fingerprint_index
        .lock()
        .ok()
        .and_then(|idx| idx.get(&fingerprint_id).cloned());

    let Some(payload) = payload else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "repository not found; run analyze first" })),
        ));
    };

    Ok(Json(repository_ask_from_payload(
        &payload,
        body.question.as_deref(),
    )))
}

async fn stop_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let manager = state.manager;
    tokio::task::spawn_blocking(move || manager.stop(&id))
        .await
        .expect("task panicked")
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(err_response)
}

async fn restart_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<RestartRequest>>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let overrides = body.map(|Json(payload)| payload.launch_overrides());
    let manager = state.manager;
    tokio::task::spawn_blocking(move || manager.restart_with_overrides(&id, overrides))
        .await
        .expect("task panicked")
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(err_response)
}

async fn workspace_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let manager = state.manager;
    tokio::task::spawn_blocking(move || manager.logs(&id))
        .await
        .expect("task panicked")
        .map(|lines| Json(json!({ "logs": lines })))
        .map_err(err_response)
}

async fn workspace_files(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let manager = state.manager;
    tokio::task::spawn_blocking(move || {
        let fs = manager.filesystem(&id)?;
        let mut files = fs
            .list(usize::MAX)?
            .into_iter()
            .filter(|path| !is_workspace_internal_file(path))
            .collect::<Vec<_>>();
        files.sort_by(|left, right| {
            workspace_file_priority(left)
                .cmp(&workspace_file_priority(right))
                .then_with(|| left.cmp(right))
        });
        Ok::<_, RuntimeError>(Json(json!({ "files": files })))
    })
    .await
    .expect("task panicked")
    .map_err(err_response)
}

fn is_workspace_internal_file(path: &str) -> bool {
    path == ".git"
        || path.starts_with(".git/")
        || path_segments(path).any(|segment| segment == "node_modules")
}

fn workspace_file_priority(path: &str) -> (u8, u8) {
    let root_file = !path.contains('/');
    let Some(name) = FsPath::new(path).file_name().and_then(|value| value.to_str()) else {
        return (1, u8::MAX);
    };
    match (root_file, name.to_ascii_lowercase().as_str()) {
        (true, "package.json") => (0, 0),
        (true, "requirements.txt") => (0, 1),
        (true, "readme.md" | "readme") => (0, 2),
        (true, "pyproject.toml") => (0, 3),
        (true, "cargo.toml") => (0, 4),
        (true, "go.mod") => (0, 5),
        _ => (1, u8::MAX),
    }
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|segment| !segment.is_empty())
}

async fn workspace_file(
    State(state): State<AppState>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let manager = state.manager;
    tokio::task::spawn_blocking(move || {
        let fs = manager.filesystem(&id)?;
        let bytes = fs.read(path.trim_start_matches('/'))?;
        let content = String::from_utf8_lossy(&bytes).to_string();
        Ok::<_, RuntimeError>(Json(json!({
            "path": path,
            "content": content
        })))
    })
    .await
    .expect("task panicked")
    .map_err(err_response)
}

fn json_payload_response(body: String) -> (StatusCode, Json<Value>) {
    match serde_json::from_str::<Value>(&body) {
        Ok(payload) => (StatusCode::OK, Json(payload)),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("invalid_endpoint_payload: {error}") })),
        ),
    }
}

fn badge_svg_response(body: String) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        body,
    )
}

async fn generate_badge(Json(body): Json<BadgeGeneratePayload>) -> (StatusCode, Json<Value>) {
    let request = BadgeGenerateRequest {
        repo_url: body.repo_url,
        branch: body.branch,
        mode: body.mode,
        visibility: body.visibility,
    };
    let (_, payload) = badge_generate_endpoint(&request);
    let (status, Json(value)) = json_payload_response(payload);
    let final_status = if value.get("error").is_some() {
        StatusCode::BAD_REQUEST
    } else {
        status
    };
    (final_status, Json(value))
}

async fn runtime_badge(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
    let (_, body) = badge_svg_endpoint(
        &owner,
        &repo,
        &BadgeExecutionSnapshot {
            health_score: 0.0,
            execution_readiness: 0.0,
            last_run_status: "pending".to_string(),
            has_execution_history: false,
            healed_artifact_available: false,
        },
    );
    badge_svg_response(body)
}

async fn healed_badge(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
    let (_, body) = healed_badge_variant_endpoint(&owner, &repo);
    badge_svg_response(body)
}

async fn seed_launch(Path((owner, repo)): Path<(String, String)>) -> (StatusCode, Json<Value>) {
    let (_, payload) = badge_seed_launch_endpoint(&owner, &repo, None);
    json_payload_response(payload)
}

async fn list_workspaces(State(state): State<AppState>) -> Json<Vec<Workspace>> {
    Json(state.manager.list_workspaces())
}

async fn cleanup_disk(State(state): State<AppState>) -> Json<Value> {
    let manager = state.manager;
    let (evicted, free_bytes) = tokio::task::spawn_blocking(move || manager.cleanup())
        .await
        .expect("task panicked");
    Json(json!({
        "evicted_workspaces": evicted,
        "free_bytes": free_bytes,
        "free_gb": (free_bytes as f64) / (1024.0 * 1024.0 * 1024.0),
    }))
}

async fn get_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Workspace>, (StatusCode, Json<Value>)> {
    let manager = state.manager;
    tokio::task::spawn_blocking(move || {
        // Update state if the spawned process has exited unexpectedly
        manager.sync_process_health(&id);
        manager.get_workspace(&id)
    })
    .await
    .expect("task panicked")
    .map(Json)
    .map_err(err_response)
}

fn with_workspace_routes(router: Router<AppState>, prefix: &str) -> Router<AppState> {
    router
        .route(
            &format!("{prefix}/workspaces"),
            get(list_workspaces).post(launch_workspace),
        )
        .route(
            &format!("{prefix}/workspaces/:id"),
            get(get_workspace).delete(stop_workspace),
        )
        .route(
            &format!("{prefix}/workspaces/:id/restart"),
            post(restart_workspace),
        )
        .route(&format!("{prefix}/workspaces/:id/logs"), get(workspace_logs))
        .route(&format!("{prefix}/workspaces/:id/files"), get(workspace_files))
        .route(
            &format!("{prefix}/workspaces/:id/files/*path"),
            get(workspace_file),
        )
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(Any)
        .expose_headers([header::CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(60 * 60))
}

fn app(state: AppState) -> Router {
    with_workspace_routes(
        with_workspace_routes(
            with_workspace_routes(Router::<AppState>::new(), ""),
            "/api/v1",
        ),
        "/api/proxy/api/v1",
    )
        .route("/health", get(health))
        .route("/api/analyze", post(analyze_repository))
        .route("/api/v1/executions", post(launch_execution))
        .route("/api/proxy/api/v1/executions", post(launch_execution))
        .route(
            "/api/v1/repositories/analyze",
            post(analyze_repository),
        )
        .route(
            "/api/proxy/api/v1/repositories/analyze",
            post(analyze_repository),
        )
        .route("/api/cleanup", post(cleanup_disk))
        .route("/api/proxy/api/cleanup", post(cleanup_disk))
        .route("/api/badges/generate", post(generate_badge))
        .route("/api/badge/generate", post(generate_badge))
        .route("/badge/:owner/:repo.svg", get(runtime_badge))
        .route("/badge/healed/:owner/:repo.svg", get(healed_badge))
        .route("/seed/:owner/:repo", get(seed_launch))
        .route(
            "/api/repositories/:id/intelligence",
            get(repository_intelligence),
        )
        .route(
            "/api/proxy/api/repositories/:id/intelligence",
            get(repository_intelligence),
        )
        .route("/api/repositories/:id/ask", post(repository_ask))
        .route(
            "/api/proxy/api/repositories/:id/ask",
            post(repository_ask),
        )
        .layer(cors_layer())
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);

    let root = std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| {
        // In production (Fly.io) set WORKSPACE_ROOT=/data/workspaces via fly.toml [env].
        // Locally, fall back to a directory next to the binary so no root access is needed.
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(".workspace-data")
            .to_string_lossy()
            .to_string()
    });
    let manager: SharedManager = Arc::new(WorkspaceManager::new(root));
    let app = app(AppState {
        manager,
        analyze_cache: Arc::new(AnalyzeCache::default()),
        fingerprint_index: Arc::new(Mutex::new(HashMap::new())),
    });

    let addr = format!("0.0.0.0:{port}");
    println!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Mutex};

    use axum::{
        body::{to_bytes, Body},
        extract::Path,
        http::{header, Request, StatusCode},
        response::IntoResponse,
    };
    use tower::ServiceExt;

    use super::{app, runtime_badge, AnalyzeCache, AppState, WorkspaceManager};

    fn test_state() -> AppState {
        let root = std::env::temp_dir().join(format!(
            "rustgit-server-tests-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        AppState {
            manager: Arc::new(WorkspaceManager::new(root)),
            analyze_cache: Arc::new(AnalyzeCache::default()),
            fingerprint_index: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tokio::test]
    async fn badge_routes_return_svg_content() {
        let response = runtime_badge(Path(("vercel".to_string(), "next.js".to_string())))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/svg+xml; charset=utf-8")
        );
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read badge body");
        let body_text = String::from_utf8(body.to_vec()).expect("badge body utf8");
        assert!(body_text.contains("<svg"));
        assert!(body_text.contains("vercel/next.js"));
    }

    #[tokio::test]
    async fn mirrored_workspace_routes_are_available_under_v1_and_proxy_alias() {
        let app = app(test_state());

        let checks = [
            ("/api/v1/workspaces/missing", "DELETE"),
            ("/api/v1/workspaces/missing/restart", "POST"),
            ("/api/v1/workspaces/missing/logs", "GET"),
            ("/api/v1/workspaces/missing/files", "GET"),
            ("/api/v1/workspaces/missing/files/package.json", "GET"),
            ("/api/proxy/api/v1/workspaces/missing", "DELETE"),
            ("/api/proxy/api/v1/workspaces/missing/restart", "POST"),
            ("/api/proxy/api/v1/workspaces/missing/logs", "GET"),
            ("/api/proxy/api/v1/workspaces/missing/files", "GET"),
            ("/api/proxy/api/v1/workspaces/missing/files/package.json", "GET"),
        ];

        for (uri, method) in checks {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn execution_and_analyze_routes_exist_on_both_prefixes() {
        let app = app(test_state());

        let checks = [
            "/api/analyze",
            "/api/v1/executions",
            "/api/v1/repositories/analyze",
            "/api/proxy/api/v1/executions",
            "/api/proxy/api/v1/repositories/analyze",
        ];

        for uri in checks {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_ne!(response.status(), StatusCode::NOT_FOUND, "POST {uri}");
        }
    }

    #[tokio::test]
    async fn launch_execution_accepts_preheal_overrides() {
        let repo = std::env::temp_dir().join(format!(
            "rustgit-launch-repo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&repo).expect("create repo");
        fs::write(
            repo.join("package.json"),
            r#"{"scripts":{"dev":"node server.js"},"dependencies":{}}"#,
        )
        .expect("write package");
        fs::write(repo.join("server.js"), "setInterval(() => {}, 1000);\n").expect("write server");

        let app = app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/executions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "repo_url": repo.to_string_lossy().to_string(),
                            "start_command": "node server.js",
                            "environment": { "PORT": "4100" },
                            "versions": { "NODE_VERSION": "20" }
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn restart_workspace_accepts_overrides_payload() {
        let app = app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/workspaces/missing/restart")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "start_command": "npm run dev",
                            "environment": { "PORT": "3001" }
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn workspace_files_endpoints_return_file_list_and_content() {
        let repo = std::env::temp_dir().join(format!(
            "rustgit-files-repo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&repo).expect("create repo");
        fs::write(
            repo.join("package.json"),
            r#"{"scripts":{"dev":"node server.js"}}"#,
        )
        .expect("write package");
        fs::write(repo.join("server.js"), "setInterval(() => {}, 1000);\n").expect("write server");
        fs::create_dir_all(repo.join("src")).expect("create src");
        for index in 0..1_100 {
            fs::write(
                repo.join("src").join(format!("file-{index}.txt")),
                format!("file {index}\n"),
            )
            .expect("write source file");
        }

        let app = app(test_state());
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/workspaces")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "repo_url": repo.to_string_lossy().to_string()
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let body = to_bytes(create_response.into_body(), 1024 * 1024)
            .await
            .expect("workspace body");
        let created: serde_json::Value = serde_json::from_slice(&body).expect("workspace json");
        let id = created["id"].as_str().expect("workspace id");

        let files_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/workspaces/{id}/files"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let files_status = files_response.status();
        let files_body = to_bytes(files_response.into_body(), 1024 * 1024)
            .await
            .expect("files body");
        assert_eq!(
            files_status,
            StatusCode::OK,
            "files endpoint payload: {}",
            String::from_utf8_lossy(&files_body)
        );
        let files_payload: serde_json::Value =
            serde_json::from_slice(&files_body).expect("files payload");
        let files = files_payload["files"]
            .as_array()
            .expect("files array")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();
        assert!(files.iter().any(|path| *path == "package.json"));
        assert!(files.len() > 1000);
        assert!(files.iter().any(|path| *path == "src/file-1099.txt"));

        let file_response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/workspaces/{id}/files/package.json"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(file_response.status(), StatusCode::OK);
        let file_body = to_bytes(file_response.into_body(), 1024 * 1024)
            .await
            .expect("file body");
        let file_payload: serde_json::Value =
            serde_json::from_slice(&file_body).expect("file payload");
        assert!(file_payload["content"]
            .as_str()
            .expect("file content")
            .contains("\"scripts\""));
    }

    #[test]
    fn workspace_files_prioritize_useful_files_and_ignore_git_internal_entries() {
        let mut files = vec![
            ".git/config".to_string(),
            "src/main.rs".to_string(),
            "node_modules/vue/package.json".to_string(),
            "README.md".to_string(),
            "requirements.txt".to_string(),
            "app/package.json".to_string(),
            "package.json".to_string(),
            ".git/hooks/pre-commit.sample".to_string(),
        ];
        files.retain(|path| !super::is_workspace_internal_file(path));
        files.sort_by(|left, right| {
            super::workspace_file_priority(left)
                .cmp(&super::workspace_file_priority(right))
                .then_with(|| left.cmp(right))
        });
        assert_eq!(
            files,
            vec![
                "package.json",
                "requirements.txt",
                "README.md",
                "app/package.json",
                "src/main.rs",
            ]
        );
    }

    #[tokio::test]
    async fn cors_preflight_is_enabled_for_v1_routes() {
        let app = app(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/executions")
                    .header(header::ORIGIN, "chrome-extension://abc")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("*")
        );
    }

    #[tokio::test]
    async fn analyze_route_generates_manifest_and_uses_cache() {
        let repo = std::env::temp_dir().join(format!(
            "rustgit-analyze-repo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&repo).expect("create repo");
        fs::write(
            repo.join("package.json"),
            r#"{"dependencies":{"next":"15.0.0"},"scripts":{"dev":"next dev","build":"next build","start":"next start"}}"#,
        )
        .expect("write package");
        fs::write(repo.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'").expect("write lockfile");

        let app = app(test_state());
        let request_body = serde_json::json!({
            "repo_url": repo.to_string_lossy().to_string(),
            "branch": "main",
            "commit": "local"
        })
        .to_string();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/analyze")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.clone()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            payload["runtime"]["packageManager"].as_str(),
            Some("pnpm"),
            "phase2 lockfile runtime detection should select pnpm"
        );
        assert_eq!(
            payload["traceability"]["phase3_manifest_first"].as_bool(),
            Some(true)
        );
        assert!(
            payload.get("repository_intelligence").is_none(),
            "analyze should only include repository summary projections when explicitly requested"
        );
        assert!(
            payload.get("repository_ask").is_none(),
            "analyze should only include repository summary projections when explicitly requested"
        );
        assert!(repo.join(".ddockit/manifest.json").exists());

        let second = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/analyze")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body))
                    .expect("request"),
            )
            .await
            .expect("response");
        let second_body = to_bytes(second.into_body(), 1024 * 1024)
            .await
            .expect("second body");
        let second_payload: serde_json::Value =
            serde_json::from_slice(&second_body).expect("second json");
        assert_eq!(second_payload["cached"].as_bool(), Some(true));
        assert!(second_payload.get("repository_intelligence").is_none());
        assert!(second_payload.get("repository_ask").is_none());
    }

    #[tokio::test]
    async fn analyze_route_uses_latest_alias_cache_without_preparing_repository() {
        let state = test_state();
        let repo_url = "https://github.com/example/slow-repo.git";
        let cache_key = super::AnalyzeCache::key(repo_url, "main", "latest");
        let fingerprint_id = "repo-fingerprint-123";
        state.analyze_cache.put(
            cache_key,
            serde_json::json!({
                "repo": repo_url,
                "fingerprint_id": fingerprint_id,
                "runtime": { "language": "typescript", "packageManager": "pnpm", "framework": "nextjs" },
                "execution": { "preferredProvider": "browser-wasm", "fallback": ["fly", "docker", "codespaces"] },
                "manifest": { "version": 1, "path": ".ddockit/manifest.json" },
                "confidence": 98,
                "cached": false,
                "durationMs": 0
            }),
        );
        let app = app(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/analyze")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "repo_url": repo_url,
                            "branch": "main",
                            "include_repository_summary": true,
                            "ask_question": "Summarize what this repository does and the best way to run it."
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["cached"].as_bool(), Some(true));
        assert_eq!(payload["repo"].as_str(), Some(repo_url));
        assert_eq!(
            payload["repository_intelligence"]["repository_id"].as_str(),
            Some(fingerprint_id)
        );
        assert_eq!(
            payload["repository_ask"]["evidence"][0].as_str(),
            Some("nextjs")
        );

        let intelligence_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/repositories/{fingerprint_id}/intelligence"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(intelligence_response.status(), StatusCode::OK);
        let intelligence_body = to_bytes(intelligence_response.into_body(), 1024 * 1024)
            .await
            .expect("intelligence body");
        let intelligence_payload: serde_json::Value =
            serde_json::from_slice(&intelligence_body).expect("intelligence payload");
        assert_eq!(payload["repository_intelligence"], intelligence_payload);

        let ask_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/repositories/{fingerprint_id}/ask"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "question": "Summarize what this repository does and the best way to run it."
                        })
                        .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(ask_response.status(), StatusCode::OK);
        let ask_body = to_bytes(ask_response.into_body(), 1024 * 1024)
            .await
            .expect("ask body");
        let ask_payload: serde_json::Value = serde_json::from_slice(&ask_body).expect("ask payload");
        assert_eq!(payload["repository_ask"], ask_payload);
    }
}
