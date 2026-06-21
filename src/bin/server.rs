use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use rustgit_wasm_runtime::{
    badge_generate_endpoint, badge_seed_launch_endpoint, badge_svg_endpoint,
    healed_badge_variant_endpoint, BadgeExecutionSnapshot, BadgeGenerateRequest, RuntimeError,
    WasmWorkspace, Workspace, WorkspaceManager,
};
use serde::Deserialize;
use serde_json::{json, Value};

type SharedManager = Arc<WorkspaceManager>;

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

fn err_response(err: RuntimeError) -> (StatusCode, Json<Value>) {
    let status = match &err {
        RuntimeError::WorkspaceMissing(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": err.to_string() })))
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn launch_workspace(
    State(manager): State<SharedManager>,
    Json(body): Json<LaunchRequest>,
) -> Result<(StatusCode, Json<Workspace>), (StatusCode, Json<Value>)> {
    let repo_url = body.repo_url;
    tokio::task::spawn_blocking(move || manager.launch(&repo_url))
        .await
        .expect("task panicked")
        .map(|ws| (StatusCode::CREATED, Json(ws)))
        .map_err(err_response)
}

async fn stop_workspace(
    State(manager): State<SharedManager>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || manager.stop(&id))
        .await
        .expect("task panicked")
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(err_response)
}

async fn restart_workspace(
    State(manager): State<SharedManager>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || manager.restart(&id))
        .await
        .expect("task panicked")
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(err_response)
}

async fn workspace_logs(
    State(manager): State<SharedManager>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    tokio::task::spawn_blocking(move || manager.logs(&id))
        .await
        .expect("task panicked")
        .map(|lines| Json(json!({ "logs": lines })))
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

fn app(manager: SharedManager) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/workspaces", post(launch_workspace))
        .route("/workspaces/:id", delete(stop_workspace))
        .route("/workspaces/:id/restart", post(restart_workspace))
        .route("/workspaces/:id/logs", get(workspace_logs))
        .route("/api/badges/generate", post(generate_badge))
        .route("/api/badge/generate", post(generate_badge))
        .route("/badge/:owner/:repo.svg", get(runtime_badge))
        .route("/badge/healed/:owner/:repo.svg", get(healed_badge))
        .route("/seed/:owner/:repo", get(seed_launch))
        .with_state(manager)
}

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);

    let root = std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| "/data/workspaces".to_string());
    let manager: SharedManager = Arc::new(WorkspaceManager::new(root));

    let app = app(manager);

    let addr = format!("0.0.0.0:{port}");
    println!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use axum::{
        body::to_bytes,
        extract::Path,
        http::{header, StatusCode},
        response::IntoResponse,
    };

    use super::runtime_badge;

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
}
