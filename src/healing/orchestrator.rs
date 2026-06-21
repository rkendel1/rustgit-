use crate::{ExecutionResult, FailureClass, FailureSignal, HealingRuntime, RepositoryFingerprint};

use super::{
    classifier::HealingClassifier,
    learner::HealingLearner,
    planner::{HealingPlan, HealingPlanner},
    rollback::RollbackEngine,
    root_cause::RootCauseEngine,
    timeline::{HealingStage, HealingTimeline},
    verifier::HealingVerifier,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealingBudget {
    pub max_attempts: usize,
}

impl Default for HealingBudget {
    fn default() -> Self {
        Self { max_attempts: 3 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrchestratorOutcome {
    Recovered {
        failure_class: FailureClass,
        plan: HealingPlan,
        result: ExecutionResult,
    },
    BudgetExhausted {
        failure_class: FailureClass,
        plan: HealingPlan,
    },
    HumanInterventionRequired {
        failure_class: FailureClass,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrchestratorDecision {
    pub outcome: OrchestratorOutcome,
    pub timeline: HealingTimeline,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HealingOrchestrator {
    classifier: HealingClassifier,
    root_cause: RootCauseEngine,
    planner: HealingPlanner,
    verifier: HealingVerifier,
    rollback: RollbackEngine,
    learner: HealingLearner,
}

impl HealingOrchestrator {
    pub fn run<R: HealingRuntime>(
        &mut self,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
        runtime: &mut R,
        budget: HealingBudget,
    ) -> OrchestratorDecision {
        let mut timeline = HealingTimeline::default();
        let classified = self.classifier.classify(failure, fingerprint);
        timeline.record(
            HealingStage::FailureClassified,
            format!("{:?}", classified.class),
        );

        let root = self.root_cause.analyze(&classified, failure, fingerprint);
        timeline.record(HealingStage::RootCauseAnalyzed, root.probable_cause.clone());

        let candidates = self
            .planner
            .rank_candidates(self.planner.generate_candidates(
                &classified,
                failure,
                fingerprint,
                &root,
            ));
        if candidates.is_empty() {
            timeline.record(HealingStage::HumanInterventionRequired, "no candidates");
            return OrchestratorDecision {
                outcome: OrchestratorOutcome::HumanInterventionRequired {
                    failure_class: classified.class,
                },
                timeline,
            };
        }

        let attempt_limit = budget.max_attempts.min(candidates.len());
        for candidate in candidates.iter().take(attempt_limit) {
            timeline.record(HealingStage::CandidateGenerated, candidate.id.clone());
            let mut applied = true;
            for repair_action in candidate.steps.iter().copied() {
                if !runtime.apply_repair(repair_action) {
                    applied = false;
                    break;
                }
            }

            if !applied {
                for rollback_action in self.rollback.rollback_steps(candidate) {
                    let _ = runtime.apply_repair(rollback_action);
                }
                timeline.record(HealingStage::VerificationFailed, candidate.id.clone());
                self.learner.learn(classified.class, &candidate.id, false);
                continue;
            }

            timeline.record(HealingStage::CandidateApplied, candidate.id.clone());
            let result = runtime.re_execute();
            let verification = self.verifier.verify(&result, runtime.health_check());
            if verification.successful() {
                self.learner.learn(classified.class, &candidate.id, true);
                timeline.record(HealingStage::VerificationPassed, candidate.id.clone());
                return OrchestratorDecision {
                    outcome: OrchestratorOutcome::Recovered {
                        failure_class: classified.class,
                        plan: candidate.clone(),
                        result,
                    },
                    timeline,
                };
            }

            for rollback_action in self.rollback.rollback_steps(candidate) {
                let _ = runtime.apply_repair(rollback_action);
            }
            self.learner.learn(classified.class, &candidate.id, false);
            timeline.record(HealingStage::VerificationFailed, candidate.id.clone());
        }

        let fallback_plan = candidates[0].clone();
        timeline.record(HealingStage::BudgetExhausted, fallback_plan.id.clone());
        OrchestratorDecision {
            outcome: OrchestratorOutcome::BudgetExhausted {
                failure_class: classified.class,
                plan: fallback_plan,
            },
            timeline,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{ExecutionResult, FailureSignal, HealingRuntime, RepositoryFingerprint};

    use super::{HealingBudget, HealingOrchestrator, OrchestratorOutcome};

    #[derive(Debug)]
    struct StubRuntime {
        fail_first_apply: bool,
        apply_count: usize,
        result: ExecutionResult,
        healthy: bool,
    }

    impl HealingRuntime for StubRuntime {
        fn apply_repair(&mut self, _action: crate::RepairAction) -> bool {
            self.apply_count += 1;
            if self.fail_first_apply && self.apply_count == 1 {
                return false;
            }
            true
        }

        fn re_execute(&mut self) -> ExecutionResult {
            self.result.clone()
        }

        fn health_check(&self) -> bool {
            self.healthy
        }
    }

    #[test]
    fn orchestrator_retries_next_candidate_after_failed_application() {
        let mut orchestrator = HealingOrchestrator::default();
        let mut runtime = StubRuntime {
            fail_first_apply: true,
            apply_count: 0,
            result: ExecutionResult {
                started: true,
                stable: true,
                message: "ok".to_string(),
            },
            healthy: true,
        };
        let failure = FailureSignal {
            message: "ERR_PNPM_LOCKFILE_MISMATCH".to_string(),
            attempted_command: Some("npm install".to_string()),
            expected_package_manager: Some("pnpm".to_string()),
            ..FailureSignal::default()
        };
        let decision = orchestrator.run(
            &failure,
            &RepositoryFingerprint::default(),
            &mut runtime,
            HealingBudget { max_attempts: 3 },
        );
        assert!(runtime.apply_count >= 3);
        match decision.outcome {
            OrchestratorOutcome::Recovered { .. } => {}
            _ => panic!("expected recovery after trying another candidate"),
        }
    }

    #[test]
    fn orchestrator_returns_budget_exhausted_when_attempts_are_used() {
        let mut orchestrator = HealingOrchestrator::default();
        let mut runtime = StubRuntime {
            fail_first_apply: false,
            apply_count: 0,
            result: ExecutionResult {
                started: true,
                stable: false,
                message: "still failing".to_string(),
            },
            healthy: false,
        };
        let failure = FailureSignal {
            message: "connection refused".to_string(),
            ..FailureSignal::default()
        };
        let decision = orchestrator.run(
            &failure,
            &RepositoryFingerprint::default(),
            &mut runtime,
            HealingBudget { max_attempts: 1 },
        );
        match decision.outcome {
            OrchestratorOutcome::BudgetExhausted { .. } => {}
            _ => panic!("expected budget exhausted"),
        }
    }
}
