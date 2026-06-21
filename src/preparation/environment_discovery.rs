use serde_json::Value;
use std::collections::BTreeMap;

pub fn discover_environment(environment_graph: &[Value]) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for entry in environment_graph {
        let name = entry.get("name").and_then(Value::as_str);
        let source = entry.get("value_source").and_then(Value::as_str);
        if let (Some(name), Some(source)) = (name, source) {
            environment.insert(name.to_string(), source.to_string());
        }
    }
    environment
}

pub fn discover_secrets(environment_graph: &[Value]) -> Vec<String> {
    let mut secrets = environment_graph
        .iter()
        .filter(|entry| entry.get("required").and_then(Value::as_bool) == Some(true))
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    secrets.sort();
    secrets.dedup();
    secrets
}
