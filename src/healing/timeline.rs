#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealingStage {
    FailureClassified,
    RootCauseAnalyzed,
    CandidateGenerated,
    CandidateApplied,
    VerificationFailed,
    VerificationPassed,
    BudgetExhausted,
    HumanInterventionRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealingTimelineEvent {
    pub stage: HealingStage,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealingTimeline {
    pub events: Vec<HealingTimelineEvent>,
}

impl HealingTimeline {
    pub fn record(&mut self, stage: HealingStage, detail: impl Into<String>) {
        self.events.push(HealingTimelineEvent {
            stage,
            detail: detail.into(),
        });
    }
}
