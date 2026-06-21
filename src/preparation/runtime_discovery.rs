use crate::RepositoryAnalysis;

use super::RuntimeExecutionProfile;

pub fn discover_runtime(analysis: &RepositoryAnalysis) -> RuntimeExecutionProfile {
    RuntimeExecutionProfile {
        language: analysis.runtime_spec.language.clone(),
        framework: analysis.runtime_spec.framework.clone(),
        package_manager: analysis.runtime_spec.package_manager.clone(),
        requires_wasm: analysis.runtime_spec.requires_wasm,
    }
}

pub fn discover_languages(analysis: &RepositoryAnalysis) -> Vec<String> {
    vec![analysis.runtime_spec.language.clone()]
}

pub fn discover_frameworks(analysis: &RepositoryAnalysis) -> Vec<String> {
    vec![analysis.runtime_spec.framework.clone()]
}
