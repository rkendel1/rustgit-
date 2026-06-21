use super::SoftwareExecutionSpec;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeterministicExecutionArtifacts {
    pub execution_lock: String,
    pub runtime_graph_json: Value,
    pub capabilities_toml: String,
    pub environment_schema_json: Value,
    pub provenance_json: Value,
    pub healing_patch: String,
    pub execution_fingerprint: String,
}

pub fn portable_execution_toml(spec: &SoftwareExecutionSpec) -> String {
    let package_manager = spec.runtime.package_manager.as_deref().unwrap_or("unknown");
    let services = spec
        .services
        .iter()
        .map(|service| format!("{} = \"{}\"", service.name, service.mode))
        .collect::<Vec<_>>()
        .join("\n");
    let environment = spec
        .environment
        .iter()
        .map(|(name, source)| format!("{name} = \"{source}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| format!("\"{capability}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"version = "{version}"
[runtime]
language = "{language}"
framework = "{framework}"
package_manager = "{package_manager}"
[services]
{services}
[environment]
{environment}
[capabilities]
all = [{capabilities}]
[healing]
apply_known_repairs = true
[confidence]
expected_success = {expected_success}"#,
        version = spec.identity.version,
        language = spec.runtime.language,
        framework = spec.runtime.framework,
        package_manager = package_manager,
        services = services,
        environment = environment,
        capabilities = capabilities,
        expected_success = format!("{:.3}", spec.confidence as f32 / 100.0)
    )
}

pub fn deterministic_execution_artifacts(
    spec: &SoftwareExecutionSpec,
    repository_hash: &str,
    runtime_component_graph: &[String],
    runtime_environment_id: &str,
) -> DeterministicExecutionArtifacts {
    let mut capabilities = spec.capabilities.clone();
    capabilities.sort();
    capabilities.dedup();
    let capabilities_toml = format!(
        "[capabilities]\nall = [{}]",
        capabilities
            .iter()
            .map(|capability| format!("\"{capability}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut component_graph = runtime_component_graph.to_vec();
    if component_graph.is_empty() {
        component_graph = vec!["runtime".to_string(), "executor".to_string()];
    }
    let runtime_graph_json = json!({
        "format": "ddockit.runtime.graph.v1",
        "nodes": component_graph,
    });

    let required_environment = spec.environment.keys().cloned().collect::<Vec<_>>();
    let environment_schema_json = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": required_environment,
        "properties": spec.environment.iter().map(|(name, source)| {
            (
                name.clone(),
                json!({
                    "type": "string",
                    "source": source
                }),
            )
        }).collect::<serde_json::Map<String, Value>>(),
    });

    let healing_patch = if spec.healing_plan.is_empty() {
        "# no healing mutations were required\n".to_string()
    } else {
        spec.healing_plan
            .iter()
            .map(|step| format!("# repair step\n+ {step}\n"))
            .collect::<Vec<_>>()
            .join("")
    };

    let provenance_json = json!({
        "format": "ddockit.provenance.v1",
        "execution_specification_id": spec.identity.spec_id,
        "environment": spec.environment.iter().map(|(name, source)| json!({
            "name": name,
            "source": source,
            "generated_by": "Environment Synthesizer",
            "confidence": f64::from(spec.confidence),
            "validated": "derived_at_analysis_time",
            "execution_successes": 0
        })).collect::<Vec<_>>(),
        "healing": [{
            "repair": "Updated runtime execution configuration",
            "reason": "Auto-generated from runtime recovery plan",
            "evidence_successes": 0,
            "confidence": f64::from(spec.confidence),
            "rollback_available": true
        }],
    });

    let execution_spec_hash = digest_label(&portable_execution_toml(spec));
    let capability_hash = digest_label(&capabilities_toml);
    let environment_schema_hash = digest_label(&environment_schema_json.to_string());
    let component_graph_hash = digest_label(&runtime_graph_json.to_string());
    let execution_fingerprint = digest_label(&format!(
        "{repository_hash}|{execution_spec_hash}|{capability_hash}|{environment_schema_hash}|{component_graph_hash}"
    ));

    let lock_material = format!(
        "fingerprint={execution_fingerprint}|runtime={runtime_environment_id}|framework={}|pm={}|deps={}",
        spec.runtime.framework,
        spec.runtime.package_manager.as_deref().unwrap_or("unknown"),
        spec.dependencies.join(",")
    );
    let runtime_hash = digest_label(&lock_material);
    let runtime_image_hash = digest_label(&format!("runtime-image:{runtime_environment_id}"));
    let node_version = if matches!(spec.runtime.language.as_str(), "javascript" | "typescript") {
        "24.2.1"
    } else {
        "n/a"
    };
    let pnpm_version = if spec.runtime.package_manager.as_deref() == Some("pnpm") {
        "10.5.2"
    } else {
        "n/a"
    };
    let next_version = if spec.runtime.framework.contains("next") {
        "16.0.1"
    } else {
        "n/a"
    };
    let execution_lock = format!(
        r#"runtime_hash = "{runtime_hash}"
node = "{node_version}"
pnpm = "{pnpm_version}"
next = "{next_version}"
component_graph = "{component_graph_hash}"
environment_schema = "{environment_schema_hash}"
capability_manifest = "{capability_hash}"
runtime_image = "{runtime_image_hash}"
execution_fingerprint = "{execution_fingerprint}""#
    );

    DeterministicExecutionArtifacts {
        execution_lock,
        runtime_graph_json,
        capabilities_toml,
        environment_schema_json,
        provenance_json,
        healing_patch,
        execution_fingerprint,
    }
}

fn digest_label(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(char::from_digit((byte >> 4) as u32, 16).expect("hex nibble"));
        hex.push(char::from_digit((byte & 0x0f) as u32, 16).expect("hex nibble"));
    }
    format!("sha256:{hex}")
}
