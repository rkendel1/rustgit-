use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use rustgit_wasm_runtime::{RuntimeError, WasmWorkspace, Workspace, WorkspaceManager};
use serde::Deserialize;
use serde_json::{json, Value};

type SharedManager = Arc<WorkspaceManager>;

#[derive(Deserialize)]
struct LaunchRequest {
    repo_url: String,
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

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);

    let root =
        std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| "/data/workspaces".to_string());
    let manager: SharedManager = Arc::new(WorkspaceManager::new(root));

    let app = Router::new()
        .route("/health", get(health))
        .route("/workspaces", post(launch_workspace))
        .route("/workspaces/:id", delete(stop_workspace))
        .route("/workspaces/:id/restart", post(restart_workspace))
        .route("/workspaces/:id/logs", get(workspace_logs))
        .with_state(manager);

    let addr = format!("0.0.0.0:{port}");
    println!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
