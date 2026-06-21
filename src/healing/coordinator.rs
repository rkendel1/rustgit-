use crate::{
    ExecutionResult, FailureSignal, HealingRuntime, RecoveryStrategy, RepairStrategy,
    RepositoryFingerprint, RepositoryTimeGraph, TemporalExecutionRouter,
};

use super::orchestrator::{HealingBudget, HealingOrchestrator, OrchestratorOutcome};

#[derive(Debug, Clone, PartialEq)]
pub enum AutonomousOutcome {
    Recovered { result: ExecutionResult },
    EscalatedToTre { selected_commit: String },
    HumanInterventionRequired,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutonomousCoordinatorDecision {
    pub failure_class: crate::FailureClass,
    pub strategy: RepairStrategy,
    pub outcome: AutonomousOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutonomousHealingCoordinator {
    orchestrator: HealingOrchestrator,
    budget: HealingBudget,
}

impl Default for AutonomousHealingCoordinator {
    fn default() -> Self {
        Self {
            orchestrator: HealingOrchestrator::default(),
            budget: HealingBudget::default(),
        }
    }
}

impl AutonomousHealingCoordinator {
    pub fn heal<R: HealingRuntime>(
        &mut self,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
        runtime: &mut R,
        temporal_router: &TemporalExecutionRouter,
        graph: &RepositoryTimeGraph,
        head_commit: &str,
    ) -> AutonomousCoordinatorDecision {
        let run = self
            .orchestrator
            .run(failure, fingerprint, runtime, self.budget);
        match run.outcome {
            OrchestratorOutcome::Recovered {
                failure_class,
                plan,
                result,
            } => AutonomousCoordinatorDecision {
                failure_class,
                strategy: plan.to_strategy(),
                outcome: AutonomousOutcome::Recovered { result },
            },
            OrchestratorOutcome::BudgetExhausted {
                failure_class,
                plan,
            } => {
                if let Some(selected_commit) =
                    temporal_router.route(graph, head_commit, RecoveryStrategy::LastKnownGood)
                {
                    AutonomousCoordinatorDecision {
                        failure_class,
                        strategy: plan.to_strategy(),
                        outcome: AutonomousOutcome::EscalatedToTre { selected_commit },
                    }
                } else {
                    AutonomousCoordinatorDecision {
                        failure_class,
                        strategy: plan.to_strategy(),
                        outcome: AutonomousOutcome::HumanInterventionRequired,
                    }
                }
            }
            OrchestratorOutcome::HumanInterventionRequired { failure_class } => {
                AutonomousCoordinatorDecision {
                    failure_class,
                    strategy: RepairStrategy {
                        strategy_id: "repair::human_intervention".to_string(),
                        confidence: 0.0,
                        actions: vec![],
                    },
                    outcome: AutonomousOutcome::HumanInterventionRequired,
                }
            }
        }
    }
}
