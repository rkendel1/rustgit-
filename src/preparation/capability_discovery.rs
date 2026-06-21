use crate::RepositoryAnalysis;

pub fn discover_capabilities(analysis: &RepositoryAnalysis) -> Vec<String> {
    let mut capabilities = analysis
        .compiled_runtime
        .wasi_component_graph
        .capabilities
        .needs
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}
