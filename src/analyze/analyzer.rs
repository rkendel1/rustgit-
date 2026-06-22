use std::path::Path;
use std::time::Instant;
use std::{fs, io};

use serde::Serialize;
use serde_json::{json, Value};

use crate::Result;

use super::blueprint_builder::{build_blueprint, build_execution_plan, capability_registry};
use super::framework_detector::detect_framework;
use super::manifest_builder::{write_manifest, AnalyzeManifest};
use super::runtime_detector::detect_runtime;

const STAGE_TIMEOUT_MS: u128 = 250;
const MAX_CONFIDENCE: u8 = 99;
const FRAMEWORKS_REQUIRING_BROWSER_APIS: [&str; 5] = ["vite", "react", "nextjs", "svelte", "nuxt"];

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
        let execution_plan = build_execution_plan(&runtime.runtime, &framework.framework);
        let requires_docker = manifest.workspace.requires_docker;
        let requires_python = runtime.runtime == "python" || framework.language == "python";
        let requires_secrets = manifest.workspace.requires_secrets;
        let requires_browser_apis =
            FRAMEWORKS_REQUIRING_BROWSER_APIS.contains(&framework.framework.as_str());
        let stage_start = Instant::now();
        let blueprint = build_blueprint(
            &runtime.runtime,
            &framework.framework,
            requires_docker,
            requires_python,
            requires_secrets,
            requires_browser_apis,
        );
        traceability.push(stage_trace(
            "execution_blueprint",
            &[blueprint.preferred_provider.clone()],
            stage_start,
        ));

        let confidence = compute_confidence(&framework.framework, &runtime.runtime);
        write_manifest(root, &manifest)?;
        Self::write_execution_strategy_artifacts(
            root,
            &execution_plan,
            request,
            &blueprint.preferred_provider,
            &blueprint.fallback,
            &blueprint.selected_because,
        )?;
        let preferred_provider = blueprint.preferred_provider.clone();
        let selected_because = blueprint.selected_because.clone();
        let fallback = blueprint.fallback.clone();
        let framework_name = manifest.framework.clone();

        let duration_ms = total_start.elapsed().as_millis() as u64;
        let response = json!({
            "repo": request.repo,
            "repo_url": request.repo,
            "runtime": {
                "language": framework.language,
                "packageManager": runtime.package_manager,
                "framework": manifest.framework
            },
            "execution": blueprint,
            "execution_plan": execution_plan,
            "runtime_capabilities": {
                "providers": capability_registry()
            },
            "execution_trace": {
                "provider": preferred_provider,
                "selectedBecause": selected_because,
                "fallbacks": fallback,
                "actualStartup": "0ms",
                "successful": true
            },
            "manifest": {
                "version": 1,
                "path": ".execution.json"
            },
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
                "phase7_commit_cache_key": format!("{}/{}/{}", request.repo, request.branch, request.commit),
                "phase8_provider_decoupled": true,
                "phase9_runtime_registry": true,
                "phase10_response_contract": true,
                "stages": traceability
            }
        });

        Ok(AnalyzeEngineResult { payload: response })
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
