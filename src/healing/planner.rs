use crate::{FailureSignal, HealingCatalog, RepairAction, RepairStrategy, RepositoryFingerprint};

use super::{
    classifier::ClassifiedFailure, confidence::plan_confidence, root_cause::RootCauseGraph,
};

#[derive(Debug, Clone, PartialEq)]
pub struct HealingPlan {
    pub id: String,
    pub confidence: f32,
    pub steps: Vec<RepairAction>,
    pub rollback: Vec<RepairAction>,
    pub verification: Vec<String>,
    pub expected_outcome: String,
    pub estimated_duration_seconds: u32,
    pub estimated_risk: f32,
}

impl HealingPlan {
    pub fn to_strategy(&self) -> RepairStrategy {
        RepairStrategy {
            strategy_id: self.id.clone(),
            confidence: self.confidence,
            actions: self.steps.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealingPlanner {
    catalog: HealingCatalog,
}

impl HealingPlanner {
    pub fn generate_candidates(
        &self,
        classified: &ClassifiedFailure,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
        root_cause: &RootCauseGraph,
    ) -> Vec<HealingPlan> {
        let mut plans = Vec::new();
        let catalog_strategy = self
            .catalog
            .strategy_for(classified.class, failure, fingerprint);
        plans.push(HealingPlan {
            id: catalog_strategy.strategy_id,
            confidence: plan_confidence(&catalog_strategy.actions, catalog_strategy.confidence),
            steps: catalog_strategy.actions,
            rollback: vec![],
            verification: default_verification_steps(),
            expected_outcome: "execution stable".to_string(),
            estimated_duration_seconds: 45,
            estimated_risk: 0.20,
        });

        if root_cause
            .probable_cause
            .to_ascii_lowercase()
            .contains("lockfile")
            || classified.class == crate::FailureClass::WrongPackageManager
        {
            let steps = vec![
                RepairAction::RegenerateLockfile,
                RepairAction::SwitchPackageManager,
            ];
            plans.push(HealingPlan {
                id: "repair::dependency_agent::lockfile_sync".to_string(),
                confidence: plan_confidence(&steps, 0.90),
                steps,
                rollback: vec![RepairAction::RegenerateLockfile],
                verification: default_verification_steps(),
                expected_outcome: "lockfile and package manager aligned".to_string(),
                estimated_duration_seconds: 75,
                estimated_risk: 0.30,
            });
        }

        plans
    }

    pub fn rank_candidates(&self, mut candidates: Vec<HealingPlan>) -> Vec<HealingPlan> {
        candidates.sort_by(|first, second| second.confidence.total_cmp(&first.confidence));
        candidates
    }
}

fn default_verification_steps() -> Vec<String> {
    vec![
        "build".to_string(),
        "tests".to_string(),
        "health".to_string(),
        "smoke".to_string(),
        "static_analysis".to_string(),
    ]
}
