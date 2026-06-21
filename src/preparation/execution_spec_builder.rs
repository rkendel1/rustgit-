use serde_json::Value;

use crate::RepositoryAnalysis;

use super::capability_discovery::discover_capabilities;
use super::configuration_discovery::{discover_filesystem, discover_network};
use super::dependency_discovery::discover_dependencies;
use super::environment_discovery::{discover_environment, discover_secrets};
use super::repository_discovery::discover_repository;
use super::runtime_discovery::{discover_frameworks, discover_languages, discover_runtime};
use super::service_discovery::{discover_ports, discover_services};
use super::validation_engine::build_validation_plan;
use super::{ExecutionSpecIdentity, SoftwareExecutionSpec};

pub fn build_execution_spec(
    analysis: &RepositoryAnalysis,
    configuration_files: &[String],
    ci_files: &[String],
    environment_graph: &[Value],
    expected_failures: &[Value],
) -> SoftwareExecutionSpec {
    let repository = discover_repository(analysis);
    let runtime = discover_runtime(analysis);
    let dependencies = discover_dependencies(analysis);
    let services = discover_services(analysis);
    let ports = discover_ports(analysis);
    let capabilities = discover_capabilities(analysis);
    let environment = discover_environment(environment_graph);
    let secrets = discover_secrets(environment_graph);
    let filesystem = discover_filesystem(configuration_files, ci_files);
    let network = discover_network(runtime.package_manager.as_deref());

    SoftwareExecutionSpec {
        identity: ExecutionSpecIdentity {
            version: "1".to_string(),
            spec_id: format!("sespec-{}", analysis.fingerprint.repo_hash),
        },
        repository,
        runtime,
        languages: discover_languages(analysis),
        frameworks: discover_frameworks(analysis),
        dependencies,
        services,
        environment,
        secrets,
        capabilities,
        filesystem,
        network,
        ports,
        build_plan: analysis.runtime_spec.build_steps.clone(),
        execution_plan: analysis.runtime_spec.execution_steps.clone(),
        healing_plan: analysis.runtime_spec.recovery_steps.clone(),
        validation_plan: build_validation_plan(expected_failures),
        optimization_plan: analysis.runtime_spec.cache_layers.clone(),
        confidence: analysis
            .image_match_confidence
            .max(crate::PREFLIGHT_REPOSITORY_HEALTH_NO_DEPS),
    }
}
