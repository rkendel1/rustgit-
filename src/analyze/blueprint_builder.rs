use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionProviderCapability {
    pub name: String,
    pub priority: u16,
    pub enabled: bool,
    pub healthy: bool,
    #[serde(rename = "estimatedStartup")]
    pub estimated_startup: String,
    pub supports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecommendedProvider {
    pub provider: String,
    pub confidence: u8,
    #[serde(skip_serializing_if = "Option::is_none", rename = "estimatedStartup")]
    pub estimated_startup: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionPlan {
    pub framework: String,
    #[serde(rename = "recommendedProviders")]
    pub recommended_providers: Vec<RecommendedProvider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionVerification {
    #[serde(rename = "canRunInWasm")]
    pub can_run_in_wasm: bool,
    #[serde(rename = "canRunNatively")]
    pub can_run_natively: bool,
    #[serde(rename = "canRunOnUserMachine")]
    pub can_run_on_user_machine: bool,
    #[serde(rename = "requiresDocker")]
    pub requires_docker: bool,
    #[serde(rename = "requiresPython")]
    pub requires_python: bool,
    #[serde(rename = "requiresSystemPackages")]
    pub requires_system_packages: bool,
    #[serde(rename = "requiresBrowserApis")]
    pub requires_browser_apis: bool,
    #[serde(rename = "requiresSecrets")]
    pub requires_secrets: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionBlueprint {
    #[serde(rename = "preferredProvider")]
    pub preferred_provider: String,
    pub fallback: Vec<String>,
    #[serde(rename = "recommendedProviders")]
    pub recommended_providers: Vec<RecommendedProvider>,
    #[serde(rename = "selectedBecause")]
    pub selected_because: Vec<String>,
    pub verification: ExecutionVerification,
}

pub fn capability_registry() -> Vec<ExecutionProviderCapability> {
    vec![
        ExecutionProviderCapability {
            name: "UserMachine".to_string(),
            priority: 100,
            enabled: true,
            healthy: true,
            estimated_startup: "<1s".to_string(),
            supports: vec![
                "node", "vite", "react", "nextjs", "python", "rust", "docker", "native",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        },
        ExecutionProviderCapability {
            name: "WASM".to_string(),
            priority: 90,
            enabled: true,
            healthy: true,
            estimated_startup: "1s".to_string(),
            supports: vec!["react", "vite", "svelte", "static", "node"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        },
        ExecutionProviderCapability {
            name: "NativeSandbox".to_string(),
            priority: 80,
            enabled: true,
            healthy: true,
            estimated_startup: "2-4s".to_string(),
            supports: vec![
                "node", "python", "fastapi", "django", "bun", "pnpm", "rust", "go",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        },
        ExecutionProviderCapability {
            name: "RemoteWorkspace".to_string(),
            priority: 50,
            enabled: true,
            healthy: true,
            estimated_startup: "10-20s".to_string(),
            supports: vec!["everything".to_string()],
        },
        ExecutionProviderCapability {
            name: "ContainerizedBuild".to_string(),
            priority: 40,
            enabled: true,
            healthy: true,
            estimated_startup: "20-40s".to_string(),
            supports: vec!["docker".to_string(), "system-packages".to_string()],
        },
        ExecutionProviderCapability {
            name: "LongRunningWorkspace".to_string(),
            priority: 30,
            enabled: true,
            healthy: true,
            estimated_startup: "30-60s".to_string(),
            supports: vec!["everything".to_string()],
        },
    ]
}

pub fn runtime_capability_statuses() -> Vec<ExecutionProviderCapabilityStatus> {
    capability_registry()
        .into_iter()
        .map(|provider| ExecutionProviderCapabilityStatus {
            name: provider.name,
            enabled: provider.enabled,
            healthy: provider.healthy,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionProviderCapabilityStatus {
    pub name: String,
    pub enabled: bool,
    pub healthy: bool,
}

pub fn build_execution_plan(runtime: &str, framework: &str) -> ExecutionPlan {
    let runtime_token = runtime.to_ascii_lowercase();
    let framework_token = framework.to_ascii_lowercase();
    let mut providers = capability_registry()
        .into_iter()
        .filter(|provider| provider.enabled && provider.healthy)
        .filter(|provider| {
            provider.supports.iter().any(|supported| {
                let token = supported.to_ascii_lowercase();
                token == "everything"
                    || token == runtime_token
                    || token == framework_token
                    || (token == "node" && matches!(runtime_token.as_str(), "bun" | "deno"))
            })
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| right.priority.cmp(&left.priority));

    if providers.is_empty() {
        providers = capability_registry()
            .into_iter()
            .filter(|provider| provider.name == "RemoteWorkspace")
            .collect();
    }

    let recommended_providers = providers
        .iter()
        .enumerate()
        .map(|(index, provider)| {
            let confidence = provider
                .priority
                .saturating_sub((index as u16).saturating_mul(5))
                .clamp(1, 100) as u8;
            RecommendedProvider {
                provider: provider.name.clone(),
                confidence,
                estimated_startup: Some(provider.estimated_startup.clone()),
            }
        })
        .collect::<Vec<_>>();
    ExecutionPlan {
        framework: framework.to_string(),
        recommended_providers,
    }
}

pub fn build_blueprint(
    runtime: &str,
    framework: &str,
    requires_docker: bool,
    requires_python: bool,
    requires_secrets: bool,
    requires_browser_apis: bool,
) -> ExecutionBlueprint {
    let plan = build_execution_plan(runtime, framework);
    let preferred_provider = plan
        .recommended_providers
        .first()
        .map(|provider| provider.provider.clone())
        .unwrap_or_else(|| "RemoteWorkspace".to_string());
    let fallback = plan
        .recommended_providers
        .iter()
        .skip(1)
        .map(|provider| provider.provider.clone())
        .collect::<Vec<_>>();
    let selected_because = vec![
        "compatible".to_string(),
        "lowest startup".to_string(),
        "healthy".to_string(),
    ];

    ExecutionBlueprint {
        preferred_provider,
        fallback,
        recommended_providers: plan.recommended_providers,
        selected_because,
        verification: ExecutionVerification {
            can_run_in_wasm: capability_registry().iter().any(|provider| {
                provider.name == "WASM"
                    && provider.enabled
                    && provider.healthy
                    && provider.supports.iter().any(|supported| {
                        let token = supported.to_ascii_lowercase();
                        token == framework.to_ascii_lowercase()
                            || token == runtime.to_ascii_lowercase()
                            || token == "everything"
                    })
            }),
            can_run_natively: capability_registry().iter().any(|provider| {
                provider.name == "NativeSandbox" && provider.enabled && provider.healthy
            }),
            can_run_on_user_machine: capability_registry().iter().any(|provider| {
                provider.name == "UserMachine" && provider.enabled && provider.healthy
            }),
            requires_docker,
            requires_python,
            requires_system_packages: requires_docker || runtime_requires_system_packages(runtime),
            requires_browser_apis,
            requires_secrets,
        },
    }
}

fn runtime_requires_system_packages(runtime: &str) -> bool {
    matches!(runtime, "rust" | "go" | "java")
}

#[cfg(test)]
mod tests {
    use super::{build_blueprint, build_execution_plan, runtime_capability_statuses};

    #[test]
    fn execution_plan_prefers_user_machine_for_vite() {
        let plan = build_execution_plan("node", "vite");
        assert_eq!(
            plan.recommended_providers
                .first()
                .map(|provider| provider.provider.as_str()),
            Some("UserMachine")
        );
    }

    #[test]
    fn execution_blueprint_includes_verification_answers() {
        let blueprint = build_blueprint("python", "python", false, true, true, false);
        assert!(blueprint.verification.can_run_on_user_machine);
        assert!(blueprint.verification.can_run_natively);
        assert!(blueprint.verification.requires_python);
        assert!(blueprint.verification.requires_secrets);
    }

    #[test]
    fn capability_statuses_report_enablement_and_health() {
        let statuses = runtime_capability_statuses();
        assert!(statuses
            .iter()
            .any(|provider| { provider.name == "WASM" && provider.enabled && provider.healthy }));
    }
}
