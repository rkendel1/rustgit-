use crate::RepositoryAnalysis;

use super::RepositoryExecutionProfile;

pub fn discover_repository(analysis: &RepositoryAnalysis) -> RepositoryExecutionProfile {
    RepositoryExecutionProfile {
        repository_root: analysis.root.to_string_lossy().to_string(),
        repository_hash: analysis.fingerprint.repo_hash.clone(),
        classification: format!("{:?}", analysis.classification),
    }
}
