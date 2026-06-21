use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use rustgit_wasm_runtime::{
    badge_generate_endpoint, badge_seed_launch_endpoint, badge_svg_endpoint,
    healed_badge_variant_endpoint, BadgeExecutionSnapshot, BadgeGenerateRequest, RuntimeError,
    WasmWorkspace, Workspace, WorkspaceManager,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};

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

#[derive(Deserialize)]
struct ExecutionRequest {
    owner: Option<String>,
    repo: Option<String>,
    repo_url: Option<String>,
    branch: Option<String>,
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

async fn launch_execution(
    State(manager): State<SharedManager>,
    Json(body): Json<ExecutionRequest>,
) -> Result<(StatusCode, Json<ExecutionResponse>), (StatusCode, Json<Value>)> {
    let repo_url = resolve_repo_url(body.repo_url, None, body.owner, body.repo)?;
    let _branch = body.branch;
    tokio::task::spawn_blocking(move || manager.launch(&repo_url))
        .await
        .expect("task panicked")
        .map(|workspace| {
            (
                StatusCode::CREATED,
                Json(ExecutionResponse {
                    execution_id: workspace.id.clone(),
                    workspace_url: format!("{}/workspaces/{}", base_url(), workspace.id),
                    status: format!("{:?}", workspace.state).to_lowercase(),
                }),
            )
        })
        .map_err(err_response)
}

async fn analyze_repository_compat(
    State(manager): State<SharedManager>,
    Json(body): Json<AnalyzeRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let repo_url = resolve_repo_url(body.repo_url, body.url, body.owner, body.repo)?;
    tokio::task::spawn_blocking(move || manager.launch(&repo_url))
        .await
        .expect("task panicked")
        .map(|workspace| {
            (
                StatusCode::OK,
                Json(json!({
                    "repo_url": workspace.repo_url,
                    "frameworks": [format!("{:?}", workspace.framework).to_lowercase()],
                    "services": [],
                })),
            )
        })
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

fn with_workspace_routes(router: Router<SharedManager>, prefix: &str) -> Router<SharedManager> {
    router
        .route(&format!("{prefix}/workspaces"), post(launch_workspace))
        .route(&format!("{prefix}/workspaces/:id"), delete(stop_workspace))
        .route(
            &format!("{prefix}/workspaces/:id/restart"),
            post(restart_workspace),
        )
        .route(&format!("{prefix}/workspaces/:id/logs"), get(workspace_logs))
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

fn app(manager: SharedManager) -> Router {
    with_workspace_routes(
        with_workspace_routes(
            with_workspace_routes(Router::<SharedManager>::new(), ""),
            "/api/v1",
        ),
        "/api/proxy/api/v1",
    )
        .route("/health", get(health))
        .route("/api/v1/executions", post(launch_execution))
        .route("/api/proxy/api/v1/executions", post(launch_execution))
        .route(
            "/api/v1/repositories/analyze",
            post(analyze_repository_compat),
        )
        .route(
            "/api/proxy/api/v1/repositories/analyze",
            post(analyze_repository_compat),
        )
        .route("/api/badges/generate", post(generate_badge))
        .route("/api/badge/generate", post(generate_badge))
        .route("/badge/:owner/:repo.svg", get(runtime_badge))
        .route("/badge/healed/:owner/:repo.svg", get(healed_badge))
        .route("/seed/:owner/:repo", get(seed_launch))
        .layer(cors_layer())
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
    use std::sync::Arc;

    use axum::{
        body::{to_bytes, Body},
        extract::Path,
        http::{header, Request, StatusCode},
        response::IntoResponse,
    };
    use tower::ServiceExt;

    use super::{app, runtime_badge, SharedManager, WorkspaceManager};

    fn test_manager() -> SharedManager {
        let root = std::env::temp_dir().join(format!(
            "rustgit-server-tests-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        Arc::new(WorkspaceManager::new(root))
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
        let app = app(test_manager());

        let checks = [
            ("/api/v1/workspaces/missing", "DELETE"),
            ("/api/v1/workspaces/missing/restart", "POST"),
            ("/api/v1/workspaces/missing/logs", "GET"),
            ("/api/proxy/api/v1/workspaces/missing", "DELETE"),
            ("/api/proxy/api/v1/workspaces/missing/restart", "POST"),
            ("/api/proxy/api/v1/workspaces/missing/logs", "GET"),
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
        let app = app(test_manager());

        let checks = [
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
    async fn cors_preflight_is_enabled_for_v1_routes() {
        let app = app(test_manager());
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
}
