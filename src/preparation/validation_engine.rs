use serde_json::Value;

pub fn build_validation_plan(expected_failures: &[Value]) -> Vec<String> {
    let mut plan = vec![
        "validate-runtime".to_string(),
        "validate-services".to_string(),
        "validate-secrets".to_string(),
        "validate-ports".to_string(),
        "validate-dependencies".to_string(),
        "validate-capabilities".to_string(),
    ];
    if !expected_failures.is_empty() {
        plan.push("validate-known-failure-remediations".to_string());
    }
    plan
}
