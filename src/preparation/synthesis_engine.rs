use super::SoftwareExecutionSpec;

pub fn portable_execution_toml(spec: &SoftwareExecutionSpec) -> String {
    let package_manager = spec.runtime.package_manager.as_deref().unwrap_or("unknown");
    let services = spec
        .services
        .iter()
        .map(|service| format!("{} = \"{}\"", service.name, service.mode))
        .collect::<Vec<_>>()
        .join("\n");
    let environment = spec
        .environment
        .iter()
        .map(|(name, source)| format!("{name} = \"{source}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let capabilities = spec
        .capabilities
        .iter()
        .map(|capability| format!("\"{capability}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"version = "{version}"
[runtime]
language = "{language}"
framework = "{framework}"
package_manager = "{package_manager}"
[services]
{services}
[environment]
{environment}
[capabilities]
all = [{capabilities}]
[healing]
apply_known_repairs = true
[confidence]
expected_success = {expected_success}"#,
        version = spec.identity.version,
        language = spec.runtime.language,
        framework = spec.runtime.framework,
        package_manager = package_manager,
        services = services,
        environment = environment,
        capabilities = capabilities,
        expected_success = format!("{:.3}", spec.confidence as f32 / 100.0)
    )
}
