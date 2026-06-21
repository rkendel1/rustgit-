pub fn discover_filesystem(configuration_files: &[String], ci_files: &[String]) -> Vec<String> {
    let mut filesystem = configuration_files
        .iter()
        .chain(ci_files.iter())
        .cloned()
        .collect::<Vec<_>>();
    filesystem.sort();
    filesystem.dedup();
    filesystem
}

pub fn discover_network(package_manager: Option<&str>) -> Vec<String> {
    let mut network = vec!["github.com".to_string()];
    match package_manager.unwrap_or("unknown") {
        "pnpm" | "npm" | "yarn" | "bun" => network.push("registry.npmjs.org".to_string()),
        "cargo" => network.push("crates.io".to_string()),
        "pip" | "pipenv" | "poetry" | "uv" => network.push("pypi.org".to_string()),
        _ => {}
    }
    network.sort();
    network.dedup();
    network
}
