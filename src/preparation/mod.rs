use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod capability_discovery;
pub mod configuration_discovery;
pub mod dependency_discovery;
pub mod environment_discovery;
pub mod execution_spec_builder;
pub mod repository_discovery;
pub mod runtime_discovery;
pub mod service_discovery;
pub mod synthesis_engine;
pub mod validation_engine;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSpecIdentity {
    pub version: String,
    pub spec_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryExecutionProfile {
    pub repository_root: String,
    pub repository_hash: String,
    pub classification: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeExecutionProfile {
    pub language: String,
    pub framework: String,
    pub package_manager: Option<String>,
    pub requires_wasm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceExecutionProfile {
    pub name: String,
    pub runtime: String,
    pub port: Option<u16>,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareExecutionSpec {
    pub identity: ExecutionSpecIdentity,
    pub repository: RepositoryExecutionProfile,
    pub runtime: RuntimeExecutionProfile,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub dependencies: Vec<String>,
    pub services: Vec<ServiceExecutionProfile>,
    pub environment: BTreeMap<String, String>,
    pub secrets: Vec<String>,
    pub capabilities: Vec<String>,
    pub filesystem: Vec<String>,
    pub network: Vec<String>,
    pub ports: Vec<u16>,
    pub build_plan: Vec<String>,
    pub execution_plan: Vec<String>,
    pub healing_plan: Vec<String>,
    pub validation_plan: Vec<String>,
    pub optimization_plan: Vec<String>,
    pub confidence: u8,
}
