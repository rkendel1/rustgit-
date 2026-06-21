use crate::RepairAction;

use super::planner::HealingPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RollbackEngine;

impl RollbackEngine {
    pub fn rollback_steps(&self, plan: &HealingPlan) -> Vec<RepairAction> {
        plan.rollback.clone()
    }
}
