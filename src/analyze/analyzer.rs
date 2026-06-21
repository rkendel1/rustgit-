use std::path::Path;
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use crate::Result;

use super::blueprint_builder::build_blueprint;
use super::framework_detector::detect_framework;
use super::manifest_builder::{write_manifest, AnalyzeManifest};
use super::runtime_detector::detect_runtime;

const STAGE_TIMEOUT_MS: u128 = 250;
const MAX_CONFIDENCE: u8 = 99;

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
        traceability.push(stage_trace("framework_detection", &framework.evidence, stage_start));

        let stage_start = Instant::now();
        let runtime = detect_runtime(root, Some(&framework.framework));
        traceability.push(stage_trace("runtime_detection", &runtime.evidence, stage_start));

        let stage_start = Instant::now();
        let blueprint = build_blueprint(&runtime.runtime);
        traceability.push(stage_trace(
            "execution_blueprint",
            &[blueprint.preferred_provider.clone()],
            stage_start,
        ));

        let confidence = compute_confidence(&framework.framework, &runtime.runtime);

        let manifest = AnalyzeManifest {
            runtime: runtime.runtime.clone(),
            framework: framework.framework.clone(),
            package_manager: runtime.package_manager.clone(),
            build: runtime.build.clone(),
            start: runtime.start.clone(),
            dev: runtime.dev.clone(),
            confidence,
        };
        write_manifest(root, &manifest)?;

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
            "manifest": {
                "version": 1,
                "path": ".ddockit/manifest.json"
            },
            "frameworks": [manifest.framework],
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
