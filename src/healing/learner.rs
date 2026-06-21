use crate::FailureClass;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnedPattern {
    pub failure_class: FailureClass,
    pub plan_id: String,
    pub successes: u32,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealingLearner {
    patterns: Vec<LearnedPattern>,
}

impl HealingLearner {
    pub fn learn(&mut self, failure_class: FailureClass, plan_id: &str, success: bool) {
        if let Some(pattern) = self
            .patterns
            .iter_mut()
            .find(|pattern| pattern.failure_class == failure_class && pattern.plan_id == plan_id)
        {
            pattern.attempts += 1;
            if success {
                pattern.successes += 1;
            }
            return;
        }
        self.patterns.push(LearnedPattern {
            failure_class,
            plan_id: plan_id.to_string(),
            successes: u32::from(success),
            attempts: 1,
        });
    }
}
