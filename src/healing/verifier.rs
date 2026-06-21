use crate::ExecutionResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    pub build_passed: bool,
    pub tests_passed: bool,
    pub health_passed: bool,
    pub smoke_passed: bool,
    pub static_analysis_passed: bool,
}

impl VerificationReport {
    pub fn successful(&self) -> bool {
        self.build_passed
            && self.tests_passed
            && self.health_passed
            && self.smoke_passed
            && self.static_analysis_passed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealingVerifier;

impl HealingVerifier {
    pub fn verify(&self, result: &ExecutionResult, healthy: bool) -> VerificationReport {
        let build_passed = result.started;
        let tests_passed = result.stable;
        VerificationReport {
            build_passed,
            tests_passed,
            health_passed: healthy,
            smoke_passed: build_passed && healthy,
            static_analysis_passed: build_passed,
        }
    }
}
