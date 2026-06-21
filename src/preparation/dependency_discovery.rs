use crate::RepositoryAnalysis;

pub fn discover_dependencies(analysis: &RepositoryAnalysis) -> Vec<String> {
    let mut dependencies = analysis.runtime_spec.dependencies.clone();
    dependencies.sort();
    dependencies.dedup();
    dependencies
}
