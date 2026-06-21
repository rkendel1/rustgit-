use std::fs;
use std::path::Path;

use serde::Serialize;

use super::registry::{runtime_from_lockfile, runtime_registry, RuntimeRegistryEntry};

const DETECTION_FILES: [&str; 13] = [
    "bun.lockb",
    "bun.lock",
    "pnpm-lock.yaml",
    "package-lock.json",
    "yarn.lock",
    "requirements.txt",
    "pyproject.toml",
    "Cargo.toml",
    "go.mod",
    "pom.xml",
    "composer.json",
    "Gemfile",
    "deno.json",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeDetection {
    pub runtime: String,
    pub package_manager: Option<String>,
    pub build: Option<String>,
    pub start: Option<String>,
    pub dev: Option<String>,
    pub evidence: Vec<String>,
}

pub fn detect_runtime(root: &Path, framework: Option<&str>) -> RuntimeDetection {
    let mut evidence = Vec::new();

    for file in DETECTION_FILES {
        if root.join(file).exists() {
            evidence.push(file.to_string());
            if let Some(entry) = runtime_from_lockfile(file) {
                return to_detection(entry, evidence);
            }
        }
    }

    if let Some(framework_name) = framework {
        if let Some(entry) = runtime_registry().get(framework_name).copied() {
            evidence.push(format!("framework:{framework_name}"));
            return to_detection(entry, evidence);
        }
    }

    if root.join("package.json").exists() {
        evidence.push("package.json".to_string());
        return RuntimeDetection {
            runtime: "node".to_string(),
            package_manager: Some("npm".to_string()),
            build: detect_script(root, "build").or(Some("npm run build".to_string())),
            start: detect_script(root, "start").or(Some("npm run start".to_string())),
            dev: detect_script(root, "dev").or(Some("npm run dev".to_string())),
            evidence,
        };
    }

    RuntimeDetection {
        runtime: "unknown".to_string(),
        package_manager: None,
        build: None,
        start: None,
        dev: None,
        evidence,
    }
}

fn to_detection(entry: RuntimeRegistryEntry, evidence: Vec<String>) -> RuntimeDetection {
    RuntimeDetection {
        runtime: entry.runtime.to_string(),
        package_manager: entry.package_manager.map(ToString::to_string),
        build: entry.build.map(ToString::to_string),
        start: entry.start.map(ToString::to_string),
        dev: entry.dev.map(ToString::to_string),
        evidence,
    }
}

fn detect_script(root: &Path, script_name: &str) -> Option<String> {
    let package_json = root.join("package.json");
    let content = fs::read_to_string(package_json).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    value
        .get("scripts")?
        .get(script_name)?
        .as_str()
        .map(|command| command.to_string())
}
