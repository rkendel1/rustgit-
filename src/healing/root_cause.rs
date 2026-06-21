use crate::{FailureSignal, RepositoryFingerprint};

use super::classifier::ClassifiedFailure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootCauseGraph {
    pub evidence_nodes: Vec<String>,
    pub probable_cause: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RootCauseEngine;

impl RootCauseEngine {
    pub fn analyze(
        &self,
        classified: &ClassifiedFailure,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
    ) -> RootCauseGraph {
        let mut evidence_nodes = vec![format!("failure_class::{:?}", classified.class)];
        if !failure.message.is_empty() {
            evidence_nodes.push("stderr".to_string());
        }
        if failure.attempted_command.is_some() {
            evidence_nodes.push("execution_graph".to_string());
        }
        if fingerprint.build_signals.has_lockfile {
            evidence_nodes.push("dependencies".to_string());
        }
        if !failure.missing_environment_variables.is_empty() {
            evidence_nodes.push("environment".to_string());
        }
        RootCauseGraph {
            evidence_nodes,
            probable_cause: classified.message.clone(),
        }
    }
}
