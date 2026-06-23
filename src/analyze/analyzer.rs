use std::path::Path;
use std::time::Instant;
use std::{fs, io};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::Result;

use super::blueprint_builder::{build_blueprint, build_execution_plan, capability_registry};
use super::framework_detector::detect_framework;
use super::manifest_builder::{write_manifest, AnalyzeManifest};
use super::runtime_detector::detect_runtime;

const STAGE_TIMEOUT_MS: u128 = 250;
const MAX_CONFIDENCE: u8 = 99;
const FRAMEWORKS_REQUIRING_BROWSER_APIS: [&str; 5] = ["vite", "react", "nextjs", "svelte", "nuxt"];
pub const ANALYSIS_VERSION: u8 = 3;

#[derive(Debug, Clone)]
pub struct AnalyzeEngineRequest {
    pub repo: String,
    pub branch: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalyzeEngineResult {
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeManifestV2 {
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    project: RuntimeManifestProject,
    runtime: RuntimeManifestRuntime,
    network: RuntimeManifestNetwork,
    providers: RuntimeManifestProviders,
    confidence: RuntimeManifestConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeManifestProject {
    framework: String,
    language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeManifestRuntime {
    #[serde(rename = "nodeVersion")]
    node_version: Option<String>,
    #[serde(rename = "packageManager")]
    package_manager: String,
    #[serde(rename = "installCommand")]
    install_command: String,
    #[serde(rename = "startCommand")]
    start_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeManifestNetwork {
    #[serde(rename = "preferredPorts")]
    preferred_ports: Vec<u16>,
    #[serde(rename = "healthCheck")]
    health_check: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeManifestProviders {
    compatible: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RuntimeManifestConfidence {
    overall: u8,
    framework: u8,
    runtime: u8,
    commands: u8,
    network: u8,
    providers: u8,
}

pub struct AnalyzeEngine;

impl AnalyzeEngine {
    pub fn analyze(root: &Path, request: &AnalyzeEngineRequest) -> Result<AnalyzeEngineResult> {
        let total_start = Instant::now();
        let mut traceability = Vec::new();

        let stage_start = Instant::now();
        let framework = detect_framework(root);
        traceability.push(stage_trace(
            "framework_detection",
            &framework.evidence,
            stage_start,
        ));

        let stage_start = Instant::now();
        let runtime = detect_runtime(root, Some(&framework.framework));
        traceability.push(stage_trace(
            "runtime_detection",
            &runtime.evidence,
            stage_start,
        ));

        let manifest = AnalyzeManifest::synthesize(
            root,
            &framework.framework,
            &runtime.runtime,
            runtime.package_manager.as_deref(),
            runtime.build.as_deref(),
            runtime.start.as_deref(),
            runtime.dev.as_deref(),
            compute_confidence(&framework.framework, &runtime.runtime),
        );
        let stage_start = Instant::now();
        traceability.push(stage_trace(
            "manifest_synthesis",
            &[manifest.framework.clone()],
            stage_start,
        ));

        let confidence = compute_confidence(&framework.framework, &runtime.runtime);
        write_manifest(root, &manifest)?;
        let runtime_manifest = build_runtime_manifest(
            &manifest,
            &framework.framework,
            &framework.language,
            &runtime.runtime,
            runtime.package_manager.as_deref(),
            runtime.dev.as_deref(),
            confidence,
        );
        validate_runtime_manifest(&runtime_manifest)?;
        Self::write_json_file(root.join("runtime-manifest.json"), &runtime_manifest)?;
        let framework_name = manifest.framework.clone();

        let duration_ms = total_start.elapsed().as_millis() as u64;
        let response = json!({
            "status": "ready",
            "analysis_version": ANALYSIS_VERSION,
            "background_processing": true,
            "repo": request.repo,
            "repo_url": request.repo,
            "runtime": {
                "language": framework.language,
                "packageManager": runtime.package_manager,
                "framework": manifest.framework
            },
            "execution": {
                "provider": "local"
            },
            "manifest": {
                "version": 2,
                "path": "runtime-manifest.json"
            },
            "runtime_manifest": runtime_manifest,
            "execution_intelligence": manifest,
            "frameworks": [framework_name],
            "services": ["root"],
            "fingerprint_id": request.commit,
            "confidence": confidence,
            "cached": false,
            "durationMs": duration_ms,
            "traceability": {
                "phase1_single_endpoint": true,
                "phase2_file_detection": true,
                "phase3_manifest_first": true,
                "phase4_staged_analyzer": true,
                "phase5_workspace_independent": true,
                "phase6_progressive_enhancement_non_blocking": true,
                "phase7_commit_cache_key": true,
                "phase8_provider_decoupled": true,
                "phase9_runtime_registry": true,
                "phase10_response_contract": true,
                "stages": traceability
            }
        });

        Ok(AnalyzeEngineResult { payload: response })
    }

    pub fn analyze_background(root: &Path, request: &AnalyzeEngineRequest) -> Result<()> {
        let framework = detect_framework(root);
        let runtime = detect_runtime(root, Some(&framework.framework));
        let execution_intelligence = AnalyzeManifest::synthesize(
            root,
            &framework.framework,
            &runtime.runtime,
            runtime.package_manager.as_deref(),
            runtime.build.as_deref(),
            runtime.start.as_deref(),
            runtime.dev.as_deref(),
            compute_confidence(&framework.framework, &runtime.runtime),
        );
        let execution_plan = build_execution_plan(&runtime.runtime, &framework.framework);
        let requires_docker = execution_intelligence.workspace.requires_docker;
        let requires_python = runtime.runtime == "python" || framework.language == "python";
        let requires_secrets = execution_intelligence.workspace.requires_secrets;
        let requires_browser_apis =
            FRAMEWORKS_REQUIRING_BROWSER_APIS.contains(&framework.framework.as_str());
        let blueprint = build_blueprint(
            &runtime.runtime,
            &framework.framework,
            requires_docker,
            requires_python,
            requires_secrets,
            requires_browser_apis,
        );
        Self::write_execution_strategy_artifacts(
            root,
            &execution_plan,
            request,
            &blueprint.preferred_provider,
            &blueprint.fallback,
            &blueprint.selected_because,
        )?;
        Self::write_json_file(
            root.join("execution-intelligence.json"),
            &execution_intelligence,
        )?;
        let summary = json!({
            "repo": request.repo,
            "branch": request.branch,
            "commit": request.commit,
            "framework": framework.framework,
            "language": framework.language,
            "runtime": runtime.runtime
        });
        Self::write_json_file(root.join("repository-summary.json"), &summary)?;
        Ok(())
    }

    fn write_execution_strategy_artifacts(
        root: &Path,
        execution_plan: &super::blueprint_builder::ExecutionPlan,
        request: &AnalyzeEngineRequest,
        selected_provider: &str,
        fallbacks: &[String],
        selected_because: &[String],
    ) -> Result<()> {
        let launch_plan = json!({
            "repo": request.repo,
            "branch": request.branch,
            "provider": selected_provider,
            "fallbacks": fallbacks,
            "selectedBecause": selected_because
        });
        let capabilities = json!({
            "providers": capability_registry().iter().map(|provider| json!({
                "name": provider.name,
                "enabled": provider.enabled,
                "healthy": provider.healthy
            })).collect::<Vec<_>>()
        });
        Self::write_json_file(root.join(".execution-plan.json"), execution_plan)?;
        Self::write_json_file(root.join(".runtime-capabilities.json"), &capabilities)?;
        Self::write_json_file(root.join(".launch-plan.json"), &launch_plan)?;
        Ok(())
    }

    fn write_json_file(path: impl AsRef<Path>, value: &impl Serialize) -> Result<()> {
        let path = path.as_ref();
        let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
            crate::RuntimeError::CommandFailed(format!("json serialization failed: {err}"))
        })?;
        fs::write(path, bytes).map_err(|err| match err.kind() {
            io::ErrorKind::PermissionDenied => crate::RuntimeError::CommandFailed(format!(
                "permission denied writing {}",
                path.display()
            )),
            _ => err.into(),
        })?;
        Ok(())
    }
}

fn stage_trace(name: &str, evidence: &[String], started: Instant) -> Value {
    let elapsed_ms = started.elapsed().as_millis();
    json!({
        "stage": name,
        "durationMs": elapsed_ms,
        "timeoutMs": STAGE_TIMEOUT_MS,
        "withinTimeout": elapsed_ms <= STAGE_TIMEOUT_MS,
        "evidence": evidence
    })
}

fn compute_confidence(framework: &str, runtime: &str) -> u8 {
    let mut score: u8 = 60;
    if framework != "unknown" {
        score = score.saturating_add(20);
    }
    if runtime != "unknown" {
        score = score.saturating_add(20);
    }
    score.min(MAX_CONFIDENCE)
}

fn build_runtime_manifest(
    manifest: &AnalyzeManifest,
    framework: &str,
    language: &str,
    runtime: &str,
    runtime_package_manager: Option<&str>,
    runtime_dev_command: Option<&str>,
    confidence: u8,
) -> RuntimeManifestV2 {
    let package_manager = manifest
        .package_manager
        .as_deref()
        .or(runtime_package_manager)
        .unwrap_or(runtime)
        .to_string();
    let start_command = manifest
        .start_command
        .as_deref()
        .or(runtime_dev_command)
        .unwrap_or("npm run dev")
        .to_string();
    let install_command = install_command_for_package_manager(&package_manager);
    let mut compatible = vec!["local".to_string(), "cloud".to_string()];
    let should_include_docker = manifest.workspace.requires_docker;
    if should_include_docker {
        compatible.push("docker".to_string());
    }
    compatible.sort();
    compatible.dedup();

    RuntimeManifestV2 {
        schema_version: 2,
        project: RuntimeManifestProject {
            framework: framework.to_string(),
            language: language.to_string(),
        },
        runtime: RuntimeManifestRuntime {
            node_version: manifest.node_version.clone(),
            package_manager: package_manager.clone(),
            install_command,
            start_command,
        },
        network: RuntimeManifestNetwork {
            preferred_ports: preferred_ports_for_framework(framework, runtime),
            health_check: health_check_for_framework(framework),
        },
        providers: RuntimeManifestProviders { compatible },
        confidence: RuntimeManifestConfidence {
            overall: confidence,
            framework: if framework == "unknown" { 50 } else { 95 },
            runtime: if runtime == "unknown" { 50 } else { 95 },
            commands: if manifest.start_command.is_some() { 95 } else { 70 },
            network: 90,
            providers: 90,
        },
    }
}

fn validate_runtime_manifest(manifest: &RuntimeManifestV2) -> Result<()> {
    if manifest.schema_version != 2 {
        return Err(crate::RuntimeError::CommandFailed(
            "runtime manifest schemaVersion must be 2".to_string(),
        ));
    }
    if manifest.project.framework.trim().is_empty() || manifest.project.language.trim().is_empty() {
        return Err(crate::RuntimeError::CommandFailed(
            "runtime manifest project metadata must be non-empty".to_string(),
        ));
    }
    if manifest.runtime.package_manager.trim().is_empty()
        || manifest.runtime.install_command.trim().is_empty()
        || manifest.runtime.start_command.trim().is_empty()
    {
        return Err(crate::RuntimeError::CommandFailed(
            "runtime manifest runtime commands must be non-empty".to_string(),
        ));
    }
    if manifest.network.preferred_ports.is_empty()
        || manifest
            .network
            .preferred_ports
            .iter()
            .any(|port| *port == 0)
    {
        return Err(crate::RuntimeError::CommandFailed(
            "runtime manifest preferredPorts must contain valid ports".to_string(),
        ));
    }
    if !manifest.network.health_check.starts_with('/') {
        return Err(crate::RuntimeError::CommandFailed(
            "runtime manifest healthCheck must start with '/'".to_string(),
        ));
    }
    if manifest.providers.compatible.is_empty() {
        return Err(crate::RuntimeError::CommandFailed(
            "runtime manifest provider compatibility must be non-empty".to_string(),
        ));
    }
    Ok(())
}

fn install_command_for_package_manager(package_manager: &str) -> String {
    match package_manager {
        "pnpm" => "pnpm install".to_string(),
        "yarn" => "yarn install".to_string(),
        "bun" => "bun install".to_string(),
        "pip" => "pip install -r requirements.txt".to_string(),
        "maven" => "mvn -q -DskipTests package".to_string(),
        "cargo" => "cargo fetch".to_string(),
        _ => "npm install".to_string(),
    }
}

fn preferred_ports_for_framework(framework: &str, runtime: &str) -> Vec<u16> {
    match framework {
        "vite" | "svelte" => vec![5173, 3000],
        "nextjs" | "react" | "express" => vec![3000],
        "fastapi" | "django" => vec![8000],
        _ if runtime == "python" => vec![8000],
        _ => vec![3000],
    }
}

fn health_check_for_framework(framework: &str) -> String {
    if framework == "fastapi" || framework == "django" || framework == "express" {
        "/health".to_string()
    } else {
        "/".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::Value;

    use super::{AnalyzeEngine, AnalyzeEngineRequest};

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{ts}"))
    }

    fn write_files(root: &std::path::Path, files: &[(&str, &str)]) {
        for (rel_path, content) in files {
            let path = root.join(rel_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(path, content).expect("write fixture file");
        }
    }

    fn assert_runtime_manifest_snapshot(name: &str, files: &[(&str, &str)], expected: Value) {
        let root = unique_temp_dir(&format!("analyze-runtime-manifest-{name}"));
        fs::create_dir_all(&root).expect("create root");
        write_files(&root, files);
        let request = AnalyzeEngineRequest {
            repo: root.to_string_lossy().to_string(),
            branch: "main".to_string(),
            commit: "local".to_string(),
        };
        AnalyzeEngine::analyze(&root, &request).expect("analyze");
        let payload = fs::read_to_string(root.join("runtime-manifest.json"))
            .expect("read runtime-manifest.json");
        let actual = serde_json::from_str::<Value>(&payload).expect("parse runtime-manifest.json");
        assert_eq!(actual, expected, "snapshot mismatch for {name}");
    }

    #[test]
    fn runtime_manifest_snapshots_cover_primary_frameworks_and_package_managers() {
        assert_runtime_manifest_snapshot(
            "nextjs-pnpm",
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"next":"15.0.0","typescript":"5.0.0"},"scripts":{"dev":"next dev"}} "#,
                ),
                ("pnpm-lock.yaml", "lockfileVersion: '9.0'"),
                (".nvmrc", "22"),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"nextjs","language":"typescript"},
              "runtime": {"nodeVersion":"22","packageManager":"pnpm","installCommand":"pnpm install","startCommand":"pnpm run start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[3000],"healthCheck":"/"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "react-npm",
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"react":"19.0.0"},"scripts":{"dev":"react-scripts start"}} "#,
                ),
                ("package-lock.json", "{}"),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"react","language":"javascript"},
              "runtime": {"nodeVersion":null,"packageManager":"npm","installCommand":"npm install","startCommand":"npm run start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[3000],"healthCheck":"/"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "vite-yarn",
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"vite":"5.0.0"},"scripts":{"dev":"vite"}} "#,
                ),
                ("yarn.lock", ""),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"vite","language":"javascript"},
              "runtime": {"nodeVersion":null,"packageManager":"yarn","installCommand":"yarn install","startCommand":"yarn start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[5173,3000],"healthCheck":"/"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "svelte-npm",
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"svelte":"5.0.0"},"scripts":{"dev":"svelte-kit dev"}} "#,
                ),
                ("package-lock.json", "{}"),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"svelte","language":"javascript"},
              "runtime": {"nodeVersion":null,"packageManager":"npm","installCommand":"npm install","startCommand":"npm run start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[5173,3000],"healthCheck":"/"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "express-npm",
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"express":"5.0.0"},"scripts":{"dev":"node server.js"}} "#,
                ),
                ("package-lock.json", "{}"),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"express","language":"javascript"},
              "runtime": {"nodeVersion":null,"packageManager":"npm","installCommand":"npm install","startCommand":"npm run start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[3000],"healthCheck":"/health"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "fastapi",
            &[("requirements.txt", "fastapi==0.111.0\nuvicorn==0.30.0"), ("main.py", "from fastapi import FastAPI\napp=FastAPI()\n")],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"fastapi","language":"python"},
              "runtime": {"nodeVersion":null,"packageManager":"pip","installCommand":"pip install -r requirements.txt","startCommand":"python main.py"},
              "network": {"preferredPorts":[8000],"healthCheck":"/health"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "django",
            &[("requirements.txt", "django==5.0.0"), ("manage.py", "print('django')")],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"django","language":"python"},
              "runtime": {"nodeVersion":null,"packageManager":"pip","installCommand":"pip install -r requirements.txt","startCommand":"python main.py"},
              "network": {"preferredPorts":[8000],"healthCheck":"/health"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "monorepo",
            &[
                ("package.json", r#"{"private":true,"workspaces":["apps/*"],"dependencies":{"next":"15.0.0"},"scripts":{"dev":"next dev"}}"#),
                ("pnpm-lock.yaml", "lockfileVersion: '9.0'"),
                ("apps/web/package.json", r#"{"dependencies":{"react":"19.0.0"}}"#),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"nextjs","language":"javascript"},
              "runtime": {"nodeVersion":null,"packageManager":"pnpm","installCommand":"pnpm install","startCommand":"pnpm run start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[3000],"healthCheck":"/"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
        assert_runtime_manifest_snapshot(
            "bun",
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"vite":"5.0.0"},"scripts":{"dev":"vite"}} "#,
                ),
                ("bun.lockb", "bun"),
            ],
            serde_json::json!({
              "schemaVersion": 2,
              "project": {"framework":"vite","language":"javascript"},
              "runtime": {"nodeVersion":null,"packageManager":"bun","installCommand":"bun install","startCommand":"bun run start --host 0.0.0.0 --port {PORT}"},
              "network": {"preferredPorts":[5173,3000],"healthCheck":"/"},
              "providers": {"compatible":["cloud","local"]},
              "confidence": {"overall":99,"framework":95,"runtime":95,"commands":95,"network":90,"providers":90}
            }),
        );
    }

    #[test]
    fn runtime_manifest_is_deterministic_for_same_repository() {
        let root = unique_temp_dir("analyze-runtime-manifest-deterministic");
        fs::create_dir_all(&root).expect("create root");
        write_files(
            &root,
            &[
                (
                    "package.json",
                    r#"{"dependencies":{"next":"15.0.0"},"scripts":{"dev":"next dev"}} "#,
                ),
                ("pnpm-lock.yaml", "lockfileVersion: '9.0'"),
            ],
        );
        let request = AnalyzeEngineRequest {
            repo: root.to_string_lossy().to_string(),
            branch: "main".to_string(),
            commit: "local".to_string(),
        };
        AnalyzeEngine::analyze(&root, &request).expect("first analyze");
        let first = fs::read_to_string(root.join("runtime-manifest.json")).expect("first manifest");
        AnalyzeEngine::analyze(&root, &request).expect("second analyze");
        let second =
            fs::read_to_string(root.join("runtime-manifest.json")).expect("second manifest");
        assert_eq!(first, second);
    }
}
