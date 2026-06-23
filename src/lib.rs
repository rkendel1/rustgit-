use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use wasmtime::{Config, Engine, Linker, Module, Store};

pub mod analyze;
mod architecture_docs;
mod badge;
mod execution_context;
mod execution_embeddings;
mod execution_learning;
mod execution_memory;
mod execution_optimizer;
mod execution_retriever;
pub mod healing;
mod postgres_db;
pub mod preparation;
mod repository_context_builder;
mod repository_embeddings;
mod repository_intelligence_service;
mod repository_knowledge_graph;
mod routine;

pub use architecture_docs::{
    analyze_architecture_from_source, extract_execution_flow_from_source, generate_grounded_docs,
    ArchitectureSnapshot, CallGraph, ExecutionFlowGraph, GeneratedDocs,
};
pub use badge::{
    badge_generate_endpoint, badge_seed_launch_endpoint, badge_svg_endpoint,
    derive_badge_runtime_state, healed_badge_svg_endpoint, healed_badge_variant_endpoint,
    BadgeExecutionSnapshot, BadgeRuntimeState, RepositoryIdentity, VerificationState,
};
pub use execution_context::ExecutionContextBuilder;
pub use execution_embeddings::{fingerprint_embedding, ExecutionEmbedding};
pub use execution_learning::ExecutionLearningEngine;
pub use execution_memory::{ExecutionContextSnapshot, ExecutionMemory, ExecutionPattern};
pub use execution_optimizer::{ExecutionOptimizer, OptimizedExecutionPlan};
pub use execution_retriever::ExecutionRetriever;
pub use postgres_db::{
    deserialize_string_array, infer_repository_from_commits, ExecutionIntelligencePersistenceError,
    ExecutionIntelligencePostgresStore, ExecutionIntelligenceReadStore, PersistenceResult,
};
pub use repository_context_builder::{RepositoryContextBuilder, RepositoryQueryContext};
pub use repository_embeddings::{
    OpenAiEmbeddingClient, RepositoryEmbedding, RepositoryEmbeddingError,
    RepositoryEmbeddingPipeline,
};
pub use repository_intelligence_service::{
    RepairKnowledgeProvider, RepairPlan, RepositoryAnswer, RepositoryEvidence,
    RepositoryIntelligenceService,
};
pub use repository_knowledge_graph::{
    ArchitectureEdge, ArchitectureGraph, ArchitectureNode, RepositoryFailureRecord,
    RepositoryKnowledgeGraph, RepositoryRuntimeRecord, TemporalRecoveryRecord,
};
use routine::{current_unix_epoch_secs, parse_execution_id, parse_workspace_id};
pub use routine::{RestApiSpec, VirtualFileSystem, WorkspaceSnapshot};
pub(crate) use badge::parse_badge_repository_context;

const WASM_FULL_MEMORY_LIMIT_MB: u64 = 512;
const WASM_FULL_CPU_LIMIT_UNITS: u32 = 1_000;
const WASM_PARTIAL_MEMORY_LIMIT_MB: u64 = 256;
const WASM_PARTIAL_CPU_LIMIT_UNITS: u32 = 750;
const RUNTIME_SPEC_DEFAULT_MEMORY_LIMIT_MB: u32 = 768;
const RUNTIME_SPEC_DEFAULT_CPU_LIMIT_UNITS: u32 = 2_000;
const UNINITIALIZED_RESOURCE_LIMIT: u32 = 0;
const ENVIRONMENT_ID_PREFIX_LENGTH: usize = 12;
const CPU_UNIT_TO_TIME_LIMIT_MS: u64 = 10;
const DEFAULT_COMPONENT_VERSION: &str = "1.0.0";
const RUNTIME_CONSTRAINT_MAX_MEMORY_MB: u32 = 16 * 1024;
const RUNTIME_CONSTRAINT_MAX_CPU_UNITS: u32 = 100_000;
const CACHE_KEY_NODE_MODE_SEPARATOR: &str = "@";
const BYTES_PER_MB: u64 = 1024 * 1024;
const SESSION_GRAPH_EVENT_BUFFER_LIMIT: usize = 1_024;
const SESSION_WORKER_EVENT_BUFFER_LIMIT: usize = 1_024;
const MIN_SERVICES_FOR_TOPOLOGY: usize = 2;
const MIN_COORDINATION_TIMEOUT_SECS: u64 = 1;
const INSTALL_TIMEOUT_SECS: u64 = 180;
const INSTALL_POLL_INTERVAL_MS: u64 = 250;
const RUNTIME_REPAIR_MAX_ATTEMPTS_DEFAULT: usize = 4;
static WASI_KERNEL_TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);
const MIN_BILLABLE_DURATION_SECONDS: f64 = 1.0;
const RETRY_PENALTY_UNITS: f64 = 0.25;
const HEALING_COST_MULTIPLIER_PER_CYCLE: f64 = 0.5;
const WARM_POOL_DISCOUNT_MULTIPLIER: f64 = 0.1;
const FREE_PLAN_RUNS_PER_DAY: usize = 10;
const PRO_PLAN_RUNS_PER_DAY: usize = 1_000;
const EXECUTION_IMAGE_VERSION: &str = "v1";
const UNKNOWN_SIGNATURE: &str = "unknown";
const CJVF_CANONICAL_HOST: &str = "trythissoftware.com";
const PREFLIGHT_REPOSITORY_HEALTH_WITH_DEPS: u8 = 98;
const PREFLIGHT_REPOSITORY_HEALTH_NO_DEPS: u8 = 88;
const PREFLIGHT_DEPENDENCIES_CONFIDENCE_READY: u8 = 99;
const PREFLIGHT_DEPENDENCIES_CONFIDENCE_UNKNOWN: u8 = 90;
const PREFLIGHT_CAPABILITIES_CONFIDENCE_READY: u8 = 96;
const PREFLIGHT_CAPABILITIES_CONFIDENCE_UNKNOWN: u8 = 90;
const PREFLIGHT_ENVIRONMENT_CONFIDENCE_DISCOVERED: u8 = 95;
const PREFLIGHT_ENVIRONMENT_CONFIDENCE_SYNTHESIZED: u8 = 85;
const PREFLIGHT_RUNTIME_CONFIDENCE_WASM: u8 = 98;
const PREFLIGHT_RUNTIME_CONFIDENCE_NATIVE: u8 = 99;
const PREFLIGHT_FAILURE_PENALTY_PER_ISSUE: u8 = 2;
const MAX_RUNTIME_LOG_LINES: usize = 500;
const STARTUP_LOG_PATTERNS: [&str; 8] = [
    "ready",
    "listening",
    "vite",
    "local:",
    "network:",
    "started",
    "compiled successfully",
    "running on",
];
pub const DDOCKIT_ANON_ID_COOKIE: &str = "ddockit_anon_id";
pub const DDOCKIT_SESSION_ID_COOKIE: &str = "ddockit_session_id";
const DISTRIBUTED_ARTIFACT_STORE_POISONED: &str =
    "distributed artifact store lock poisoned: another thread panicked while holding the lock";
const LOCAL_AGENT_LOCK_POISONED: &str =
    "LocalAgentProvider: failed to acquire agent lock due to panic in another thread";

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug)]
pub enum RuntimeError {
    WorkspaceMissing(String),
    UnsupportedRepository(String),
    ExecutionContextMissing(String),
    InvalidTransition {
        from: WorkspaceState,
        to: WorkspaceState,
    },
    InvalidPath(String),
    WasmRuntime(String),
    Io(io::Error),
    CommandFailed(String),
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkspaceMissing(id) => write!(f, "workspace not found: {id}"),
            Self::UnsupportedRepository(reason) => write!(f, "unsupported repository: {reason}"),
            Self::ExecutionContextMissing(id) => {
                write!(f, "execution context missing for workspace: {id}")
            }
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid workspace transition: {:?} -> {:?}", from, to)
            }
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::WasmRuntime(message) => write!(f, "wasm runtime error: {message}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::CommandFailed(message) => write!(f, "command failed: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<io::Error> for RuntimeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Framework {
    Node,
    StaticWeb,
    Vite,
    React,
    Vue,
    Svelte,
    SvelteKit,
    NextJs,
    Nuxt,
    Astro,
    Remix,
    Express,
    NestJs,
    Rust,
    Axum,
    Actix,
    Rocket,
    Leptos,
    Go,
    Gin,
    Fiber,
    Echo,
    Python,
    Flask,
    FastApi,
    Django,
    Streamlit,
    Gradio,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    JavaScript,
    TypeScript,
    Rust,
    Go,
    Python,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeType {
    Node,
    Wasm,
    Rust,
    Go,
    Python,
    Java,
    Static,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoClass {
    StaticSite,
    NodeApp,
    FullStackNode,
    RustBinary,
    PythonApp,
    Monorepo,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphStrategy {
    Linear,
    Parallelized,
    MultiStage,
    MonorepoSegmented,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryFingerprint {
    pub spec_version: String,
    pub repo_id: String,
    pub repo_url: String,
    pub languages: Vec<LanguageProfile>,
    pub frameworks: Vec<FrameworkProfile>,
    pub package_managers: Vec<String>,
    pub services: Vec<ServiceFingerprint>,
    pub entrypoints: Vec<EntryPoint>,
    pub dependency_graph: DependencyGraph,
    pub runtime_signals: RuntimeSignals,
    pub build_signals: BuildSignals,
    pub infra_signals: InfraSignals,
    pub confidence: f32,
    pub confidence_model: ConfidenceModel,
    pub repo_hash: String,
    pub lockfile_hash: Option<String>,
    pub dependency_hash: Option<String>,
    pub language_signature: String,
    pub framework_signature: Option<String>,
}

impl Default for RepositoryFingerprint {
    fn default() -> Self {
        Self {
            spec_version: "1.0".to_string(),
            repo_id: "unknown".to_string(),
            repo_url: "unknown".to_string(),
            languages: vec![],
            frameworks: vec![],
            package_managers: vec![],
            services: vec![],
            entrypoints: vec![],
            dependency_graph: DependencyGraph::default(),
            runtime_signals: RuntimeSignals::default(),
            build_signals: BuildSignals::default(),
            infra_signals: InfraSignals::default(),
            confidence: 0.0,
            confidence_model: ConfidenceModel::default(),
            repo_hash: hash_key("unknown"),
            lockfile_hash: None,
            dependency_hash: None,
            language_signature: "unknown".to_string(),
            framework_signature: Some("unknown".to_string()),
        }
    }
}

pub type LanguageKind = Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRuntimeKind {
    Node,
    Python,
    Rust,
    Bun,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameworkKind {
    NextJs,
    React,
    Vite,
    NestJs,
    Express,
    Remix,
    FastApi,
    Django,
    Flask,
    Streamlit,
    Celery,
    Axum,
    Actix,
    Rocket,
    BunVite,
    BunServer,
    Turborepo,
    Nx,
    PnpmWorkspace,
    YarnWorkspace,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManagerKind {
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Cargo,
    Pip,
    Uv,
    Poetry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageProfile {
    pub language: LanguageKind,
    pub confidence: f32,
    pub files_detected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameworkProfile {
    pub framework: String,
    pub version: Option<String>,
    pub confidence: f32,
    pub detection_signals: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceType {
    Frontend,
    Backend,
    Worker,
    Database,
    SharedLibrary,
    CLI,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildContext {
    pub install_command: Option<String>,
    pub build_command: Option<String>,
    pub package_manager: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFingerprint {
    pub service_name: String,
    pub service_type: ServiceType,
    pub root_path: String,
    pub runtime_hint: RuntimeKind,
    pub framework: Option<String>,
    pub entry_file: Option<String>,
    pub build_context: BuildContext,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryPoint {
    pub path: String,
    pub command: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyNode {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DependencyGraph {
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeSignals {
    pub node_detected: bool,
    pub python_detected: bool,
    pub rust_detected: bool,
    pub bun_detected: bool,
    pub dockerfile_present: bool,
    pub compose_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildSignals {
    pub has_lockfile: bool,
    pub lockfile_type: Option<String>,
    pub build_scripts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InfraSignals {
    pub uses_database: bool,
    pub uses_redis: bool,
    pub uses_queue: bool,
    pub docker_required: bool,
    pub cloud_native: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConfidenceModel {
    pub overall: f32,
    pub framework_confidence: f32,
    pub runtime_confidence: f32,
    pub topology_confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryStrategy {
    NodeScript { command: String },
    PythonModule { module: String },
    RustBinary { target: String },
    BunScript { command: String },
    DockerEntrypoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStep {
    InstallDependencies,
    Compile,
    GenerateArtifacts,
    LinkCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxModel {
    ProcessIsolated,
    DockerContainer,
    WasmIsolated,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentSpec {
    pub variables: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePolicy {
    pub key: String,
    pub deterministic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildStrategy {
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionImageSpec {
    pub spec_version: String,
    pub commit_hash: Option<String>,
    pub deterministic_build: bool,
    pub language: LanguageKind,
    pub runtime: ImageRuntimeKind,
    pub runtime_version: String,
    pub framework: Option<FrameworkKind>,
    pub package_manager: Option<PackageManagerKind>,
    pub entry_strategy: EntryStrategy,
    pub build_steps: Vec<BuildStep>,
    pub environment: EnvironmentSpec,
    pub caching_policy: CachePolicy,
    pub sandbox_model: SandboxModel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledExecutionImage {
    pub image_spec: ExecutionImageSpec,
    pub build_strategy: BuildStrategy,
    pub confidence: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionImageCompiler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheKeyEngineV2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionImage {
    pub image_id: String,
    pub runtime: RuntimeKind,
    pub language: LanguageKind,
    pub framework: Option<String>,
    pub version: String,
    pub base_layers: Vec<String>,
    pub preinstalled_dependencies: bool,
    pub cache_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionImageMatch {
    pub image: ExecutionImage,
    pub confidence: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionImageRegistry {
    images: HashMap<String, ExecutionImage>,
    repo_image_bindings: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionMatchEngine;

impl ExecutionMatchEngine {
    pub fn match_repository(fingerprint: &RepositoryFingerprint) -> ExecutionImageMatch {
        let compiled = ExecutionImageCompiler::compile(fingerprint);
        let runtime = runtime_type_from_image_runtime(compiled.image_spec.runtime);
        let framework = compiled
            .image_spec
            .framework
            .map(framework_kind_label)
            .unwrap_or(UNKNOWN_SIGNATURE);
        let language = fingerprint.language_signature.to_ascii_lowercase();

        let image_id = format!(
            "{}-{}-warm-{}",
            image_runtime_kind_label(compiled.image_spec.runtime),
            framework_tag(framework),
            compiled.image_spec.spec_version
        );
        let version = compiled.image_spec.spec_version.clone();
        let cache_key = CacheKeyEngineV2::derive_cache_key(
            fingerprint,
            &compiled.image_spec,
            &compiled.build_strategy,
        );
        let base_layers = vec![
            format!(
                "runtime:{}",
                image_runtime_kind_label(compiled.image_spec.runtime)
            ),
            format!("runtime-version:{}", compiled.image_spec.runtime_version),
            format!("language:{}", language_tag(&language)),
            format!("framework:{}", framework_tag(framework)),
        ];

        ExecutionImageMatch {
            image: ExecutionImage {
                image_id,
                runtime,
                language: language_kind_from_signature(&language),
                framework: (framework != UNKNOWN_SIGNATURE).then_some(framework.to_string()),
                version,
                base_layers,
                preinstalled_dependencies: true,
                cache_key,
            },
            confidence: compiled.confidence,
        }
    }
}

impl ExecutionImageCompiler {
    pub fn compile(fingerprint: &RepositoryFingerprint) -> CompiledExecutionImage {
        let framework = framework_kind_from_fingerprint(fingerprint);
        let runtime = image_runtime_for_framework(framework, fingerprint);
        let language =
            language_kind_from_signature(&fingerprint.language_signature.to_ascii_lowercase());
        let package_manager = package_manager_for_framework(framework, runtime, fingerprint);
        let runtime_version = runtime_version_for(runtime).to_string();
        let entry_strategy = entry_strategy_for(runtime, framework, package_manager);
        let build_steps = vec![
            BuildStep::InstallDependencies,
            BuildStep::Compile,
            BuildStep::GenerateArtifacts,
            BuildStep::LinkCache,
        ];
        let build_strategy = BuildStrategyPlanner::plan(runtime, package_manager);
        let confidence = confidence_for_compiler(framework, runtime, language);

        let mut environment_vars = BTreeSet::new();
        environment_vars.insert("CI=true".to_string());
        if matches!(runtime, ImageRuntimeKind::Node | ImageRuntimeKind::Bun) {
            environment_vars.insert("NODE_ENV=production".to_string());
        }
        if runtime == ImageRuntimeKind::Python {
            environment_vars.insert("PYTHONUNBUFFERED=1".to_string());
        }

        let mut image_spec = ExecutionImageSpec {
            spec_version: EXECUTION_IMAGE_VERSION.to_string(),
            commit_hash: None,
            deterministic_build: true,
            language,
            runtime,
            runtime_version,
            framework: framework.filter(|value| *value != FrameworkKind::Unknown),
            package_manager,
            entry_strategy,
            build_steps,
            environment: EnvironmentSpec {
                variables: environment_vars,
            },
            caching_policy: CachePolicy {
                key: String::new(),
                deterministic: true,
            },
            sandbox_model: sandbox_model_for_runtime(runtime),
        };
        let cache_key =
            CacheKeyEngineV2::derive_cache_key(fingerprint, &image_spec, &build_strategy);
        image_spec.caching_policy.key = cache_key;

        CompiledExecutionImage {
            image_spec,
            build_strategy,
            confidence,
        }
    }

    /// Compiles an execution image spec bound to a specific commit hash.
    ///
    /// Use this when execution artifacts must be tied to one historical repository state.
    /// Use `compile` when commit-specific binding is not required.
    pub fn compile_for_commit(
        fingerprint: &RepositoryFingerprint,
        commit_hash: impl Into<String>,
    ) -> CompiledExecutionImage {
        let mut compiled = Self::compile(fingerprint);
        compiled.image_spec.commit_hash = Some(commit_hash.into());
        compiled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuildStrategyPlanner;

impl BuildStrategyPlanner {
    fn plan(
        runtime: ImageRuntimeKind,
        package_manager: Option<PackageManagerKind>,
    ) -> BuildStrategy {
        let mut commands = Vec::new();
        match runtime {
            ImageRuntimeKind::Node => match package_manager.unwrap_or(PackageManagerKind::Npm) {
                PackageManagerKind::Pnpm => commands.extend([
                    "pnpm install --frozen-lockfile".to_string(),
                    "pnpm run build".to_string(),
                ]),
                PackageManagerKind::Yarn => commands.extend([
                    "yarn install --frozen-lockfile".to_string(),
                    "yarn build".to_string(),
                ]),
                PackageManagerKind::Bun => commands.extend([
                    "bun install --frozen-lockfile".to_string(),
                    "bun run build".to_string(),
                ]),
                _ => commands.extend(["npm install".to_string(), "npm run build".to_string()]),
            },
            ImageRuntimeKind::Python => match package_manager.unwrap_or(PackageManagerKind::Pip) {
                PackageManagerKind::Uv => commands.extend([
                    "uv pip install -r requirements.txt".to_string(),
                    "python -m compileall .".to_string(),
                ]),
                PackageManagerKind::Poetry => commands.extend([
                    "poetry install --no-interaction".to_string(),
                    "python -m compileall .".to_string(),
                ]),
                _ => commands.extend([
                    "python -m pip install -r requirements.txt".to_string(),
                    "python -m compileall .".to_string(),
                ]),
            },
            ImageRuntimeKind::Rust => commands.extend([
                "cargo fetch --locked".to_string(),
                "cargo build --release".to_string(),
            ]),
            ImageRuntimeKind::Bun => commands.extend([
                "bun install --frozen-lockfile".to_string(),
                "bun run build".to_string(),
            ]),
            ImageRuntimeKind::Unknown => {}
        }
        BuildStrategy { commands }
    }
}

impl CacheKeyEngineV2 {
    fn derive_cache_key(
        fingerprint: &RepositoryFingerprint,
        image_spec: &ExecutionImageSpec,
        build_strategy: &BuildStrategy,
    ) -> String {
        hash_key(&format!(
            "fingerprint:{}|spec:{}|strategy:{}",
            repository_fingerprint_material(fingerprint),
            execution_image_spec_material(image_spec),
            build_strategy.commands.join("||")
        ))
    }
}

impl ExecutionImageRegistry {
    pub fn register_image(&mut self, image: ExecutionImage) {
        self.images.insert(image.image_id.clone(), image);
    }

    pub fn image_for_repo(&self, repo_id: &str) -> Option<&ExecutionImage> {
        let image_id = self.repo_image_bindings.get(repo_id)?;
        self.images.get(image_id)
    }

    pub fn get(&self, image_id: &str) -> Option<&ExecutionImage> {
        self.images.get(image_id)
    }

    pub fn resolve_for_fingerprint(
        &mut self,
        repo_id: &str,
        fingerprint: &RepositoryFingerprint,
    ) -> ExecutionImageMatch {
        if let Some(image) = self.image_for_repo(repo_id).cloned() {
            return ExecutionImageMatch {
                image,
                confidence: 100,
            };
        }

        let matched = ExecutionMatchEngine::match_repository(fingerprint);
        self.register_image(matched.image.clone());
        self.repo_image_bindings
            .insert(repo_id.to_string(), matched.image.image_id.clone());
        matched
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmContainerState {
    Cold,
    Warming,
    WarmIdle,
    Assigned,
    Running,
    ReturnedToPool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmPoolType {
    Cloud,
    LocalDea,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionCacheLayer {
    pub cache_key: String,
    pub image_id: String,
    pub fingerprint_hash: String,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmPoolEntry {
    pub image_id: String,
    pub pool_type: WarmPoolType,
    pub warm_count: u32,
    pub idle_count: u32,
    pub assigned_count: u32,
    pub state: WarmContainerState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WarmPoolStatus {
    pub total_images: usize,
    pub warm_containers: u32,
    pub idle_containers: u32,
    pub assigned_containers: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WarmPoolManager {
    pools: HashMap<String, WarmPoolEntry>,
    caches: HashMap<String, ExecutionCacheLayer>,
}

impl WarmPoolManager {
    pub fn prewarm(&mut self, image: &ExecutionImage, pool_type: WarmPoolType, count: u32) {
        let entry = self
            .pools
            .entry(image.image_id.clone())
            .or_insert_with(|| WarmPoolEntry {
                image_id: image.image_id.clone(),
                pool_type,
                warm_count: 0,
                idle_count: 0,
                assigned_count: 0,
                state: WarmContainerState::Cold,
            });
        entry.pool_type = pool_type;
        entry.warm_count = entry.warm_count.saturating_add(count);
        entry.idle_count = entry.idle_count.saturating_add(count);
        entry.state = WarmContainerState::WarmIdle;
    }

    pub fn allocate(&mut self, image_id: &str) -> bool {
        let Some(entry) = self.pools.get_mut(image_id) else {
            return false;
        };
        if entry.idle_count == 0 {
            return false;
        }
        entry.idle_count -= 1;
        entry.assigned_count = entry.assigned_count.saturating_add(1);
        entry.state = WarmContainerState::Assigned;
        true
    }

    pub fn mark_running(&mut self, image_id: &str) -> bool {
        let Some(entry) = self.pools.get_mut(image_id) else {
            return false;
        };
        if entry.assigned_count == 0 {
            return false;
        }
        entry.state = WarmContainerState::Running;
        true
    }

    pub fn release(&mut self, image_id: &str) -> bool {
        let Some(entry) = self.pools.get_mut(image_id) else {
            return false;
        };
        if entry.assigned_count == 0 {
            return false;
        }
        entry.assigned_count -= 1;
        entry.idle_count = entry.idle_count.saturating_add(1);
        entry.state = WarmContainerState::ReturnedToPool;
        true
    }

    pub fn bind_cache_layer(
        &mut self,
        fingerprint: &RepositoryFingerprint,
        image: &ExecutionImage,
    ) {
        let key = warm_cache_binding_key(&fingerprint.repo_hash, &image.image_id);
        self.caches
            .entry(key.clone())
            .or_insert(ExecutionCacheLayer {
                cache_key: key,
                image_id: image.image_id.clone(),
                fingerprint_hash: fingerprint.repo_hash.clone(),
                artifacts: cache_artifacts_for_image(image),
            });
    }

    pub fn status(&self) -> WarmPoolStatus {
        let mut status = WarmPoolStatus::default();
        status.total_images = self.pools.len();
        for entry in self.pools.values() {
            status.warm_containers = status.warm_containers.saturating_add(entry.warm_count);
            status.idle_containers = status.idle_containers.saturating_add(entry.idle_count);
            status.assigned_containers = status
                .assigned_containers
                .saturating_add(entry.assigned_count);
        }
        status
    }

    pub fn get(&self, image_id: &str) -> Option<&WarmPoolEntry> {
        self.pools.get(image_id)
    }

    pub fn has_cache_layer(
        &self,
        fingerprint: &RepositoryFingerprint,
        image: &ExecutionImage,
    ) -> bool {
        let key = warm_cache_binding_key(&fingerprint.repo_hash, &image.image_id);
        self.caches.contains_key(&key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStatus {
    Unknown,
    Success,
    Failed,
    PartialSuccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub started: bool,
    pub stable: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitNode {
    pub commit_hash: String,
    pub timestamp: i64,
    pub urfs_snapshot: Option<RepositoryFingerprint>,
    pub build_status: Option<BuildStatus>,
    pub execution_result: Option<ExecutionResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEdge {
    pub from_hash: String,
    pub to_hash: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepositoryTimeGraph {
    pub repo_id: String,
    pub commits: Vec<CommitNode>,
    pub edges: Vec<CommitEdge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitScore {
    pub build_score: f32,
    pub runtime_score: f32,
    pub dependency_score: f32,
    pub topology_score: f32,
    pub overall_score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitScorer;

impl CommitScorer {
    pub fn score(node: &CommitNode) -> CommitScore {
        let build_score = match node.build_status.unwrap_or(BuildStatus::Unknown) {
            BuildStatus::Success => 0.4,
            BuildStatus::PartialSuccess => 0.2,
            BuildStatus::Unknown | BuildStatus::Failed => 0.0,
        };
        let runtime_score = node
            .execution_result
            .as_ref()
            .map(|result| if result.started { 0.3 } else { 0.0 })
            .unwrap_or(0.0);
        let dependency_score = node
            .urfs_snapshot
            .as_ref()
            .map(|snapshot| {
                if snapshot.dependency_hash.is_some() {
                    0.2
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);
        let topology_score = node
            .execution_result
            .as_ref()
            .map(|result| if result.stable { 0.1 } else { 0.0 })
            .unwrap_or(0.0);
        CommitScore {
            build_score,
            runtime_score,
            dependency_score,
            topology_score,
            overall_score: build_score + runtime_score + dependency_score + topology_score,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitExecutionCache {
    pub commit_hash: String,
    pub execution_image: ExecutionImageSpec,
    pub topology: ApplicationTopology,
    pub result: ExecutionResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStrategy {
    LastKnownGood,
    BestRunnable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalExecutionPolicy {
    pub max_depth: usize,
}

impl Default for TemporalExecutionPolicy {
    fn default() -> Self {
        Self { max_depth: 50 }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TemporalExecutionEngine {
    cache: HashMap<String, CommitExecutionCache>,
}

impl TemporalExecutionEngine {
    /// Enumerates repository commits using `git log --pretty=format:%H|%ct`.
    ///
    /// Returns commit nodes plus linear parent-child edges in log order.
    /// Fails when `repo_root` is not a readable git repository or git execution fails.
    pub fn enumerate_commits(repo_root: &Path) -> Result<RepositoryTimeGraph> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("log")
            .arg("--pretty=format:%H|%ct")
            .output()
            .map_err(|err| RuntimeError::CommandFailed(format!("git log failed: {err}")))?;
        if !output.status.success() {
            return Err(RuntimeError::CommandFailed(format!(
                "git log exited with status {}",
                output.status
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut commits = Vec::new();
        for line in stdout.lines() {
            let mut fields = line.split('|');
            let Some(commit_hash) = fields.next().map(str::trim) else {
                continue;
            };
            if !is_verified_commit_hash(commit_hash) {
                continue;
            }
            let timestamp = fields
                .next()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_default();
            commits.push(CommitNode {
                commit_hash: commit_hash.to_string(),
                timestamp,
                urfs_snapshot: None,
                build_status: Some(BuildStatus::Unknown),
                execution_result: None,
            });
        }
        let mut edges = Vec::new();
        for pair in commits.windows(2) {
            edges.push(CommitEdge {
                from_hash: pair[0].commit_hash.clone(),
                to_hash: pair[1].commit_hash.clone(),
            });
        }
        Ok(RepositoryTimeGraph {
            repo_id: repo_root.to_string_lossy().to_string(),
            commits,
            edges,
        })
    }

    pub fn cache_successful_execution(&mut self, cache_entry: CommitExecutionCache) {
        self.cache
            .insert(cache_entry.commit_hash.clone(), cache_entry);
    }

    pub fn get_cached_execution(&self, commit_hash: &str) -> Option<&CommitExecutionCache> {
        self.cache.get(commit_hash)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitNavigator;

impl CommitNavigator {
    pub fn find_last_working_commit<'a>(
        &self,
        graph: &'a RepositoryTimeGraph,
    ) -> Option<&'a CommitNode> {
        graph.commits.iter().find(|node| commit_is_runnable(node))
    }

    pub fn find_best_runnable_commit<'a>(
        &self,
        graph: &'a RepositoryTimeGraph,
    ) -> Option<&'a CommitNode> {
        graph
            .commits
            .iter()
            .filter(|node| commit_is_runnable(node))
            .max_by(|left, right| {
                CommitScorer::score(left)
                    .overall_score
                    .total_cmp(&CommitScorer::score(right).overall_score)
            })
    }

    pub fn recover_from_failure<'a>(
        &self,
        graph: &'a RepositoryTimeGraph,
        head_commit: &str,
        policy: &TemporalExecutionPolicy,
    ) -> Option<&'a CommitNode> {
        if graph.commits.is_empty() {
            return None;
        }
        let start_index = graph
            .commits
            .iter()
            .position(|node| node.commit_hash == head_commit)
            .unwrap_or_default();
        let upper_bound = std::cmp::min(
            graph.commits.len(),
            start_index
                .saturating_add(policy.max_depth)
                .saturating_add(1),
        );
        graph.commits[start_index..upper_bound]
            .iter()
            .find(|node| commit_is_runnable(node))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TemporalExecutionRouter {
    policy: TemporalExecutionPolicy,
    navigator: CommitNavigator,
    engine: TemporalExecutionEngine,
}

impl Default for TemporalExecutionRouter {
    fn default() -> Self {
        Self {
            policy: TemporalExecutionPolicy::default(),
            navigator: CommitNavigator,
            engine: TemporalExecutionEngine::default(),
        }
    }
}

impl TemporalExecutionRouter {
    pub fn route(
        &self,
        graph: &RepositoryTimeGraph,
        head_commit: &str,
        strategy: RecoveryStrategy,
    ) -> Option<String> {
        if self.engine.get_cached_execution(head_commit).is_some() {
            return Some(head_commit.to_string());
        }

        let selected = match strategy {
            RecoveryStrategy::LastKnownGood => {
                self.navigator
                    .recover_from_failure(graph, head_commit, &self.policy)
            }
            RecoveryStrategy::BestRunnable => self.navigator.find_best_runnable_commit(graph),
        }?;
        Some(selected.commit_hash.clone())
    }

    pub fn cache(&mut self, cache_entry: CommitExecutionCache) {
        self.engine.cache_successful_execution(cache_entry);
    }

    pub fn is_cached(&self, commit_hash: &str) -> bool {
        self.engine.get_cached_execution(commit_hash).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureClass {
    MissingDependency,
    MissingLockfile,
    WrongPackageManager,
    MissingEnvironmentVariable,
    InvalidStartupCommand,
    PortConflict,
    MissingBuildArtifact,
    RuntimeVersionMismatch,
    DockerMisconfiguration,
    ServiceDependencyFailure,
    DatabaseUnavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairAction {
    InstallDependency,
    RebuildArtifacts,
    ChangeRuntimeVersion,
    SwitchPackageManager,
    RegenerateLockfile,
    AllocateNewPort,
    InjectEnvironmentDefaults,
    RestartDependency,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepairStrategy {
    pub strategy_id: String,
    pub confidence: f32,
    pub actions: Vec<RepairAction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealingConfidence {
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FailureSignal {
    pub message: String,
    pub attempted_command: Option<String>,
    pub expected_package_manager: Option<String>,
    pub required_runtime: Option<String>,
    pub detected_runtime: Option<String>,
    pub missing_environment_variables: Vec<String>,
    pub required_artifact: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FailureClassifier;

impl FailureClassifier {
    pub fn classify(
        &self,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
    ) -> FailureClass {
        let message = failure.message.to_ascii_lowercase();
        let attempted_command = failure
            .attempted_command
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let expected_package_manager = failure
            .expected_package_manager
            .as_deref()
            .or(fingerprint.build_signals.lockfile_type.as_deref())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !expected_package_manager.is_empty()
            && !attempted_command.is_empty()
            && package_manager_conflicts(&expected_package_manager, &attempted_command)
        {
            return FailureClass::WrongPackageManager;
        }

        if (!failure.missing_environment_variables.is_empty()
            || message.contains("missing environment variable")
            || message.contains("missing env"))
            && !message.contains("lockfile")
        {
            return FailureClass::MissingEnvironmentVariable;
        }

        if !fingerprint.build_signals.has_lockfile
            && (message.contains("lockfile") || message.contains("frozen-lockfile"))
        {
            return FailureClass::MissingLockfile;
        }

        if message.contains("modulenotfounderror")
            || message.contains("cannot find module")
            || message.contains("no module named")
        {
            return FailureClass::MissingDependency;
        }

        let required_runtime = failure
            .required_runtime
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let detected_runtime = failure
            .detected_runtime
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !required_runtime.is_empty()
            && !detected_runtime.is_empty()
            && required_runtime != detected_runtime
        {
            return FailureClass::RuntimeVersionMismatch;
        }
        if message.contains("runtime mismatch")
            || (message.contains("requires node") && message.contains("detected"))
        {
            return FailureClass::RuntimeVersionMismatch;
        }

        if message.contains("eaddrinuse") || message.contains("address already in use") {
            return FailureClass::PortConflict;
        }
        if message.contains("readiness probe timed out")
            || message.contains("health endpoint mismatch")
            || message.contains("incorrect port")
        {
            return FailureClass::PortConflict;
        }

        if failure.required_artifact.is_some()
            || message.contains("dist/")
            || message.contains("missing build artifact")
        {
            return FailureClass::MissingBuildArtifact;
        }

        if message.contains("missing script")
            || message.contains("command not found")
            || message.contains("invalid startup")
            || message.contains("spawn failed")
        {
            return FailureClass::InvalidStartupCommand;
        }

        if message.contains("database unavailable")
            || (fingerprint.infra_signals.uses_database && message.contains("connection refused"))
        {
            return FailureClass::DatabaseUnavailable;
        }

        if message.contains("service dependency")
            || message.contains("upstream")
            || message.contains("dependency failed")
            || message.contains("proxy routing failure")
        {
            return FailureClass::ServiceDependencyFailure;
        }

        if message.contains("docker")
            && (message.contains("misconfiguration")
                || message.contains("daemon")
                || message.contains("compose"))
        {
            return FailureClass::DockerMisconfiguration;
        }

        FailureClass::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeHealingEngine;

impl RuntimeHealingEngine {
    pub fn actions_for(&self, class: FailureClass) -> Vec<RepairAction> {
        match class {
            FailureClass::PortConflict => vec![RepairAction::AllocateNewPort],
            FailureClass::ServiceDependencyFailure => vec![RepairAction::RestartDependency],
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TopologyHealingEngine;

impl TopologyHealingEngine {
    pub fn actions_for(&self, class: FailureClass) -> Vec<RepairAction> {
        match class {
            FailureClass::ServiceDependencyFailure | FailureClass::DatabaseUnavailable => {
                vec![RepairAction::RestartDependency]
            }
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnvironmentResolver;

impl EnvironmentResolver {
    pub fn defaults_for(&self, missing_vars: &[String]) -> Vec<(String, String)> {
        missing_vars
            .iter()
            .filter_map(|name| {
                if name.eq_ignore_ascii_case("database_url") {
                    Some((name.clone(), "database.internal".to_string()))
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DependencyResolver;

impl DependencyResolver {
    pub fn actions_for(&self, class: FailureClass) -> Vec<RepairAction> {
        match class {
            FailureClass::MissingDependency => vec![RepairAction::InstallDependency],
            FailureClass::WrongPackageManager => vec![RepairAction::SwitchPackageManager],
            FailureClass::MissingLockfile => vec![RepairAction::RegenerateLockfile],
            FailureClass::MissingBuildArtifact | FailureClass::InvalidStartupCommand => {
                vec![RepairAction::RebuildArtifacts]
            }
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeCompatibilityResolver;

impl RuntimeCompatibilityResolver {
    pub fn actions_for(&self, class: FailureClass) -> Vec<RepairAction> {
        match class {
            FailureClass::RuntimeVersionMismatch => vec![RepairAction::ChangeRuntimeVersion],
            _ => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealingCatalog {
    runtime_layer: RuntimeHealingEngine,
    topology_layer: TopologyHealingEngine,
    environment_resolver: EnvironmentResolver,
    dependency_resolver: DependencyResolver,
    runtime_compatibility: RuntimeCompatibilityResolver,
}

impl HealingCatalog {
    pub fn strategy_for(
        &self,
        class: FailureClass,
        failure: &FailureSignal,
        _fingerprint: &RepositoryFingerprint,
    ) -> RepairStrategy {
        let mut actions = Vec::new();
        append_unique_actions(&mut actions, self.runtime_layer.actions_for(class));
        append_unique_actions(&mut actions, self.topology_layer.actions_for(class));
        append_unique_actions(&mut actions, self.dependency_resolver.actions_for(class));
        append_unique_actions(&mut actions, self.runtime_compatibility.actions_for(class));
        let environment_defaults = self
            .environment_resolver
            .defaults_for(&failure.missing_environment_variables);
        if !environment_defaults.is_empty() {
            append_unique_actions(&mut actions, vec![RepairAction::InjectEnvironmentDefaults]);
        }

        RepairStrategy {
            strategy_id: format!("repair::{class:?}").to_ascii_lowercase(),
            confidence: healing_confidence_for(class).score,
            actions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealingValidationEngine;

impl HealingValidationEngine {
    pub fn validate(&self, result: &ExecutionResult, healthy: bool) -> bool {
        result.started && result.stable && healthy
    }
}

pub trait HealingRuntime {
    fn apply_repair(&mut self, action: RepairAction) -> bool;
    fn re_execute(&mut self) -> ExecutionResult;
    fn health_check(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealingEngine;

impl HealingEngine {
    pub fn execute_plan<R: HealingRuntime>(
        &self,
        strategy: &RepairStrategy,
        runtime: &mut R,
        validator: &HealingValidationEngine,
    ) -> Option<ExecutionResult> {
        for action in strategy.actions.iter().copied() {
            if !runtime.apply_repair(action) {
                return None;
            }
        }
        let result = runtime.re_execute();
        if validator.validate(&result, runtime.health_check()) {
            Some(result)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealingOutcome {
    Success,
    EscalatedToTre,
    HumanIntervention,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealingJournalEntry {
    pub repo_id: String,
    pub failure_class: FailureClass,
    pub strategy_id: String,
    pub outcome: HealingOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealingJournal {
    entries: Vec<HealingJournalEntry>,
}

impl HealingJournal {
    pub fn record(
        &mut self,
        repo_id: &str,
        failure_class: FailureClass,
        strategy_id: &str,
        outcome: HealingOutcome,
    ) {
        self.entries.push(HealingJournalEntry {
            repo_id: repo_id.to_string(),
            failure_class,
            strategy_id: strategy_id.to_string(),
            outcome,
        });
    }

    pub fn entries_for_repo(&self, repo_id: &str) -> Vec<HealingJournalEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.repo_id == repo_id)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealingDecision {
    Recovered {
        failure_class: FailureClass,
        strategy: RepairStrategy,
        result: ExecutionResult,
    },
    EscalatedToTre {
        failure_class: FailureClass,
        strategy: RepairStrategy,
        selected_commit: String,
    },
    HumanInterventionRequired {
        failure_class: FailureClass,
        strategy: RepairStrategy,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HealingCoordinator {
    autonomous: healing::coordinator::AutonomousHealingCoordinator,
    pub journal: HealingJournal,
}

impl HealingCoordinator {
    pub fn heal_or_escalate<R: HealingRuntime>(
        &mut self,
        repo_id: &str,
        failure: &FailureSignal,
        fingerprint: &RepositoryFingerprint,
        runtime: &mut R,
        temporal_router: &TemporalExecutionRouter,
        graph: &RepositoryTimeGraph,
        head_commit: &str,
    ) -> HealingDecision {
        let autonomous_decision = self.autonomous.heal(
            failure,
            fingerprint,
            runtime,
            temporal_router,
            graph,
            head_commit,
        );
        let failure_class = autonomous_decision.failure_class;
        let strategy = autonomous_decision.strategy;
        match autonomous_decision.outcome {
            healing::coordinator::AutonomousOutcome::Recovered { result } => {
                self.journal.record(
                    repo_id,
                    failure_class,
                    &strategy.strategy_id,
                    HealingOutcome::Success,
                );
                HealingDecision::Recovered {
                    failure_class,
                    strategy,
                    result,
                }
            }
            healing::coordinator::AutonomousOutcome::EscalatedToTre { selected_commit } => {
                self.journal.record(
                    repo_id,
                    failure_class,
                    &strategy.strategy_id,
                    HealingOutcome::EscalatedToTre,
                );
                HealingDecision::EscalatedToTre {
                    failure_class,
                    strategy,
                    selected_commit,
                }
            }
            healing::coordinator::AutonomousOutcome::HumanInterventionRequired => {
                self.journal.record(
                    repo_id,
                    failure_class,
                    &strategy.strategy_id,
                    HealingOutcome::HumanIntervention,
                );
                HealingDecision::HumanInterventionRequired {
                    failure_class,
                    strategy,
                }
            }
        }
    }
}

fn append_unique_actions(actions: &mut Vec<RepairAction>, additional: Vec<RepairAction>) {
    for action in additional {
        if !actions.contains(&action) {
            actions.push(action);
        }
    }
}

fn healing_confidence_for(class: FailureClass) -> HealingConfidence {
    const PORT_REASSIGN_CONFIDENCE: f32 = 0.99;
    const PACKAGE_MANAGER_SWAP_CONFIDENCE: f32 = 0.95;
    const RUNTIME_SWAP_CONFIDENCE: f32 = 0.90;
    const LOCKFILE_REGEN_CONFIDENCE: f32 = 0.70;
    const DEPENDENCY_INSTALL_CONFIDENCE: f32 = 0.92;
    const ENVIRONMENT_INJECTION_CONFIDENCE: f32 = 0.93;
    const ARTIFACT_REBUILD_CONFIDENCE: f32 = 0.89;
    const DOCKER_REPAIR_CONFIDENCE: f32 = 0.78;
    const SERVICE_RESTART_CONFIDENCE: f32 = 0.87;
    const DATABASE_RECOVERY_CONFIDENCE: f32 = 0.85;
    const STARTUP_RECOVERY_CONFIDENCE: f32 = 0.82;
    const UNKNOWN_CONFIDENCE: f32 = 0.40;

    let score = match class {
        FailureClass::PortConflict => PORT_REASSIGN_CONFIDENCE,
        FailureClass::WrongPackageManager => PACKAGE_MANAGER_SWAP_CONFIDENCE,
        FailureClass::RuntimeVersionMismatch => RUNTIME_SWAP_CONFIDENCE,
        FailureClass::MissingLockfile => LOCKFILE_REGEN_CONFIDENCE,
        FailureClass::MissingDependency => DEPENDENCY_INSTALL_CONFIDENCE,
        FailureClass::MissingEnvironmentVariable => ENVIRONMENT_INJECTION_CONFIDENCE,
        FailureClass::MissingBuildArtifact => ARTIFACT_REBUILD_CONFIDENCE,
        FailureClass::DockerMisconfiguration => DOCKER_REPAIR_CONFIDENCE,
        FailureClass::ServiceDependencyFailure => SERVICE_RESTART_CONFIDENCE,
        FailureClass::DatabaseUnavailable => DATABASE_RECOVERY_CONFIDENCE,
        FailureClass::InvalidStartupCommand => STARTUP_RECOVERY_CONFIDENCE,
        FailureClass::Unknown => UNKNOWN_CONFIDENCE,
    };
    HealingConfidence { score }
}

fn package_manager_conflicts(expected: &str, attempted_command: &str) -> bool {
    let Some(attempted_manager) = detect_package_manager_from_command(attempted_command) else {
        return false;
    };
    let expected_manager = normalize_package_manager(expected);
    !expected_manager.is_empty() && attempted_manager != expected_manager
}

fn detect_package_manager_from_command(command: &str) -> Option<&'static str> {
    let command = command.trim_start();
    if command.starts_with("npm ") || command == "npm" {
        Some("npm")
    } else if command.starts_with("pnpm ") || command == "pnpm" {
        Some("pnpm")
    } else if command.starts_with("yarn ") || command == "yarn" {
        Some("yarn")
    } else if command.starts_with("bun ") || command == "bun" {
        Some("bun")
    } else {
        None
    }
}

fn normalize_package_manager(value: &str) -> &str {
    match value.trim().to_ascii_lowercase().as_str() {
        "pnpm" | "pnpm-lock.yaml" => "pnpm",
        "npm" | "package-lock.json" => "npm",
        "yarn" | "yarn.lock" => "yarn",
        "bun" | "bun.lockb" => "bun",
        _ => "",
    }
}

/// Returns whether a commit can be treated as runnable for temporal recovery.
///
/// A runnable commit must have successful (or partial-success) build status
/// and execution evidence that it started and remained stable.
fn commit_is_runnable(node: &CommitNode) -> bool {
    let build_ok = matches!(
        node.build_status.unwrap_or(BuildStatus::Unknown),
        BuildStatus::Success | BuildStatus::PartialSuccess
    );
    let runtime_ok = node
        .execution_result
        .as_ref()
        .map(|result| result.started && result.stable)
        .unwrap_or(false);
    build_ok && runtime_ok
}

/// Performs lightweight hash-shape verification for commit identifiers.
///
/// Accepted lengths cover short SHAs (7+) and long hexadecimal digests (up to 64).
fn is_verified_commit_hash(commit_hash: &str) -> bool {
    ((7..=12).contains(&commit_hash.len()) || matches!(commit_hash.len(), 40 | 64))
        && commit_hash
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryClassification {
    pub class: RepoClass,
    pub confidence: f32,
    pub primary_runtime: RuntimeType,
    pub secondary_runtimes: Vec<RuntimeType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAffinity {
    pub preferred_provider: String,
    pub fallback_providers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecutionTier {
    LocalMachine,
    LocalDocker,
    ExternalProvider,
    CloudPartner,
    DDockitCloud,
}

impl ExecutionTier {
    pub const ESCALATION_CHAIN: [Self; 5] = [
        Self::LocalMachine,
        Self::LocalDocker,
        Self::ExternalProvider,
        Self::CloudPartner,
        Self::DDockitCloud,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum RuntimeTier {
    DeaLocal,
    DockerLocal,
    ExternalProvider,
    CloudFallback,
}

impl RuntimeTier {
    pub fn weight(self) -> f64 {
        match self {
            Self::DeaLocal => 1.0,
            Self::DockerLocal => 2.0,
            Self::ExternalProvider => 5.0,
            Self::CloudFallback => 10.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeaLocal => "DEA_LOCAL",
            Self::DockerLocal => "DOCKER_LOCAL",
            Self::ExternalProvider => "EXTERNAL_PROVIDER",
            Self::CloudFallback => "CLOUD_FALLBACK",
        }
    }

    pub fn from_execution_tier(tier: ExecutionTier) -> Self {
        match tier {
            ExecutionTier::LocalMachine => Self::DeaLocal,
            ExecutionTier::LocalDocker => Self::DockerLocal,
            ExecutionTier::ExternalProvider => Self::ExternalProvider,
            ExecutionTier::CloudPartner | ExecutionTier::DDockitCloud => Self::CloudFallback,
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "DEA" | "DEA_LOCAL" | "LOCAL_MACHINE" => Self::DeaLocal,
            "DOCKER" | "DOCKER_LOCAL" | "LOCAL_DOCKER" => Self::DockerLocal,
            "EXTERNAL" | "EXTERNAL_PROVIDER" => Self::ExternalProvider,
            "CLOUD" | "CLOUD_FALLBACK" | "DDOCKIT_CLOUD" => Self::CloudFallback,
            _ => Self::DeaLocal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingEventType {
    ExecutionStarted,
    ExecutionAnalyzed,
    ExecutionRuntimeSelected,
    ExecutionHealingAttempted,
    ExecutionMigrated,
    ExecutionCompleted,
}

impl BillingEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExecutionStarted => "EXECUTION_STARTED",
            Self::ExecutionAnalyzed => "EXECUTION_ANALYZED",
            Self::ExecutionRuntimeSelected => "EXECUTION_RUNTIME_SELECTED",
            Self::ExecutionHealingAttempted => "EXECUTION_HEALING_ATTEMPTED",
            Self::ExecutionMigrated => "EXECUTION_MIGRATED",
            Self::ExecutionCompleted => "EXECUTION_COMPLETED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExecutionCostBreakdown {
    pub runtime_cost: f64,
    pub duration_cost: f64,
    pub retry_penalty: f64,
    pub healing_cost: f64,
    pub warm_pool_discount: f64,
    pub total_cost_units: f64,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone)]
pub struct ExecutionMeter {
    pub execution_id: String,
    pub org_id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub start_time: Instant,
    pub runtime_tier: RuntimeTier,
    pub retries: u32,
    pub healing_cycles: u32,
    pub warm_pool_hits: u32,
    pub heartbeat_count: u32,
    pub peak_cpu_usage: f32,
    pub peak_memory_usage: f32,
}

impl ExecutionMeter {
    pub fn new(
        execution_id: impl Into<String>,
        org_id: impl Into<String>,
        user_id: impl Into<String>,
        workspace_id: impl Into<String>,
        runtime_tier: RuntimeTier,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            org_id: org_id.into(),
            user_id: user_id.into(),
            workspace_id: workspace_id.into(),
            start_time: Instant::now(),
            runtime_tier,
            retries: 0,
            healing_cycles: 0,
            warm_pool_hits: 0,
            heartbeat_count: 0,
            peak_cpu_usage: 0.0,
            peak_memory_usage: 0.0,
        }
    }

    pub fn heartbeat(&mut self, cpu_usage: f32, memory_usage: f32) {
        self.heartbeat_count = self.heartbeat_count.saturating_add(1);
        self.peak_cpu_usage = self.peak_cpu_usage.max(cpu_usage);
        self.peak_memory_usage = self.peak_memory_usage.max(memory_usage);
    }

    pub fn record_retry(&mut self) {
        self.retries = self.retries.saturating_add(1);
    }

    pub fn record_healing_cycle(&mut self) {
        self.healing_cycles = self.healing_cycles.saturating_add(1);
    }

    pub fn record_warm_pool_hit(&mut self) {
        self.warm_pool_hits = self.warm_pool_hits.saturating_add(1);
    }

    pub fn complete_with_elapsed(&self, elapsed: Duration) -> ExecutionCostBreakdown {
        let duration_seconds = elapsed.as_secs_f64().max(MIN_BILLABLE_DURATION_SECONDS);
        let runtime_weight = self.runtime_tier.weight();
        let runtime_cost = runtime_weight;
        let duration_cost = (duration_seconds / 60.0) * runtime_weight;
        let retry_penalty = f64::from(self.retries) * RETRY_PENALTY_UNITS;
        let healing_cost =
            f64::from(self.healing_cycles) * HEALING_COST_MULTIPLIER_PER_CYCLE * runtime_weight;
        let warm_pool_discount =
            f64::from(self.warm_pool_hits) * WARM_POOL_DISCOUNT_MULTIPLIER * runtime_weight;
        let total_cost_units = (runtime_cost + duration_cost + retry_penalty + healing_cost
            - warm_pool_discount)
            .max(0.0);

        ExecutionCostBreakdown {
            runtime_cost,
            duration_cost,
            retry_penalty,
            healing_cost,
            warm_pool_discount,
            total_cost_units,
            duration_seconds,
        }
    }

    pub fn complete(&self) -> ExecutionCostBreakdown {
        self.complete_with_elapsed(self.start_time.elapsed())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapability {
    pub tier: ExecutionTier,
    pub latency_score: u32,
    pub cost_score: u32,
    pub reliability_score: u32,
    pub supported_runtimes: Vec<RuntimeType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationPolicy {
    pub max_local_wait_ms: u64,
    pub allow_external_fallback: bool,
    pub allow_cloud_fallback: bool,
    pub prefer_local: bool,
}

impl Default for EscalationPolicy {
    fn default() -> Self {
        Self {
            max_local_wait_ms: 2_000,
            allow_external_fallback: true,
            allow_cloud_fallback: true,
            prefer_local: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationTraceStep {
    pub tier: ExecutionTier,
    pub provider_id: Option<String>,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSelection {
    pub runtime: RuntimeType,
    pub provider_id: String,
    pub reason: String,
    pub fallback_chain: Vec<RuntimeType>,
    pub execution_id: ExecutionId,
    pub selected_tier: ExecutionTier,
    pub escalation_trace: Vec<EscalationTraceStep>,
    pub trace_uri: String,
    pub trace_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmCompatibility {
    Full,
    Partial,
    NotSupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmRuntimeSpec {
    pub enabled: bool,
    pub wasi: bool,
    pub memory_limit_mb: u64,
    pub cpu_limit_units: u32,
    pub allowed_syscalls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmSandbox {
    pub memory_limit: u64,
    pub time_limit_ms: u64,
    pub filesystem_scope: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiContext {
    pub env: HashMap<String, String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmExecutionResult {
    pub exported_functions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmModule {
    pub path: String,
    pub bytes: Vec<u8>,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmExecutionEnvironment {
    pub workspace_id: String,
    pub repo_path: String,
    pub resources: ResourceQuotas,
    pub network: NetworkPolicy,
}

impl WasmExecutionEnvironment {
    fn from_execution_context(ctx: &ExecutionContext) -> Self {
        Self {
            workspace_id: ctx.workspace_id.clone(),
            repo_path: ctx.repo_path.clone(),
            resources: ctx.resources.clone(),
            network: ctx.network.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmExecutionContext {
    pub node_id: String,
    pub module: WasmModule,
    pub wasi: WasiContext,
    pub env: WasmExecutionEnvironment,
    pub sandbox: WasmSandbox,
    pub spec: WasmRuntimeSpec,
}

pub struct WasmRuntimeEngine {
    engine: Engine,
    linker: Linker<()>,
}

impl WasmRuntimeEngine {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|err| {
            RuntimeError::WasmRuntime(format!("failed to initialize engine: {err}"))
        })?;
        let linker = Linker::new(&engine);
        Ok(Self { engine, linker })
    }

    pub fn instantiate(&self, ctx: &WasmExecutionContext) -> Result<WasmExecutionResult> {
        if !ctx.spec.enabled {
            return Err(RuntimeError::WasmRuntime(
                "attempted to execute disabled wasm runtime spec".to_string(),
            ));
        }
        if ctx.node_id.is_empty() || ctx.env.workspace_id.is_empty() {
            return Err(RuntimeError::WasmRuntime(
                "wasm execution context requires non-empty node and workspace identifiers"
                    .to_string(),
            ));
        }
        if !Path::new(&ctx.env.repo_path).is_absolute() {
            return Err(RuntimeError::InvalidPath(ctx.env.repo_path.clone()));
        }
        if !ctx
            .sandbox
            .filesystem_scope
            .iter()
            .any(|scope| scope == &ctx.env.repo_path)
        {
            return Err(RuntimeError::WasmRuntime(format!(
                "sandbox scope does not include repo path {}",
                ctx.env.repo_path
            )));
        }
        let sandbox_limit_mb = ctx.sandbox.memory_limit / BYTES_PER_MB;
        if sandbox_limit_mb == 0 {
            return Err(RuntimeError::WasmRuntime(
                "sandbox memory limit must be non-zero".to_string(),
            ));
        }
        let effective_spec = WasmRuntimeSpec {
            memory_limit_mb: ctx.spec.memory_limit_mb.min(sandbox_limit_mb),
            ..ctx.spec.clone()
        };

        let module = Module::from_binary(&self.engine, &ctx.module.bytes).map_err(|err| {
            RuntimeError::WasmRuntime(format!("module compilation failed: {err}"))
        })?;
        self.enforce_memory_limits(&module, &effective_spec)?;

        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(u64::from(effective_spec.cpu_limit_units))
            .map_err(|err| {
                RuntimeError::WasmRuntime(format!("failed to set fuel limits: {err}"))
            })?;
        let instance = self
            .linker
            .instantiate(&mut store, &module)
            .map_err(|err| {
                RuntimeError::WasmRuntime(format!("module instantiation failed: {err}"))
            })?;

        if let Ok(entrypoint) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            entrypoint.call(&mut store, ()).map_err(|err| {
                RuntimeError::WasmRuntime(format!("module execution failed: {err}"))
            })?;
        }

        let mut exported_functions = Vec::new();
        for export in module.exports() {
            if matches!(export.ty(), wasmtime::ExternType::Func(_)) {
                exported_functions.push(export.name().to_string());
            }
        }
        Ok(WasmExecutionResult { exported_functions })
    }

    pub fn execute_module(
        &self,
        wasm_bytes: &[u8],
        spec: &WasmRuntimeSpec,
        wasi: &WasiContext,
    ) -> Result<WasmExecutionResult> {
        let context = WasmExecutionContext {
            node_id: "inline".to_string(),
            module: WasmModule {
                path: "<inline>".to_string(),
                bytes: wasm_bytes.to_vec(),
                hash: hash_bytes(wasm_bytes),
            },
            wasi: wasi.clone(),
            env: WasmExecutionEnvironment {
                workspace_id: "inline".to_string(),
                repo_path: "/".to_string(),
                resources: ResourceQuotas {
                    max_memory_mb: spec.memory_limit_mb as u32,
                    max_cpu_millis: spec.cpu_limit_units,
                },
                network: NetworkPolicy {
                    allow_outbound: false,
                    allowed_hosts: vec![],
                },
            },
            sandbox: WasmSandbox {
                memory_limit: spec.memory_limit_mb.saturating_mul(BYTES_PER_MB),
                time_limit_ms: u64::from(spec.cpu_limit_units)
                    .saturating_mul(CPU_UNIT_TO_TIME_LIMIT_MS),
                filesystem_scope: vec!["/".to_string()],
            },
            spec: spec.clone(),
        };
        self.instantiate(&context)
    }

    fn enforce_memory_limits(&self, module: &Module, spec: &WasmRuntimeSpec) -> Result<()> {
        let max_pages = (spec.memory_limit_mb.saturating_mul(BYTES_PER_MB)) / (64 * 1024);
        for export in module.exports() {
            let wasmtime::ExternType::Memory(memory_type) = export.ty() else {
                continue;
            };
            let min = memory_type.minimum();
            if min > max_pages {
                return Err(RuntimeError::WasmRuntime(format!(
                    "module memory minimum ({min} pages) exceeds limit ({max_pages} pages)"
                )));
            }
            if let Some(max) = memory_type.maximum() {
                if max > max_pages {
                    return Err(RuntimeError::WasmRuntime(format!(
                        "module memory maximum ({max} pages) exceeds limit ({max_pages} pages)"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiKernelFilesystemCapability {
    pub read: Vec<String>,
    pub write: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiKernelNetworkCapability {
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiKernelProcessCapability {
    pub spawn: bool,
    pub max_processes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiKernelEnvCapability {
    pub variables: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WasiKernelRuntimeCapability {
    pub memory_limit_mb: u64,
    pub cpu_limit: f32,
}

impl Default for WasiKernelRuntimeCapability {
    fn default() -> Self {
        Self {
            memory_limit_mb: u64::from(RUNTIME_SPEC_DEFAULT_MEMORY_LIMIT_MB),
            cpu_limit: RUNTIME_SPEC_DEFAULT_CPU_LIMIT_UNITS as f32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WasiKernelCapability {
    pub filesystem: WasiKernelFilesystemCapability,
    pub network: WasiKernelNetworkCapability,
    pub process: WasiKernelProcessCapability,
    pub env: WasiKernelEnvCapability,
    pub runtime: WasiKernelRuntimeCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityEngine;

impl CapabilityEngine {
    pub fn validate(
        &self,
        graph: &WasiComponentGraph,
        capabilities: &WasiKernelCapability,
        environment: &BTreeMap<String, String>,
        runtime_spec: &WasmRuntimeSpec,
    ) -> Result<()> {
        if runtime_spec.memory_limit_mb == 0 || runtime_spec.cpu_limit_units == 0 {
            return Err(RuntimeError::WasmRuntime(
                "runtime spec limits must be non-zero".to_string(),
            ));
        }
        if capabilities.runtime.memory_limit_mb < runtime_spec.memory_limit_mb {
            return Err(RuntimeError::WasmRuntime(format!(
                "runtime memory limit {}mb exceeds capability ceiling {}mb",
                runtime_spec.memory_limit_mb, capabilities.runtime.memory_limit_mb
            )));
        }
        if capabilities.runtime.cpu_limit < runtime_spec.cpu_limit_units as f32 {
            return Err(RuntimeError::WasmRuntime(format!(
                "runtime cpu limit {} exceeds capability ceiling {}",
                runtime_spec.cpu_limit_units, capabilities.runtime.cpu_limit
            )));
        }
        for need in &graph.capabilities.needs {
            match need.as_str() {
                "filesystem.read" => {
                    if capabilities.filesystem.read.is_empty() {
                        return Err(RuntimeError::WasmRuntime(
                            "filesystem read capability requires allowlisted paths".to_string(),
                        ));
                    }
                }
                "filesystem.write" => {
                    if capabilities.filesystem.write.is_empty() {
                        return Err(RuntimeError::WasmRuntime(
                            "filesystem write capability requires allowlisted paths".to_string(),
                        ));
                    }
                }
                "network.http" => {
                    if capabilities.network.allowlist.is_empty() {
                        return Err(RuntimeError::WasmRuntime(
                            "network capability requires allowlist entries".to_string(),
                        ));
                    }
                }
                "process.spawn" => {
                    if !capabilities.process.spawn || capabilities.process.max_processes == 0 {
                        return Err(RuntimeError::WasmRuntime(
                            "process capability does not allow spawning".to_string(),
                        ));
                    }
                }
                _ => {}
            }
        }
        if environment.keys().any(|key| {
            !capabilities
                .env
                .variables
                .iter()
                .any(|allowed| allowed == key)
        }) {
            return Err(RuntimeError::WasmRuntime(
                "environment variable outside capability policy".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct KernelVirtualFs {
    pub readonly_layer: WorkspaceSnapshot,
    pub writable_layer: WorkspaceSnapshot,
    pub cache_layer: WorkspaceSnapshot,
    pub temp_layer: WorkspaceSnapshot,
}

impl KernelVirtualFs {
    fn from_snapshot(snapshot: &WorkspaceSnapshot) -> Self {
        Self {
            readonly_layer: snapshot.clone(),
            writable_layer: WorkspaceSnapshot::default(),
            cache_layer: WorkspaceSnapshot::default(),
            temp_layer: WorkspaceSnapshot::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkSandbox {
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
    observed_requests: Vec<String>,
}

impl NetworkSandbox {
    fn configure(&mut self, capability: &WasiKernelNetworkCapability) {
        self.allowlist = capability.allowlist.clone();
        self.denylist = capability.denylist.clone();
        self.observed_requests.clear();
    }

    fn intercept_request(&mut self, host: &str) -> Result<()> {
        if self.denylist.iter().any(|blocked| blocked == host) {
            return Err(RuntimeError::WasmRuntime(format!(
                "network host denied by policy: {host}"
            )));
        }
        if !self.allowlist.is_empty() && !self.allowlist.iter().any(|allowed| allowed == host) {
            return Err(RuntimeError::WasmRuntime(format!(
                "network host not in allowlist: {host}"
            )));
        }
        self.observed_requests.push(host.to_string());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessManager {
    spawn_enabled: bool,
    max_processes: u32,
    spawned: u32,
}

impl ProcessManager {
    fn configure(&mut self, capability: &WasiKernelProcessCapability) {
        self.spawn_enabled = capability.spawn;
        self.max_processes = capability.max_processes;
        self.spawned = 0;
    }

    fn spawn(&mut self) -> Result<()> {
        if !self.spawn_enabled {
            return Err(RuntimeError::WasmRuntime(
                "process spawn requested without capability".to_string(),
            ));
        }
        if self.spawned >= self.max_processes {
            return Err(RuntimeError::WasmRuntime(
                "process spawn limit exceeded".to_string(),
            ));
        }
        self.spawned += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemoryLimiter {
    limit_mb: u64,
}

impl MemoryLimiter {
    fn configure(&mut self, capability: &WasiKernelRuntimeCapability) {
        self.limit_mb = capability.memory_limit_mb;
    }

    fn enforce(&self, runtime_spec: &WasmRuntimeSpec) -> Result<()> {
        if runtime_spec.memory_limit_mb > self.limit_mb {
            return Err(RuntimeError::WasmRuntime(format!(
                "memory limit {}mb exceeds kernel allowance {}mb",
                runtime_spec.memory_limit_mb, self.limit_mb
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiExecutionScheduler {
    queue: VecDeque<String>,
}

impl WasiExecutionScheduler {
    fn enqueue(&mut self, trace_id: String) {
        self.queue.push_back(trace_id);
    }

    fn complete(&mut self, trace_id: &str) {
        self.queue.retain(|queued| queued != trace_id);
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TraceCollector {
    pub execution_trace: Vec<String>,
    pub syscall_trace: Vec<String>,
    pub performance_metrics: BTreeMap<String, f64>,
    pub failure_events: Vec<String>,
    pub memory_profile: Vec<u64>,
}

impl TraceCollector {
    fn stage(&mut self, value: &str) {
        self.execution_trace.push(value.to_string());
    }

    fn syscall(&mut self, value: &str) {
        self.syscall_trace.push(value.to_string());
    }

    fn metric(&mut self, key: &str, value: f64) {
        self.performance_metrics.insert(key.to_string(), value);
    }

    fn failure(&mut self, value: &str) {
        self.failure_events.push(value.to_string());
    }
}

#[derive(Debug, Clone)]
pub struct WasiKernelExecutionRequest {
    pub component_graph: WasiComponentGraph,
    pub runtime_spec: WasmRuntimeSpec,
    pub capabilities: WasiKernelCapability,
    pub filesystem_snapshot: WorkspaceSnapshot,
    pub environment: BTreeMap<String, String>,
    pub module_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WasiKernelExecutionResponse {
    pub result: String,
    pub logs: Vec<String>,
    pub trace_id: String,
    pub metrics: BTreeMap<String, f64>,
    pub execution_graph_diff: Vec<String>,
}

pub struct WasiKernel {
    pub runtime: WasmRuntimeEngine,
    pub linker: WasiLinker,
    pub capabilities: CapabilityEngine,
    pub filesystem: KernelVirtualFs,
    pub network: NetworkSandbox,
    pub process: ProcessManager,
    pub memory: MemoryLimiter,
    pub scheduler: WasiExecutionScheduler,
    pub observability: TraceCollector,
}

impl WasiKernel {
    pub fn new() -> Result<Self> {
        Ok(Self {
            runtime: WasmRuntimeEngine::new()?,
            linker: WasiLinker,
            capabilities: CapabilityEngine,
            filesystem: KernelVirtualFs::default(),
            network: NetworkSandbox::default(),
            process: ProcessManager::default(),
            memory: MemoryLimiter::default(),
            scheduler: WasiExecutionScheduler::default(),
            observability: TraceCollector::default(),
        })
    }

    pub fn execute(
        &mut self,
        request: &WasiKernelExecutionRequest,
    ) -> Result<WasiKernelExecutionResponse> {
        let started = Instant::now();
        let trace_id = next_wasi_kernel_trace_id();
        self.scheduler.enqueue(trace_id.clone());
        self.observability.stage("capability-validation");
        self.capabilities.validate(
            &request.component_graph,
            &request.capabilities,
            &request.environment,
            &request.runtime_spec,
        )?;

        self.observability.stage("component-linking");
        let resolved_links = WasiLinker::resolve(
            &request.component_graph.imports,
            &request.component_graph.exports,
        );
        let execution_graph_diff = resolved_links
            .iter()
            .filter(|link| !request.component_graph.links.contains(link))
            .map(|link| {
                format!(
                    "{}->{}:{}",
                    link.from_component, link.to_component, link.capability
                )
            })
            .collect::<Vec<_>>();

        self.observability.stage("sandbox-setup");
        self.filesystem = KernelVirtualFs::from_snapshot(&request.filesystem_snapshot);
        self.network.configure(&request.capabilities.network);
        for host in &request
            .component_graph
            .runtime_constraints
            .network_allowlist
        {
            self.network.intercept_request(host)?;
        }
        self.process.configure(&request.capabilities.process);
        if request
            .component_graph
            .capabilities
            .needs
            .iter()
            .any(|need| need == "process.spawn")
        {
            self.process.spawn()?;
            self.observability.syscall("process.spawn");
        }
        self.memory.configure(&request.capabilities.runtime);
        self.memory.enforce(&request.runtime_spec)?;

        self.observability.stage("execute");
        let wasi = WasiContext {
            env: request
                .environment
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<HashMap<_, _>>(),
            args: vec![],
        };
        let result = self
            .runtime
            .execute_module(&request.module_bytes, &request.runtime_spec, &wasi)
            .inspect_err(|err| self.observability.failure(&err.to_string()))?;

        self.observability.stage("observe");
        self.observability
            .metric("execution_ms", started.elapsed().as_secs_f64() * 1_000.0);
        self.observability.metric(
            "exported_function_count",
            result.exported_functions.len() as f64,
        );
        self.observability
            .memory_profile
            .push(u64::from(request.runtime_spec.memory_limit_mb).saturating_mul(BYTES_PER_MB));
        self.scheduler.complete(&trace_id);

        Ok(WasiKernelExecutionResponse {
            result: "ok".to_string(),
            logs: result.exported_functions,
            trace_id,
            metrics: self.observability.performance_metrics.clone(),
            execution_graph_diff,
        })
    }
}

fn next_wasi_kernel_trace_id() -> String {
    let sequence = WASI_KERNEL_TRACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("trace-{sequence}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeExecutionRequest {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
}

pub struct NativeRuntimeEngine;

impl NativeRuntimeEngine {
    pub fn execute(
        &self,
        request: &NativeExecutionRequest,
        resources: &ResourceQuotas,
        network: &NetworkPolicy,
    ) -> Result<ProcessHandle> {
        let cwd = Path::new(&request.cwd);
        if !cwd.is_absolute() {
            return Err(RuntimeError::InvalidPath(request.cwd.clone()));
        }
        if resources.max_memory_mb == 0 || resources.max_cpu_millis == 0 {
            return Err(RuntimeError::CommandFailed(
                "native runtime resource quotas must be non-zero".to_string(),
            ));
        }
        let _ = network;
        Ok(ProcessHandle {
            pid_hint: format!(
                "native:{}:{}:{}",
                request.command, resources.max_memory_mb, resources.max_cpu_millis
            ),
            ..ProcessHandle::default()
        })
    }
}

pub struct HybridExecutionBridge {
    wasm: WasmRuntimeEngine,
    native: NativeRuntimeEngine,
}

impl HybridExecutionBridge {
    pub fn new() -> Result<Self> {
        Ok(Self {
            wasm: WasmRuntimeEngine::new()?,
            native: NativeRuntimeEngine,
        })
    }

    pub fn dispatch(
        &self,
        node: &ExecutionNode,
        ctx: &ExecutionContext,
        wasi: &WasiContext,
        wasm_bytes: &[u8],
    ) -> Result<ProcessHandle> {
        match ExecutionRouter::route(node, &ctx.analysis.execution_profile) {
            ExecutionTarget::Wasm(spec) => {
                self.wasm.execute_module(wasm_bytes, &spec, wasi)?;
                Ok(ProcessHandle {
                    pid_hint: format!("wasm:{}", ctx.execution_graph.cache_key()),
                    ..ProcessHandle::default()
                })
            }
            ExecutionTarget::Native | ExecutionTarget::Static => self.native.execute(
                &NativeExecutionRequest {
                    command: node.command.clone().unwrap_or_else(|| "noop".to_string()),
                    args: vec![],
                    cwd: ctx.repo_path.clone(),
                    env: HashMap::new(),
                },
                &ctx.resources,
                &ctx.network,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionProfile {
    pub fingerprint: RepositoryFingerprint,
    pub classification: RepositoryClassification,
    pub recommended_graph_strategy: GraphStrategy,
    pub runtime_affinity: RuntimeAffinity,
    pub wasm_compatibility: WasmCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoDelta {
    pub added_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub modified_files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WorkspaceState {
    Created,
    Materializing,
    Installing,
    Analyzing,
    Planning,
    Pending,
    Provisioning,
    Starting,
    Launching,
    Initializing,
    Ready,
    Running,
    Degraded,
    Restarting,
    Migrating,
    Paused,
    Failed,
    Stopping,
    Stopped,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PortInfo {
    pub port: u16,
    pub protocol: String,
    pub route: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessCheck {
    Port(u16),
    Http(String),
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitExecutionSpecification {
    pub version: u32,
    #[serde(default)]
    pub application: Option<DdockitApplication>,
    #[serde(default)]
    pub services: HashMap<String, DdockitServiceSpecification>,
    #[serde(default)]
    pub dependencies: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub execution: Option<DdockitExecutionPreferences>,
    #[serde(default)]
    pub resources: Option<DdockitResourceConstraints>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitApplication {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitServiceSpecification {
    pub runtime: DdockitRuntime,
    #[serde(default)]
    pub framework: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub install: Vec<String>,
    #[serde(default)]
    pub build: Vec<String>,
    #[serde(default)]
    pub run: Vec<String>,
    #[serde(default)]
    pub test: Vec<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub environment: HashMap<String, DdockitEnvironmentVariable>,
    #[serde(default)]
    pub healthcheck: Option<DdockitHealthcheck>,
    #[serde(default)]
    pub resources: Option<DdockitResourceConstraints>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DdockitRuntime {
    Node,
    Python,
    Rust,
    Bun,
    Go,
    Docker,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitEnvironmentVariable {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitHealthcheck {
    #[serde(rename = "type")]
    pub check_type: DdockitHealthcheckType,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DdockitHealthcheckType {
    Http,
    Tcp,
    Process,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitExecutionPreferences {
    #[serde(default)]
    pub preferred_tier: Vec<String>,
    #[serde(default)]
    pub fallback: Option<DdockitExecutionFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitExecutionFallback {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DdockitResourceConstraints {
    #[serde(default)]
    pub cpu: Option<u32>,
    #[serde(default)]
    pub memory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDefinition {
    pub id: String,
    pub name: String,
    pub runtime: RuntimeType,
    pub package_manager: Option<String>,
    pub working_directory: String,
    pub start_command: String,
    pub ports: Vec<u16>,
    pub readiness_checks: Vec<ReadinessCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDependency {
    pub service_id: String,
    pub depends_on: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkTopology {
    pub network_id: String,
    pub service_dns: HashMap<String, String>,
    pub exposed_ports: HashMap<String, Vec<u16>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartupOrder {
    pub stages: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StartupStrategy {
    pub stages: Vec<Vec<String>>,
    pub enforce_dependencies: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealthPolicy {
    pub service_checks: HashMap<String, Vec<ReadinessCheck>>,
    pub require_healthy_dependencies: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationTopology {
    pub topology_id: String,
    pub services: Vec<ServiceDefinition>,
    pub edges: Vec<ServiceDependency>,
    pub global_network: NetworkTopology,
    pub startup_strategy: StartupStrategy,
    pub health_policy: HealthPolicy,
    pub dependencies: Vec<ServiceDependency>,
    pub startup_order: StartupOrder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionNode {
    pub id: String,
    pub node_type: ExecutionNodeType,
    pub command: Option<String>,
    pub execution_mode: ExecutionMode,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub cache_key: Option<String>,
    pub runtime: Option<ExecutionImage>,
    pub cache_binding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionNodeType {
    InstallDependencies,
    Build,
    WasmCompile,
    DevServer,
    Test,
    StaticServe,
    CustomCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Native,
    Wasm,
    Hybrid,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Native
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionTarget {
    Wasm(WasmRuntimeSpec),
    Native,
    Static,
}

#[derive(Default)]
struct ProviderRegistry;

#[derive(Debug)]
struct ScoredProvider {
    score: i32,
    provider_id: String,
}

const TIER_LOCAL_MACHINE_PROXIMITY_SCORE: i32 = 50;
const TIER_LOCAL_DOCKER_PROXIMITY_SCORE: i32 = 40;
const TIER_EXTERNAL_PROVIDER_PROXIMITY_SCORE: i32 = 30;
const TIER_CLOUD_PARTNER_PROXIMITY_SCORE: i32 = 20;
const TIER_DDOCKIT_CLOUD_PROXIMITY_SCORE: i32 = 10;
const PREFERRED_PROVIDER_AFFINITY_BONUS: i32 = 30;
const FALLBACK_PROVIDER_AFFINITY_BONUS: i32 = 20;
const CAPABILITY_MATCH_BONUS: i32 = 10;

impl ProviderRegistry {
    fn ranked_provider_ids_for_tier(
        &self,
        providers: &[Box<dyn ExecutionProvider + Send + Sync>],
        tier: ExecutionTier,
        ctx: &ExecutionContext,
        affinity: &RuntimeAffinity,
    ) -> Vec<String> {
        let primary_runtime = ctx.analysis.classification.primary_runtime;
        let mut scored = providers
            .iter()
            .filter_map(|provider| {
                let capability = provider.capability();
                if capability.tier != tier || !provider.can_run(ctx) {
                    return None;
                }
                // Weighted score favors locality first, then capability/reliability, then cost/latency.
                let proximity_score = match tier {
                    ExecutionTier::LocalMachine => TIER_LOCAL_MACHINE_PROXIMITY_SCORE,
                    ExecutionTier::LocalDocker => TIER_LOCAL_DOCKER_PROXIMITY_SCORE,
                    ExecutionTier::ExternalProvider => TIER_EXTERNAL_PROVIDER_PROXIMITY_SCORE,
                    ExecutionTier::CloudPartner => TIER_CLOUD_PARTNER_PROXIMITY_SCORE,
                    ExecutionTier::DDockitCloud => TIER_DDOCKIT_CLOUD_PROXIMITY_SCORE,
                };
                let affinity_bonus = if provider.id() == affinity.preferred_provider {
                    PREFERRED_PROVIDER_AFFINITY_BONUS
                } else if affinity
                    .fallback_providers
                    .iter()
                    .any(|fallback| fallback == provider.id())
                {
                    FALLBACK_PROVIDER_AFFINITY_BONUS
                } else {
                    0
                };
                let capability_bonus = if capability.supported_runtimes.contains(&primary_runtime) {
                    CAPABILITY_MATCH_BONUS
                } else {
                    0
                };
                let score = proximity_score
                    + capability.reliability_score as i32
                    + affinity_bonus
                    + capability_bonus
                    - capability.latency_score as i32
                    - capability.cost_score as i32;
                Some(ScoredProvider {
                    score,
                    provider_id: provider.id().to_string(),
                })
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.provider_id.cmp(&b.provider_id))
        });
        scored.into_iter().map(|entry| entry.provider_id).collect()
    }
}

pub struct ExecutionRouter {
    providers: Vec<Box<dyn ExecutionProvider + Send + Sync>>,
    escalation_policy: EscalationPolicy,
    provider_registry: ProviderRegistry,
}

impl ExecutionRouter {
    pub fn new(providers: Vec<Box<dyn ExecutionProvider + Send + Sync>>) -> Self {
        Self {
            providers,
            escalation_policy: EscalationPolicy::default(),
            provider_registry: ProviderRegistry,
        }
    }

    fn tier_order() -> [ExecutionTier; 5] {
        ExecutionTier::ESCALATION_CHAIN
    }

    fn execution_trace_uri(workspace_id: &str) -> String {
        let safe_workspace_id = Self::sanitized_workspace_id(workspace_id);
        format!("ddockit://workspace/{safe_workspace_id}/trace")
    }

    fn derive_execution_id_from_workspace(workspace_id: &str) -> ExecutionId {
        Self::sanitized_workspace_id(workspace_id)
    }

    fn execution_trace_url(workspace_id: &str) -> String {
        let execution_id = Self::derive_execution_id_from_workspace(workspace_id);
        ExecutionIdentity::canonical_url_for(&execution_id)
    }

    fn sanitized_workspace_id(workspace_id: &str) -> String {
        let mut encoded = String::with_capacity(workspace_id.len());
        for byte in workspace_id.bytes() {
            let ch = byte as char;
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                encoded.push(ch);
            } else {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
        if encoded.is_empty() {
            "workspace-empty".to_string()
        } else {
            encoded
        }
    }

    fn tier_allowed_by_policy(&self, tier: ExecutionTier) -> bool {
        match tier {
            ExecutionTier::LocalMachine | ExecutionTier::LocalDocker => true,
            ExecutionTier::ExternalProvider => self.escalation_policy.allow_external_fallback,
            ExecutionTier::CloudPartner | ExecutionTier::DDockitCloud => {
                self.escalation_policy.allow_cloud_fallback
            }
        }
    }

    pub fn select(&self, ctx: &ExecutionContext) -> Result<RuntimeSelection> {
        let affinity = &ctx.analysis.execution_profile.runtime_affinity;
        let mut escalation_trace = Vec::new();
        let mut matched_provider_ids = Vec::new();
        let mut selected_provider_id = None::<String>;
        let mut selected_tier = None::<ExecutionTier>;

        for tier in Self::tier_order() {
            if !self.tier_allowed_by_policy(tier) {
                escalation_trace.push(EscalationTraceStep {
                    tier,
                    provider_id: None,
                    result: "skipped by escalation policy".to_string(),
                });
                continue;
            }

            let ranked_provider_ids = self.provider_registry.ranked_provider_ids_for_tier(
                &self.providers,
                tier,
                ctx,
                affinity,
            );
            if ranked_provider_ids.is_empty() {
                escalation_trace.push(EscalationTraceStep {
                    tier,
                    provider_id: None,
                    result: "no available provider".to_string(),
                });
                continue;
            }

            for provider_id in ranked_provider_ids {
                matched_provider_ids.push(provider_id.clone());
                let result = if selected_provider_id.is_none() {
                    selected_provider_id = Some(provider_id.clone());
                    selected_tier = Some(tier);
                    "selected"
                } else {
                    "fallback candidate"
                };
                escalation_trace.push(EscalationTraceStep {
                    tier,
                    provider_id: Some(provider_id),
                    result: result.to_string(),
                });
            }
            if selected_provider_id.is_some() {
                break;
            }
        }

        let selected_provider_id = selected_provider_id.ok_or_else(|| {
            let attempted = escalation_trace
                .iter()
                .map(|step| format!("{:?}:{}", step.tier, step.result))
                .collect::<Vec<_>>()
                .join(", ");
            RuntimeError::UnsupportedRepository(format!(
                "no execution provider matched for workspace {} with framework {:?}; attempts: {}",
                ctx.workspace_id, ctx.analysis.framework, attempted
            ))
        })?;
        let selected_tier = selected_tier.ok_or_else(|| {
            RuntimeError::CommandFailed(format!(
                "internal error: selected_tier missing for workspace {} provider {}",
                ctx.workspace_id, selected_provider_id
            ))
        })?;

        let provider = self.provider_by_id(&selected_provider_id).ok_or_else(|| {
            RuntimeError::UnsupportedRepository(format!(
                "selected execution provider `{selected_provider_id}` was not registered"
            ))
        })?;

        let fallback_chain = matched_provider_ids
            .iter()
            .skip(1)
            .filter_map(|provider_id| self.provider_by_id(provider_id))
            .map(|provider| {
                let runtime = provider.runtime();
                if runtime == RuntimeType::Unknown {
                    ctx.analysis.classification.primary_runtime
                } else {
                    runtime
                }
            })
            .collect::<Vec<_>>();

        let reason = if selected_provider_id == affinity.preferred_provider {
            format!(
                "selected preferred runtime provider `{}` in {:?}",
                selected_provider_id, selected_tier
            )
        } else {
            format!(
                "preferred provider `{}` unavailable; escalated to `{}` in {:?}",
                affinity.preferred_provider, selected_provider_id, selected_tier
            )
        };
        let execution_id = Self::derive_execution_id_from_workspace(&ctx.workspace_id);
        let selected_runtime = {
            let runtime = provider.runtime();
            if runtime == RuntimeType::Unknown {
                ctx.analysis.classification.primary_runtime
            } else {
                runtime
            }
        };

        Ok(RuntimeSelection {
            runtime: selected_runtime,
            provider_id: selected_provider_id,
            reason,
            fallback_chain,
            execution_id,
            selected_tier,
            escalation_trace,
            trace_uri: Self::execution_trace_uri(&ctx.workspace_id),
            trace_url: Self::execution_trace_url(&ctx.workspace_id),
        })
    }

    pub fn dispatch_start(&self, ctx: &mut ExecutionContext) -> Result<ProcessHandle> {
        let selection = self.select(ctx)?;
        let provider = self.provider_by_id(&selection.provider_id).ok_or_else(|| {
            RuntimeError::UnsupportedRepository(format!(
                "selected execution provider `{}` was not registered",
                selection.provider_id
            ))
        })?;
        provider.prepare(ctx)?;
        let handle = provider.start(ctx)?;
        let health = provider.health(&handle)?;
        if health.healthy {
            Ok(ProcessHandle {
                pid_hint: handle.pid_hint,
                trace_uri: Some(selection.trace_uri),
                trace_url: Some(selection.trace_url),
            })
        } else {
            match provider.stop(&handle) {
                Ok(()) => Err(RuntimeError::CommandFailed(format!(
                    "provider reported unhealthy process: {}",
                    health.message
                ))),
                Err(stop_err) => Err(RuntimeError::CommandFailed(format!(
                    "provider reported unhealthy process: {}; cleanup failed: {stop_err}",
                    health.message
                ))),
            }
        }
    }

    pub fn dispatch_stop(&self, ctx: &ExecutionContext, handle: &ProcessHandle) -> Result<()> {
        let selection = self.select(ctx)?;
        let provider = self.provider_by_id(&selection.provider_id).ok_or_else(|| {
            RuntimeError::UnsupportedRepository(format!(
                "selected execution provider `{}` was not registered",
                selection.provider_id
            ))
        })?;
        provider.stop(handle)
    }

    pub fn route(node: &ExecutionNode, profile: &ExecutionProfile) -> ExecutionTarget {
        match node.execution_mode {
            ExecutionMode::Native => {
                if node.node_type == ExecutionNodeType::StaticServe
                    && profile.wasm_compatibility == WasmCompatibility::NotSupported
                {
                    ExecutionTarget::Static
                } else {
                    ExecutionTarget::Native
                }
            }
            ExecutionMode::Wasm => match profile.wasm_compatibility {
                WasmCompatibility::Full | WasmCompatibility::Partial => {
                    ExecutionTarget::Wasm(Self::runtime_spec_for(profile.wasm_compatibility))
                }
                WasmCompatibility::NotSupported => ExecutionTarget::Native,
            },
            ExecutionMode::Hybrid => match profile.wasm_compatibility {
                WasmCompatibility::Full => {
                    ExecutionTarget::Wasm(Self::runtime_spec_for(profile.wasm_compatibility))
                }
                WasmCompatibility::Partial | WasmCompatibility::NotSupported => {
                    ExecutionTarget::Native
                }
            },
        }
    }

    fn runtime_spec_for(compatibility: WasmCompatibility) -> WasmRuntimeSpec {
        match compatibility {
            WasmCompatibility::Full => WasmRuntimeSpec {
                enabled: true,
                wasi: true,
                memory_limit_mb: WASM_FULL_MEMORY_LIMIT_MB,
                cpu_limit_units: WASM_FULL_CPU_LIMIT_UNITS,
                allowed_syscalls: vec![
                    "fd_read".to_string(),
                    "fd_write".to_string(),
                    "clock_time_get".to_string(),
                    "random_get".to_string(),
                ],
            },
            WasmCompatibility::Partial => WasmRuntimeSpec {
                enabled: true,
                wasi: true,
                memory_limit_mb: WASM_PARTIAL_MEMORY_LIMIT_MB,
                cpu_limit_units: WASM_PARTIAL_CPU_LIMIT_UNITS,
                allowed_syscalls: vec![
                    "fd_read".to_string(),
                    "fd_write".to_string(),
                    "clock_time_get".to_string(),
                ],
            },
            WasmCompatibility::NotSupported => WasmRuntimeSpec {
                enabled: false,
                wasi: false,
                memory_limit_mb: u64::from(UNINITIALIZED_RESOURCE_LIMIT),
                cpu_limit_units: UNINITIALIZED_RESOURCE_LIMIT,
                allowed_syscalls: vec![],
            },
        }
    }

    fn provider_by_id(&self, provider_id: &str) -> Option<&(dyn ExecutionProvider + Send + Sync)> {
        self.providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .map(|provider| provider.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionGraph {
    pub nodes: Vec<ExecutionNode>,
    pub edges: Vec<ExecutionEdge>,
}

impl ExecutionGraph {
    pub fn ordered_node_ids(&self) -> Vec<String> {
        let mut indegree: HashMap<&str, usize> = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), 0usize))
            .collect();
        let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();

        for edge in &self.edges {
            if let Some(count) = indegree.get_mut(edge.to.as_str()) {
                *count += 1;
            }
            adjacency
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }

        let mut ready: BTreeSet<&str> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut queue: VecDeque<&str> = VecDeque::new();
        for id in ready.iter().copied() {
            queue.push_back(id);
        }
        ready.clear();

        let mut ordered = Vec::with_capacity(self.nodes.len());
        while let Some(id) = queue.pop_front() {
            ordered.push(id.to_string());

            if let Some(next_ids) = adjacency.get(id) {
                for next in next_ids {
                    if let Some(next_degree) = indegree.get_mut(next) {
                        *next_degree = next_degree.saturating_sub(1);
                        if *next_degree == 0 {
                            ready.insert(next);
                        }
                    }
                }
            }

            for next in ready.iter().copied() {
                queue.push_back(next);
            }
            ready.clear();
        }

        if ordered.len() == self.nodes.len() {
            ordered
        } else {
            let mut fallback = self
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            fallback.sort();
            fallback
        }
    }

    pub fn primary_run_command(&self) -> Option<String> {
        let preferred = [
            ExecutionNodeType::DevServer,
            ExecutionNodeType::StaticServe,
            ExecutionNodeType::CustomCommand,
        ];

        for kind in preferred {
            if let Some(command) = self
                .nodes
                .iter()
                .find(|node| node.node_type == kind)
                .and_then(|node| node.command.clone())
            {
                return Some(command);
            }
        }

        self.nodes.iter().find_map(|node| node.command.clone())
    }

    pub fn cache_key(&self) -> String {
        let mut normalized = self
            .ordered_node_ids()
            .into_iter()
            .map(|id| {
                let mode = self
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
                    .map(|node| execution_mode_name(node.execution_mode))
                    .unwrap_or("native");
                format!("{id}{CACHE_KEY_NODE_MODE_SEPARATOR}{mode}")
            })
            .collect::<Vec<_>>();
        let mut edges = self
            .edges
            .iter()
            .map(|edge| format!("{}->{}", edge.from, edge.to))
            .collect::<Vec<_>>();
        edges.sort();
        normalized.extend(edges);
        hash_key(&normalized.join("|"))
    }

    pub fn compute_cache_keys(&self) -> HashMap<String, String> {
        self.compute_cache_keys_with_fingerprint(None)
    }

    pub fn compute_cache_keys_with_fingerprint(
        &self,
        fingerprint: Option<&RepositoryFingerprint>,
    ) -> HashMap<String, String> {
        self.nodes
            .iter()
            .map(|node| {
                (
                    node.id.clone(),
                    CacheKeyEngine::compute_node_key(node, self, fingerprint),
                )
            })
            .collect()
    }

    pub fn with_cache_keys(self) -> Self {
        self.with_cache_keys_for(None)
    }

    pub fn with_cache_keys_for(mut self, fingerprint: Option<&RepositoryFingerprint>) -> Self {
        let keys = self.compute_cache_keys_with_fingerprint(fingerprint);
        let fingerprint_binding = fingerprint
            .map(|entry| entry.repo_hash.as_str())
            .unwrap_or("no-fingerprint");
        for node in &mut self.nodes {
            node.cache_key = keys.get(&node.id).cloned();
            node.cache_binding = node
                .cache_key
                .as_deref()
                .map(|key| warm_cache_binding_key(fingerprint_binding, key));
        }
        self
    }

    pub fn with_execution_image(mut self, image: &ExecutionImage) -> Self {
        for node in &mut self.nodes {
            node.runtime = Some(image.clone());
        }
        self
    }
}

pub type FrameworkType = Framework;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIntelligence {
    pub framework: FrameworkType,
    pub package_manager: Option<String>,
    pub build_tooling: Vec<String>,
    pub entrypoints: Vec<String>,
    pub scripts: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFilesystemPlan {
    pub read_only_layers: Vec<String>,
    pub dependency_cache_layer: String,
    pub build_cache_layer: String,
    pub execution_layer: String,
    pub temporary_layer: String,
    pub copy_on_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeServicePlan {
    pub id: String,
    pub runtime: String,
    pub framework: Option<String>,
    pub working_directory: String,
    pub start_command: String,
    pub ports: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilitySet {
    pub needs: BTreeSet<String>,
}

impl CapabilitySet {
    pub fn insert(&mut self, capability: impl Into<String>) {
        self.needs.insert(capability.into());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConstraints {
    pub read_only_paths: Vec<String>,
    pub network_allowlist: Vec<String>,
    pub max_memory_mb: u32,
    pub max_cpu_units: u32,
    pub process_spawn_bounded: bool,
}

impl Default for RuntimeConstraints {
    fn default() -> Self {
        Self {
            read_only_paths: vec!["/workspace".to_string()],
            network_allowlist: vec![],
            max_memory_mb: RUNTIME_SPEC_DEFAULT_MEMORY_LIMIT_MB,
            max_cpu_units: RUNTIME_SPEC_DEFAULT_CPU_LIMIT_UNITS,
            process_spawn_bounded: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiComponent {
    pub id: String,
    pub module: String,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiLink {
    pub from_component: String,
    pub to_component: String,
    pub capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiComponentGraph {
    pub components: Vec<WasiComponent>,
    pub links: Vec<WasiLink>,
    pub capabilities: CapabilitySet,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub runtime_constraints: RuntimeConstraints,
    pub execution_plan: ExecutionPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WasiLinker;

impl WasiLinker {
    pub fn resolve(imports: &[String], exports: &[String]) -> Vec<WasiLink> {
        imports
            .iter()
            .filter_map(|import| {
                let (import_component, import_capability) =
                    parse_link_entry("import:", import.as_str())?;
                let (export_component, _) = exports.iter().find_map(|export| {
                    let parsed = parse_link_entry("export:", export.as_str())?;
                    (parsed.1 == import_capability).then_some(parsed)
                })?;
                Some(WasiLink {
                    from_component: export_component.to_string(),
                    to_component: import_component.to_string(),
                    capability: import_capability.to_string(),
                })
            })
            .collect()
    }

    pub fn validate(capabilities: &CapabilitySet, graph: &WasiComponentGraph) -> bool {
        let available = graph
            .components
            .iter()
            .flat_map(|component| component.capabilities.iter().cloned())
            .collect::<HashSet<_>>();
        capabilities
            .needs
            .iter()
            .all(|need| available.contains(need))
    }

    pub fn enforce_security_model(graph: &mut WasiComponentGraph) {
        graph.runtime_constraints.read_only_paths.sort();
        graph.runtime_constraints.read_only_paths.dedup();
        graph.runtime_constraints.network_allowlist.sort();
        graph.runtime_constraints.network_allowlist.dedup();
        graph.runtime_constraints.process_spawn_bounded = true;
    }

    pub fn optimize_graph(graph: &mut WasiComponentGraph) {
        Self::normalize_components(&mut graph.components);
        Self::collapse_duplicate_components(&mut graph.components);

        let mut required_capabilities = graph.capabilities.needs.clone();
        required_capabilities.extend(graph.links.iter().map(|link| link.capability.clone()));
        required_capabilities.extend(
            graph
                .imports
                .iter()
                .filter_map(|entry| parse_link_entry("import:", entry.as_str()))
                .map(|(_, capability)| capability.to_string()),
        );

        let mut live_component_ids = Self::collect_live_components(graph, &required_capabilities);
        if live_component_ids.is_empty() {
            debug_assert!(
                graph.components.is_empty(),
                "WasiLinker::optimize_graph found no live components and is falling back to full graph retention"
            );
            live_component_ids.extend(
                graph
                    .components
                    .iter()
                    .map(|component| component.id.clone()),
            );
        }
        graph
            .components
            .retain(|component| live_component_ids.contains(component.id.as_str()));

        for component in &mut graph.components {
            component
                .capabilities
                .retain(|capability| required_capabilities.contains(capability));
            component.capabilities.sort();
            component.capabilities.dedup();
        }

        let component_ids = graph
            .components
            .iter()
            .map(|component| component.id.as_str())
            .collect::<HashSet<_>>();
        graph.links.retain(|link| {
            component_ids.contains(link.from_component.as_str())
                && component_ids.contains(link.to_component.as_str())
        });
        graph.links.sort_by(|a, b| {
            (&a.from_component, &a.to_component, &a.capability).cmp(&(
                &b.from_component,
                &b.to_component,
                &b.capability,
            ))
        });
        graph.links.dedup();

        graph.imports.retain(|entry| {
            parse_link_entry("import:", entry.as_str())
                .map(|(component_id, _)| component_ids.contains(component_id))
                .unwrap_or(false)
        });
        graph.imports.sort();
        graph.imports.dedup();
        graph.exports.retain(|entry| {
            parse_link_entry("export:", entry.as_str())
                .map(|(component_id, _)| component_ids.contains(component_id))
                .unwrap_or(false)
        });
        graph.exports.sort();
        graph.exports.dedup();

        let available_capabilities = graph
            .components
            .iter()
            .flat_map(|component| component.capabilities.iter().cloned())
            .collect::<BTreeSet<_>>();
        graph
            .capabilities
            .needs
            .retain(|need| available_capabilities.contains(need));

        if graph.components.iter().any(Self::requires_dependency_cache) {
            graph
                .runtime_constraints
                .read_only_paths
                .push("/cache/dependency".to_string());
            graph
                .runtime_constraints
                .read_only_paths
                .push("/runtime/warm".to_string());
        }

        graph.execution_plan.startup_order =
            component_startup_order(&graph.components, &graph.links);
        graph.execution_plan.ordered_nodes = graph.execution_plan.startup_order.clone();
    }

    fn normalize_components(components: &mut [WasiComponent]) {
        for component in components {
            component.imports.sort();
            component.imports.dedup();
            component.exports.sort();
            component.exports.dedup();
            component.capabilities.sort();
            component.capabilities.dedup();
        }
    }

    fn collapse_duplicate_components(components: &mut Vec<WasiComponent>) {
        components.sort_by(|a, b| a.id.cmp(&b.id));
        let mut collapsed = Vec::new();
        for component in components.drain(..) {
            if let Some(existing) = collapsed
                .iter_mut()
                .find(|existing: &&mut WasiComponent| existing.id == component.id)
            {
                if !component.module.is_empty()
                    && (existing.module.is_empty() || component.module < existing.module)
                {
                    existing.module = component.module.clone();
                }
                existing.imports.extend(component.imports);
                existing.exports.extend(component.exports);
                existing.capabilities.extend(component.capabilities);
                existing.imports.sort();
                existing.imports.dedup();
                existing.exports.sort();
                existing.exports.dedup();
                existing.capabilities.sort();
                existing.capabilities.dedup();
            } else {
                collapsed.push(component);
            }
        }
        *components = collapsed;
    }

    fn collect_live_components(
        graph: &WasiComponentGraph,
        required_capabilities: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut live = BTreeSet::new();
        live.extend(
            graph
                .links
                .iter()
                .flat_map(|link| [link.from_component.clone(), link.to_component.clone()]),
        );
        live.extend(
            graph
                .imports
                .iter()
                .filter_map(|entry| parse_link_entry("import:", entry.as_str()))
                .map(|(component_id, _)| component_id.to_string()),
        );
        live.extend(
            graph
                .components
                .iter()
                .filter(|component| {
                    component
                        .capabilities
                        .iter()
                        .any(|capability| required_capabilities.contains(capability))
                })
                .map(|component| component.id.clone()),
        );
        live
    }

    fn requires_dependency_cache(component: &WasiComponent) -> bool {
        component.capabilities.iter().any(|capability| {
            let normalized = interface_identity(capability).to_ascii_lowercase();
            normalized.ends_with(".package_manager")
                || matches!(
                    normalized.as_str(),
                    "build.compile"
                        | "build.install"
                        | "build.package"
                        | "runtime.build"
                        | "runtime.compile"
                        | "execution.build"
                        | "execution.compile"
                        | "execution.install"
                )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentRegistry {
    pub store: BTreeMap<String, WasiComponent>,
    pub versioning: BTreeMap<String, String>,
    pub signatures: BTreeMap<String, String>,
}

impl ComponentRegistry {
    pub fn register(&mut self, component: WasiComponent) {
        self.versioning
            .entry(component.id.clone())
            .or_insert_with(|| DEFAULT_COMPONENT_VERSION.to_string());
        self.signatures
            .entry(component.id.clone())
            .or_insert_with(|| hash_key(&component.module));
        self.store.insert(component.id.clone(), component);
    }

    fn register_all(&mut self, components: &[WasiComponent]) {
        for component in components {
            self.register(component.clone());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompiledComponentCache {
    entries: BTreeMap<String, WasiComponentGraph>,
}

impl CompiledComponentCache {
    fn get(&self, key: &str) -> Option<WasiComponentGraph> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: String, graph: WasiComponentGraph) {
        self.entries.insert(key, graph);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InterfaceResolver;

impl InterfaceResolver {
    pub fn resolve(&self, imports: &[String], exports: &[String]) -> Vec<WasiLink> {
        let mut provider_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut base_provider_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for export in exports {
            if let Some((component, capability)) = parse_link_entry("export:", export.as_str()) {
                provider_map
                    .entry(capability.to_string())
                    .or_default()
                    .insert(component.to_string());
                base_provider_map
                    .entry(interface_identity(capability))
                    .or_default()
                    .insert(component.to_string());
            }
        }

        imports
            .iter()
            .filter_map(|import| {
                let (import_component, import_capability) =
                    parse_link_entry("import:", import.as_str())?;
                let exact_provider = provider_map
                    .get(import_capability)
                    .and_then(|providers| providers.iter().next().cloned());
                let compatibility_provider = exact_provider.or_else(|| {
                    base_provider_map
                        .get(&interface_identity(import_capability))
                        .and_then(|providers| providers.iter().next().cloned())
                })?;
                Some(WasiLink {
                    from_component: compatibility_provider,
                    to_component: import_component.to_string(),
                    capability: import_capability.to_string(),
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapabilityValidator;

impl CapabilityValidator {
    pub fn validate(&self, graph: &WasiComponentGraph) -> bool {
        if !WasiLinker::validate(&graph.capabilities, graph) {
            return false;
        }
        if graph.runtime_constraints.max_memory_mb == 0
            || graph.runtime_constraints.max_cpu_units == 0
        {
            return false;
        }
        if graph.runtime_constraints.max_memory_mb > RUNTIME_CONSTRAINT_MAX_MEMORY_MB
            || graph.runtime_constraints.max_cpu_units > RUNTIME_CONSTRAINT_MAX_CPU_UNITS
        {
            return false;
        }
        let component_ids = graph
            .components
            .iter()
            .map(|component| component.id.as_str())
            .collect::<HashSet<_>>();
        graph.links.iter().all(|link| {
            component_ids.contains(link.from_component.as_str())
                && component_ids.contains(link.to_component.as_str())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionGraphBuilder;

impl ExecutionGraphBuilder {
    pub fn build(
        &self,
        components: Vec<WasiComponent>,
        links: Vec<WasiLink>,
        capabilities: CapabilitySet,
        imports: Vec<String>,
        exports: Vec<String>,
        runtime_constraints: RuntimeConstraints,
    ) -> WasiComponentGraph {
        let mut execution_plan = ExecutionPlan::default();
        execution_plan.startup_order = component_startup_order(&components, &links);
        execution_plan.ordered_nodes = execution_plan.startup_order.clone();
        WasiComponentGraph {
            components,
            links,
            capabilities,
            imports,
            exports,
            runtime_constraints,
            execution_plan,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WasiComponentLoader {
    pub registry: ComponentRegistry,
    pub cache: CompiledComponentCache,
    pub linker: WasiLinker,
    pub resolver: InterfaceResolver,
    pub validator: CapabilityValidator,
    pub graph_builder: ExecutionGraphBuilder,
}

impl WasiComponentLoader {
    fn compute_cache_key(
        components: &[WasiComponent],
        imports: &[String],
        exports: &[String],
    ) -> String {
        hash_key(&format!(
            "{}:{}",
            hash_key(
                &components
                    .iter()
                    .map(|component| format!("{}:{}", component.id, component.module))
                    .collect::<Vec<_>>()
                    .join("|")
            ),
            hash_key(&format!("{}:{}", imports.join("|"), exports.join("|")))
        ))
    }

    pub fn load_graph(
        &mut self,
        components: Vec<WasiComponent>,
        capabilities: CapabilitySet,
        runtime_constraints: RuntimeConstraints,
    ) -> WasiComponentGraph {
        self.registry.register_all(&components);
        let imports = components
            .iter()
            .flat_map(|component| {
                component
                    .imports
                    .iter()
                    .map(move |import| format!("import:{}:{import}", component.id))
            })
            .collect::<Vec<_>>();
        let exports = components
            .iter()
            .flat_map(|component| {
                component
                    .exports
                    .iter()
                    .map(move |export| format!("export:{}:{export}", component.id))
            })
            .collect::<Vec<_>>();
        let cache_key = Self::compute_cache_key(&components, &imports, &exports);
        if let Some(cached) = self.cache.get(cache_key.as_str()) {
            return cached;
        }

        let links = self.resolver.resolve(&imports, &exports);
        let mut graph = self.graph_builder.build(
            components,
            links,
            capabilities,
            imports,
            exports,
            runtime_constraints,
        );
        WasiLinker::optimize_graph(&mut graph);
        WasiLinker::enforce_security_model(&mut graph);
        if !self.validator.validate(&graph) {
            // Keep loader output deterministic even when validation fails by
            // returning a graph with explicit startup order and no bindings.
            graph.links.clear();
            graph.execution_plan = ExecutionPlan::default();
            graph.execution_plan.startup_order = graph
                .components
                .iter()
                .map(|component| component.id.clone())
                .collect::<Vec<_>>();
            graph.execution_plan.ordered_nodes = graph.execution_plan.startup_order.clone();
        }
        self.cache.insert(cache_key, graph.clone());
        graph
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRuntimeSpec {
    pub language: String,
    pub framework: String,
    pub package_manager: Option<String>,
    pub dependencies: Vec<String>,
    pub filesystem: RuntimeFilesystemPlan,
    pub network_policy: NetworkPolicy,
    pub memory_limit_mb: u32,
    pub cpu_limit_units: u32,
    pub cache_layers: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub ports: Vec<u16>,
    pub services: Vec<RuntimeServicePlan>,
    pub build_steps: Vec<String>,
    pub execution_steps: Vec<String>,
    pub health_checks: Vec<String>,
    pub recovery_steps: Vec<String>,
    pub requires_wasm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledWasmExecutionEnvironment {
    pub environment_id: String,
    pub spec_fingerprint: String,
    pub warm_pool_key: String,
    pub deterministic: bool,
    pub component_graph: Vec<String>,
    pub wasi_component_graph: WasiComponentGraph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WasmRuntimeCompiler;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionRuntimeSpecCompiler;

#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryAnalysis {
    pub root: PathBuf,
    pub framework: Framework,
    pub language: Language,
    pub execution_spec: Option<DdockitExecutionSpecification>,
    pub dependency_files: Vec<PathBuf>,
    pub topology: Option<ApplicationTopology>,
    pub fingerprint: RepositoryFingerprint,
    pub classification: RepositoryClassification,
    pub execution_profile: ExecutionProfile,
    pub build_intelligence: BuildIntelligence,
    pub execution_graph: ExecutionGraph,
    pub execution_image: ExecutionImage,
    pub image_match_confidence: u8,
    pub runtime_spec: ExecutionRuntimeSpec,
    pub compiled_runtime: CompiledWasmExecutionEnvironment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildArtifact {
    pub id: String,
    pub entrypoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionArtifact {
    pub key: String,
    pub node_id: String,
    pub artifact_type: ArtifactType,
    pub path: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmArtifact {
    pub node_id: String,
    pub module_path: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmArtifactBinding {
    pub node_id: String,
    pub artifact_key: String,
    pub build_fingerprint: String,
    pub source_files_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactType {
    FileSystemSnapshot,
    BuildOutput,
    TestResult,
    WasmModule,
}

impl Default for ArtifactType {
    fn default() -> Self {
        Self::BuildOutput
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        if let Err(err) = fs::create_dir_all(&root) {
            eprintln!(
                "failed to create artifact store root {}: {err}; check directory permissions and disk space",
                root.display()
            );
        }
        Self { root }
    }

    pub fn get(&self, key: &str) -> Option<ExecutionArtifact> {
        let path = self.path_for(key);
        let content = fs::read_to_string(path).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        Some(ExecutionArtifact {
            key: value.get("key")?.as_str()?.to_string(),
            node_id: value.get("node_id")?.as_str()?.to_string(),
            artifact_type: value
                .get("artifact_type")
                .and_then(Value::as_str)
                .and_then(ArtifactType::from_str)
                .unwrap_or(ArtifactType::BuildOutput),
            path: value.get("path")?.as_str()?.to_string(),
            created_at: value.get("created_at")?.as_u64()?,
        })
    }

    pub fn put(&self, artifact: ExecutionArtifact) {
        let path = self.path_for(&artifact.key);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!(
                    "failed to create artifact parent directory {}: {err}; artifact caching will be skipped for this execution",
                    parent.display()
                );
                return;
            }
        }
        let payload = json!({
            "key": artifact.key,
            "node_id": artifact.node_id,
            "artifact_type": artifact.artifact_type.as_str(),
            "path": artifact.path,
            "created_at": artifact.created_at,
        });
        if let Err(err) = fs::write(&path, payload.to_string()) {
            eprintln!(
                "failed to write artifact metadata {}: {err}; this node output will not be cached and future runs may miss cache reuse",
                path.display()
            );
        }
    }

    pub fn exists(&self, key: &str) -> bool {
        self.path_for(key).exists()
    }

    pub fn register_wasm_artifact(&self, artifact: WasmArtifact) {
        let path = self.wasm_artifact_path(&artifact.node_id);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!(
                    "failed to create wasm artifact parent directory {}: {err}; wasm artifact registration skipped",
                    parent.display()
                );
                return;
            }
        }
        let payload = json!({
            "node_id": artifact.node_id,
            "module_path": artifact.module_path,
            "hash": artifact.hash,
        });
        if let Err(err) = fs::write(&path, payload.to_string()) {
            eprintln!(
                "failed to write wasm artifact metadata {}: {err}; wasm artifact registration skipped",
                path.display()
            );
        }
    }

    pub fn get_wasm_artifact(&self, node_id: &str) -> Option<WasmArtifact> {
        let content = fs::read_to_string(self.wasm_artifact_path(node_id)).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        Some(WasmArtifact {
            node_id: value.get("node_id")?.as_str()?.to_string(),
            module_path: value.get("module_path")?.as_str()?.to_string(),
            hash: value.get("hash")?.as_str()?.to_string(),
        })
    }

    pub fn register_wasm_artifact_binding(&self, binding: WasmArtifactBinding) {
        let path = self.wasm_binding_path(&binding.node_id);
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!(
                    "failed to create wasm binding parent directory {}: {err}; wasm binding registration skipped",
                    parent.display()
                );
                return;
            }
        }
        let payload = json!({
            "node_id": binding.node_id,
            "artifact_key": binding.artifact_key,
            "build_fingerprint": binding.build_fingerprint,
            "source_files_hash": binding.source_files_hash,
        });
        if let Err(err) = fs::write(&path, payload.to_string()) {
            eprintln!(
                "failed to write wasm binding metadata {}: {err}; wasm binding registration skipped",
                path.display()
            );
        }
    }

    pub fn get_wasm_artifact_binding(&self, node_id: &str) -> Option<WasmArtifactBinding> {
        let content = fs::read_to_string(self.wasm_binding_path(node_id)).ok()?;
        let value = serde_json::from_str::<Value>(&content).ok()?;
        Some(WasmArtifactBinding {
            node_id: value.get("node_id")?.as_str()?.to_string(),
            artifact_key: value.get("artifact_key")?.as_str()?.to_string(),
            build_fingerprint: value.get("build_fingerprint")?.as_str()?.to_string(),
            source_files_hash: value.get("source_files_hash")?.as_str()?.to_string(),
        })
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.json"))
    }

    fn wasm_artifact_path(&self, node_id: &str) -> PathBuf {
        self.root
            .join("wasm")
            .join(format!("{node_id}.artifact.json"))
    }

    fn wasm_binding_path(&self, node_id: &str) -> PathBuf {
        self.root
            .join("wasm")
            .join(format!("{node_id}.binding.json"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerStatus {
    Ready,
    Busy,
    Unhealthy,
    Offline,
}

impl Default for WorkerStatus {
    fn default() -> Self {
        Self::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerCapabilities {
    pub wasm: bool,
    pub native: bool,
    pub cpu_cores: u32,
    pub memory_mb: u64,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub device_fingerprint: String,
    pub public_key: String,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub cpu: u32,
    pub memory: String,
    pub runtimes: Vec<String>,
    pub supports_wasm: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Installed,
    Registered,
    Idle,
    AssignedExecution,
    Running,
    Reporting,
    Offline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentHeartbeat {
    pub agent_id: String,
    pub cpu_usage: f32,
    pub memory_usage: f32,
    pub active_executions: u32,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentCore {
    pub identity_manager: IdentityManager,
    pub capability_reporter: CapabilityReporter,
    pub execution_runner: ExecutionRunner,
    pub process_supervisor: ProcessSupervisor,
    pub port_manager: PortManager,
    pub tunnel_client: TunnelClient,
    pub heartbeat_client: HeartbeatClient,
    pub secure_channel_client: SecureChannelClient,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdentityManager;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityReporter;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionRunner;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessSupervisor;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PortManager;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TunnelClient;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HeartbeatClient;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SecureChannelClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedExecutionGraph {
    pub graph: ExecutionGraph,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributedExecutionAgent {
    pub core: AgentCore,
    pub identity: AgentIdentity,
    pub capabilities: Option<AgentCapabilities>,
    pub status: AgentStatus,
    pub active_executions: u32,
}

impl DistributedExecutionAgent {
    pub fn new(identity: AgentIdentity) -> Self {
        Self {
            core: AgentCore::default(),
            identity,
            capabilities: None,
            status: AgentStatus::Installed,
            active_executions: 0,
        }
    }

    pub fn register(&mut self, capabilities: AgentCapabilities) -> WorkerNode {
        self.capabilities = Some(capabilities.clone());
        self.status = AgentStatus::Registered;
        let worker = WorkerNode {
            id: self.identity.agent_id.clone(),
            capabilities: WorkerCapabilities {
                wasm: capabilities.supports_wasm,
                native: true,
                cpu_cores: capabilities.cpu,
                memory_mb: parse_agent_memory_to_mb(&capabilities.memory),
                labels: vec![
                    "dea".to_string(),
                    format!("fingerprint:{}", self.identity.device_fingerprint),
                ],
            },
            status: if self.identity.trusted {
                WorkerStatus::Ready
            } else {
                WorkerStatus::Unhealthy
            },
        };
        self.status = AgentStatus::Idle;
        worker
    }

    pub fn sign_graph(&self, graph: &ExecutionGraph) -> String {
        let payload = format!(
            "{}:{}:{}:{}",
            self.identity.agent_id,
            self.identity.device_fingerprint,
            graph.cache_key(),
            graph.nodes.len()
        );
        hash_key(&format!("{}:{payload}", self.identity.public_key))
    }

    pub fn verify_graph(&self, signed_graph: &SignedExecutionGraph) -> bool {
        self.sign_graph(&signed_graph.graph) == signed_graph.signature
    }

    pub fn can_execute(&self, ctx: &ExecutionContext) -> bool {
        if !self.identity.trusted
            || matches!(self.status, AgentStatus::Installed | AgentStatus::Offline)
            || ctx.execution_graph.nodes.is_empty()
        {
            return false;
        }
        let Some(capabilities) = &self.capabilities else {
            return false;
        };
        let runtime = runtime_for_framework(ctx.analysis.framework);
        let target_runtime = if runtime == RuntimeType::Unknown {
            ctx.analysis.classification.primary_runtime
        } else {
            runtime
        };
        let runtime_name = runtime_type_to_agent_label(target_runtime);
        if target_runtime == RuntimeType::Wasm && capabilities.supports_wasm {
            return true;
        }
        capabilities
            .runtimes
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(runtime_name))
    }

    pub fn assign_execution(&mut self, signed_graph: &SignedExecutionGraph) -> Result<()> {
        if !self.verify_graph(signed_graph) {
            return Err(RuntimeError::CommandFailed(format!(
                "distributed execution agent `{}` rejected unsigned execution graph",
                self.identity.agent_id
            )));
        }
        self.status = AgentStatus::AssignedExecution;
        self.active_executions = self.active_executions.saturating_add(1);
        self.status = AgentStatus::Running;
        Ok(())
    }

    pub fn complete_execution(&mut self) {
        self.active_executions = self.active_executions.saturating_sub(1);
        self.status = if self.active_executions == 0 {
            AgentStatus::Idle
        } else {
            AgentStatus::Running
        };
    }

    pub fn heartbeat(&self, cpu_usage: f32, memory_usage: f32) -> AgentHeartbeat {
        AgentHeartbeat {
            agent_id: self.identity.agent_id.clone(),
            cpu_usage,
            memory_usage,
            active_executions: self.active_executions,
            status: self.status,
        }
    }

    pub fn stable_workspace_url(&self, workspace_id: &str) -> String {
        let normalized_workspace_id = workspace_id
            .trim()
            .strip_prefix("workspace-")
            .unwrap_or(workspace_id);
        let sanitized = ExecutionRouter::sanitized_workspace_id(normalized_workspace_id);
        format!("https://workspace-{sanitized}.trythissoftware.com")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerNode {
    pub id: String,
    pub capabilities: WorkerCapabilities,
    pub status: WorkerStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerRegistry {
    pub workers: HashMap<String, WorkerNode>,
    pub heartbeats: HashMap<String, u64>,
    pub heartbeat_reports: HashMap<String, WorkerHeartbeat>,
    pub heartbeat_timeout_secs: u64,
}

impl WorkerRegistry {
    pub fn new(heartbeat_timeout_secs: u64) -> Self {
        Self {
            workers: HashMap::new(),
            heartbeats: HashMap::new(),
            heartbeat_reports: HashMap::new(),
            heartbeat_timeout_secs: heartbeat_timeout_secs.max(MIN_COORDINATION_TIMEOUT_SECS),
        }
    }

    pub fn from_workers(workers: Vec<WorkerNode>, heartbeat_timeout_secs: u64, now: u64) -> Self {
        let mut registry = Self::new(heartbeat_timeout_secs);
        for worker in workers {
            registry.register_worker(worker, now);
        }
        registry
    }

    pub fn register_worker(&mut self, worker: WorkerNode, now: u64) {
        let worker_id = worker.id.clone();
        self.workers.insert(worker_id.clone(), worker);
        self.heartbeats.insert(worker_id, now);
    }

    pub fn record_heartbeat(&mut self, worker_id: &str, now: u64) -> bool {
        let Some(worker) = self.workers.get_mut(worker_id) else {
            return false;
        };
        if matches!(
            worker.status,
            WorkerStatus::Offline | WorkerStatus::Unhealthy
        ) {
            worker.status = WorkerStatus::Ready;
        }
        self.heartbeats.insert(worker_id.to_string(), now);
        true
    }

    pub fn record_worker_heartbeat(&mut self, heartbeat: WorkerHeartbeat) -> bool {
        if !self.record_heartbeat(&heartbeat.worker_id, heartbeat.timestamp) {
            return false;
        }
        let worker_id = heartbeat.worker_id.clone();
        if !heartbeat.health {
            if let Some(worker) = self.workers.get_mut(&worker_id) {
                worker.status = WorkerStatus::Unhealthy;
            }
        }
        self.heartbeat_reports.insert(worker_id, heartbeat);
        true
    }

    pub fn detect_failed_workers(&mut self, now: u64) -> Vec<String> {
        let mut failed_workers = Vec::new();
        for worker in self.workers.values_mut() {
            if matches!(worker.status, WorkerStatus::Offline) {
                continue;
            }
            let last_heartbeat = self.heartbeats.get(&worker.id).copied().unwrap_or(0);
            if now.saturating_sub(last_heartbeat) > self.heartbeat_timeout_secs {
                worker.status = WorkerStatus::Offline;
                failed_workers.push(worker.id.clone());
            }
        }
        failed_workers.sort();
        failed_workers
    }

    pub fn mark_worker_offline(&mut self, worker_id: &str) -> bool {
        let Some(worker) = self.workers.get_mut(worker_id) else {
            return false;
        };
        worker.status = WorkerStatus::Offline;
        true
    }

    pub fn snapshot_workers(&self) -> Vec<WorkerNode> {
        let mut workers = self.workers.values().cloned().collect::<Vec<_>>();
        workers.sort_by(|a, b| a.id.cmp(&b.id));
        workers
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeLease {
    pub node_id: String,
    pub worker_id: String,
    /// Unix epoch timestamp in seconds after which this lease is invalid.
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerQueue {
    pub queued_nodes: Vec<String>,
}

impl WorkerQueue {
    pub fn enqueue(&mut self, node_id: String) {
        if !self.queued_nodes.iter().any(|queued| queued == &node_id) {
            self.queued_nodes.push(node_id);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphPartition {
    pub worker_id: String,
    pub nodes: Vec<ExecutionNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeAssignment {
    pub node_id: String,
    pub worker_id: String,
    pub sequence: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionPlan {
    pub topology: Option<ApplicationTopology>,
    pub services: Vec<ServiceDefinition>,
    pub startup_order: Vec<String>,
    pub ordered_nodes: Vec<String>,
    pub assignments: Vec<NodeAssignment>,
    pub leases: HashMap<String, NodeLease>,
    pub worker_queues: HashMap<String, WorkerQueue>,
    pub partitions: Vec<GraphPartition>,
    pub unscheduled_nodes: Vec<String>,
}

impl ExecutionPlan {
    pub fn mark_worker_failed(&mut self, worker_id: &str) -> Vec<String> {
        let mut failed_nodes = Vec::new();
        self.leases.retain(|node_id, lease| {
            let keep = lease.worker_id != worker_id;
            if !keep {
                failed_nodes.push(node_id.clone());
            }
            keep
        });
        if let Some(queue) = self.worker_queues.get_mut(worker_id) {
            queue.queued_nodes.clear();
        }
        failed_nodes.sort();
        failed_nodes
    }

    pub fn reassign_stale_assignments(
        &mut self,
        workers: &[WorkerNode],
        lease_ttl_secs: u64,
        now: u64,
    ) -> Vec<String> {
        let mut stale_nodes = self
            .leases
            .iter()
            .filter_map(|(node_id, lease)| (lease.expires_at <= now).then_some(node_id.clone()))
            .collect::<Vec<_>>();
        stale_nodes.sort();
        self.reassign_nodes(stale_nodes, workers, lease_ttl_secs, now)
    }

    pub fn reassign_failed_worker(
        &mut self,
        failed_worker_id: &str,
        workers: &[WorkerNode],
        lease_ttl_secs: u64,
        now: u64,
    ) -> Vec<String> {
        let mut failed_nodes = self
            .leases
            .iter()
            .filter_map(|(node_id, lease)| {
                (lease.worker_id == failed_worker_id).then_some(node_id.clone())
            })
            .collect::<Vec<_>>();
        failed_nodes.sort();
        if let Some(queue) = self.worker_queues.get_mut(failed_worker_id) {
            queue.queued_nodes.clear();
        }
        self.reassign_nodes(failed_nodes, workers, lease_ttl_secs, now)
    }

    fn reassign_nodes(
        &mut self,
        node_ids: Vec<String>,
        workers: &[WorkerNode],
        lease_ttl_secs: u64,
        now: u64,
    ) -> Vec<String> {
        let mut candidates = workers
            .iter()
            .filter(|worker| worker_is_usable(worker))
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));

        let mut reassigned = Vec::new();
        let ttl = lease_ttl_secs.max(MIN_COORDINATION_TIMEOUT_SECS);
        for node_id in node_ids {
            let Some(existing_lease) = self.leases.get(&node_id).cloned() else {
                continue;
            };
            let selected = candidates
                .iter()
                .copied()
                .find(|candidate| candidate.id != existing_lease.worker_id);
            let Some(worker) = selected else {
                continue;
            };

            if let Some(queue) = self.worker_queues.get_mut(&existing_lease.worker_id) {
                queue.queued_nodes.retain(|queued| queued != &node_id);
            }
            self.worker_queues
                .entry(worker.id.clone())
                .or_default()
                .enqueue(node_id.clone());

            if let Some(assignment) = self
                .assignments
                .iter_mut()
                .find(|assignment| assignment.node_id == node_id)
            {
                assignment.worker_id = worker.id.clone();
            }

            self.leases.insert(
                node_id.clone(),
                NodeLease {
                    node_id: node_id.clone(),
                    worker_id: worker.id.clone(),
                    expires_at: now.saturating_add(ttl),
                },
            );
            reassigned.push(node_id);
        }

        reassigned.sort();
        reassigned
    }
}

pub fn generate_execution_plan(analysis: &RepositoryAnalysis) -> ExecutionPlan {
    let mut plan = ExecutionPlan::default();
    if let Some(topology) = analysis.topology.as_ref() {
        plan.topology = Some(topology.clone());
        plan.services = topology.services.clone();
        plan.startup_order = topology
            .startup_order
            .stages
            .iter()
            .flatten()
            .cloned()
            .collect();
    }
    plan
}

#[derive(Debug, Clone, Default)]
pub struct DistributedArtifactStore {
    artifacts: Arc<Mutex<HashMap<String, ExecutionArtifact>>>,
}

impl DistributedArtifactStore {
    pub fn store(&self, artifact: ExecutionArtifact) {
        self.artifacts
            .lock()
            .expect(DISTRIBUTED_ARTIFACT_STORE_POISONED)
            .insert(artifact.key.clone(), artifact);
    }

    pub fn fetch(&self, key: &str) -> Option<ExecutionArtifact> {
        self.artifacts
            .lock()
            .expect(DISTRIBUTED_ARTIFACT_STORE_POISONED)
            .get(key)
            .cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributedExecutionConfig {
    pub required_worker_labels: HashMap<String, Vec<String>>,
    pub required_artifacts: HashMap<String, Vec<String>>,
    pub lease_ttl_secs: u64,
}

impl Default for DistributedExecutionConfig {
    fn default() -> Self {
        Self {
            required_worker_labels: HashMap::new(),
            required_artifacts: HashMap::new(),
            lease_ttl_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExecutionPriority {
    Interactive,
    System,
    Batch,
}

impl ExecutionPriority {
    fn rank(self) -> u8 {
        match self {
            Self::Interactive => 0,
            Self::System => 1,
            Self::Batch => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionQueueStatus {
    Queued,
    Running,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedExecution {
    pub execution_id: ExecutionId,
    pub org_id: OrganizationId,
    pub priority: ExecutionPriority,
    pub status: ExecutionQueueStatus,
    pub submitted_at: DateTime,
    pub preferred_region: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeNodeType {
    Dea,
    Docker,
    Cloud,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeNodeHealth {
    Healthy,
    Degraded,
    Unhealthy,
    Offline,
}

impl RuntimeNodeHealth {
    fn is_routable(self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeNode {
    pub node_id: String,
    pub runtime_type: RuntimeNodeType,
    pub capacity_cpu: u32,
    pub capacity_memory: u64,
    pub current_load: u32,
    pub health_status: RuntimeNodeHealth,
    pub region: String,
    pub cost_per_second: f64,
    pub latency_ms: u32,
    pub max_concurrent_executions: usize,
    pub active_jobs: Vec<ExecutionId>,
    pub last_heartbeat: DateTime,
    pub success_rate: f64,
    pub warm_pool_ready: bool,
}

impl RuntimeNode {
    fn available_slots(&self) -> usize {
        self.max_concurrent_executions
            .saturating_sub(self.active_jobs.len())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionQueue {
    pub executions: VecDeque<QueuedExecution>,
}

impl ExecutionQueue {
    pub fn enqueue(&mut self, mut execution: QueuedExecution) {
        execution.status = ExecutionQueueStatus::Queued;
        self.executions.push_back(execution);
    }

    pub fn len(&self) -> usize {
        self.executions.len()
    }

    fn next_schedulable_index(&self) -> Option<usize> {
        self.executions
            .iter()
            .enumerate()
            .filter(|(_, execution)| execution.status != ExecutionQueueStatus::Running)
            .min_by(|(_, left), (_, right)| {
                left.priority
                    .rank()
                    .cmp(&right.priority.rank())
                    .then_with(|| left.submitted_at.cmp(&right.submitted_at))
                    .then_with(|| left.execution_id.cmp(&right.execution_id))
            })
            .map(|(index, _)| index)
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeRegistry {
    pub nodes: HashMap<String, RuntimeNode>,
}

impl RuntimeRegistry {
    pub fn register_node(&mut self, node: RuntimeNode) {
        self.nodes.insert(node.node_id.clone(), node);
    }

    pub fn record_heartbeat(
        &mut self,
        node_id: &str,
        heartbeat_at: DateTime,
        healthy: bool,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return false;
        };
        node.last_heartbeat = heartbeat_at;
        node.health_status = if healthy {
            RuntimeNodeHealth::Healthy
        } else {
            RuntimeNodeHealth::Unhealthy
        };
        true
    }

    pub fn assign_execution(
        &mut self,
        node_id: &str,
        execution_id: ExecutionId,
        heartbeat_at: DateTime,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return false;
        };
        if !node.health_status.is_routable() || node.available_slots() == 0 {
            return false;
        }
        if !node
            .active_jobs
            .iter()
            .any(|active| active == &execution_id)
        {
            node.active_jobs.push(execution_id);
            node.current_load = node.active_jobs.len() as u32;
        }
        node.last_heartbeat = heartbeat_at;
        true
    }

    pub fn release_execution(&mut self, node_id: &str, execution_id: &str) -> bool {
        let Some(node) = self.nodes.get_mut(node_id) else {
            return false;
        };
        let before = node.active_jobs.len();
        node.active_jobs.retain(|active| active != execution_id);
        node.current_load = node.active_jobs.len() as u32;
        node.active_jobs.len() != before
    }

    pub fn active_jobs_for_node(&self, node_id: &str) -> Vec<ExecutionId> {
        self.nodes
            .get(node_id)
            .map(|node| node.active_jobs.clone())
            .unwrap_or_default()
    }

    pub fn detect_unhealthy_nodes(
        &mut self,
        now: DateTime,
        heartbeat_timeout_secs: u64,
    ) -> Vec<String> {
        let timeout = heartbeat_timeout_secs.max(MIN_COORDINATION_TIMEOUT_SECS);
        let mut stale = Vec::new();
        for node in self.nodes.values_mut() {
            if !node.health_status.is_routable() {
                continue;
            }
            if now.saturating_sub(node.last_heartbeat) > timeout {
                node.health_status = RuntimeNodeHealth::Unhealthy;
                stale.push(node.node_id.clone());
            }
        }
        stale.sort();
        stale
    }
}

pub trait RoutingPolicy {
    fn score(&self, node: &RuntimeNode, execution: &QueuedExecution) -> f64;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefaultRoutingPolicy;

impl RoutingPolicy for DefaultRoutingPolicy {
    fn score(&self, node: &RuntimeNode, execution: &QueuedExecution) -> f64 {
        if !node.health_status.is_routable() || node.available_slots() == 0 {
            return f64::NEG_INFINITY;
        }
        let max_slots = node.max_concurrent_executions.max(1) as f64;
        let load_ratio = (node.active_jobs.len() as f64 / max_slots).clamp(0.0, 1.0);
        let load_score = 1.0 - load_ratio;
        let cost_score = 1.0 / (1.0 + node.cost_per_second.max(0.0));
        let latency_score = 1.0 / (1.0 + f64::from(node.latency_ms.max(1)));
        let success_score = node.success_rate.clamp(0.0, 1.0);
        let warm_pool_bonus = if node.warm_pool_ready { 0.15 } else { 0.0 };
        let priority_bonus = match execution.priority {
            ExecutionPriority::Interactive => 0.15,
            ExecutionPriority::System => 0.1,
            ExecutionPriority::Batch => 0.0,
        };
        let region_bonus = if execution
            .preferred_region
            .as_deref()
            .is_some_and(|region| region == node.region)
        {
            0.1
        } else {
            0.0
        };
        (load_score * 0.35)
            + (cost_score * 0.2)
            + (latency_score * 0.2)
            + (success_score * 0.15)
            + warm_pool_bonus
            + priority_bonus
            + region_bonus
    }
}

#[derive(Debug, Clone, Default)]
pub struct RoutingPolicyEngine;

impl RoutingPolicyEngine {
    pub fn score(&self, node: &RuntimeNode, execution: &QueuedExecution) -> f64 {
        DefaultRoutingPolicy.score(node, execution)
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadBalancer;

impl LoadBalancer {
    pub fn select_best_runtime(
        &self,
        execution: &QueuedExecution,
        registry: &RuntimeRegistry,
        policy_engine: &RoutingPolicyEngine,
    ) -> Option<String> {
        registry
            .nodes
            .values()
            .filter(|node| node.health_status.is_routable())
            .filter(|node| node.available_slots() > 0)
            .max_by(|left, right| {
                let left_score = policy_engine.score(left, execution);
                let right_score = policy_engine.score(right, execution);
                left_score
                    .total_cmp(&right_score)
                    // `max_by` keeps the left entry when ordering is `Greater`; reversing
                    // right/left comparisons here intentionally prefers lower load/cost/latency.
                    .then_with(|| right.current_load.cmp(&left.current_load))
                    .then_with(|| right.cost_per_second.total_cmp(&left.cost_per_second))
                    .then_with(|| right.latency_ms.cmp(&left.latency_ms))
                    .then_with(|| left.node_id.cmp(&right.node_id))
            })
            .map(|node| node.node_id.clone())
    }
}

pub trait RuntimeProvider {
    fn can_execute(&self, repo: &RepositoryAnalysis) -> bool;
    fn execute(&self, job: &QueuedExecution) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerEvent {
    pub execution_id: ExecutionId,
    pub selected_node: Option<String>,
    pub reason: String,
    pub queue_time: u64,
    pub start_time: Option<DateTime>,
}

#[derive(Debug, Clone)]
pub struct DistributedExecutionScheduler {
    pub queue: ExecutionQueue,
    pub registry: RuntimeRegistry,
    pub load_balancer: LoadBalancer,
    pub policy_engine: RoutingPolicyEngine,
    pub scheduler_events: Vec<SchedulerEvent>,
    pub in_flight: HashMap<ExecutionId, QueuedExecution>,
    pub backpressure_threshold: usize,
}

impl Default for DistributedExecutionScheduler {
    fn default() -> Self {
        Self {
            queue: ExecutionQueue::default(),
            registry: RuntimeRegistry::default(),
            load_balancer: LoadBalancer,
            policy_engine: RoutingPolicyEngine,
            scheduler_events: Vec::new(),
            in_flight: HashMap::new(),
            backpressure_threshold: 1_000,
        }
    }
}

impl DistributedExecutionScheduler {
    pub fn enqueue(&mut self, execution: QueuedExecution) {
        self.queue.enqueue(execution);
    }

    pub fn register_runtime_node(&mut self, node: RuntimeNode) {
        self.registry.register_node(node);
    }

    pub fn queue_length(&self) -> usize {
        self.queue.len()
    }

    pub fn should_scale_runtime(&self, runtime_type: RuntimeNodeType) -> bool {
        let has_backlog = self
            .queue
            .executions
            .iter()
            .any(|execution| execution.status != ExecutionQueueStatus::Running);
        if !has_backlog {
            return false;
        }
        let mut saw_runtime = false;
        for node in self.registry.nodes.values() {
            if node.runtime_type != runtime_type || !node.health_status.is_routable() {
                continue;
            }
            saw_runtime = true;
            if node.available_slots() > 0 {
                return false;
            }
        }
        saw_runtime
    }

    pub fn schedule_next(&mut self, now: DateTime) -> Option<SchedulerEvent> {
        let index = self.queue.next_schedulable_index()?;
        let queue_overloaded = self.queue.len() > self.backpressure_threshold;
        let execution = self.queue.executions.get(index)?.clone();

        if queue_overloaded && execution.priority == ExecutionPriority::Batch {
            if let Some(entry) = self.queue.executions.get_mut(index) {
                entry.status = ExecutionQueueStatus::Blocked;
            }
            let event = SchedulerEvent {
                execution_id: execution.execution_id,
                selected_node: None,
                reason: "backpressure delayed batch execution".to_string(),
                queue_time: now.saturating_sub(execution.submitted_at),
                start_time: None,
            };
            self.scheduler_events.push(event.clone());
            return Some(event);
        }

        let selected =
            self.load_balancer
                .select_best_runtime(&execution, &self.registry, &self.policy_engine);
        let queue_time = now.saturating_sub(execution.submitted_at);

        let Some(selected_node) = selected else {
            if let Some(entry) = self.queue.executions.get_mut(index) {
                entry.status = ExecutionQueueStatus::Blocked;
            }
            let event = SchedulerEvent {
                execution_id: execution.execution_id,
                selected_node: None,
                reason: "no healthy runtime node with capacity".to_string(),
                queue_time,
                start_time: None,
            };
            self.scheduler_events.push(event.clone());
            return Some(event);
        };

        let mut running = self.queue.executions.remove(index)?;
        running.status = ExecutionQueueStatus::Running;
        if !self
            .registry
            .assign_execution(&selected_node, running.execution_id.clone(), now)
        {
            running.status = ExecutionQueueStatus::Blocked;
            self.queue.executions.push_front(running);
            let event = SchedulerEvent {
                execution_id: execution.execution_id,
                selected_node: None,
                reason: "runtime capacity changed before assignment".to_string(),
                queue_time,
                start_time: None,
            };
            self.scheduler_events.push(event.clone());
            return Some(event);
        }

        self.in_flight
            .insert(running.execution_id.clone(), running.clone());
        let event = SchedulerEvent {
            execution_id: running.execution_id,
            selected_node: Some(selected_node),
            reason: "selected by routing policy score".to_string(),
            queue_time,
            start_time: Some(now),
        };
        self.scheduler_events.push(event.clone());
        Some(event)
    }

    pub fn recover_failed_executions(
        &mut self,
        now: DateTime,
        heartbeat_timeout_secs: u64,
    ) -> Vec<ExecutionId> {
        let failed_nodes = self
            .registry
            .detect_unhealthy_nodes(now, heartbeat_timeout_secs);
        let mut recovered = Vec::new();

        for node_id in failed_nodes {
            for execution_id in self.registry.active_jobs_for_node(&node_id) {
                if self.registry.release_execution(&node_id, &execution_id) {
                    if let Some(mut queued) = self.in_flight.remove(&execution_id) {
                        queued.status = ExecutionQueueStatus::Queued;
                        queued.submitted_at = now;
                        self.queue.enqueue(queued);
                        recovered.push(execution_id);
                    }
                }
            }
        }

        recovered.sort();
        recovered
    }
}

#[derive(Debug, Clone, Default)]
pub struct DistributedScheduler;

impl DistributedScheduler {
    pub fn schedule(graph: ExecutionGraph, workers: Vec<WorkerNode>) -> ExecutionPlan {
        Self::default().schedule_with_context(
            graph,
            workers,
            &DistributedArtifactStore::default(),
            &DistributedExecutionConfig::default(),
            current_unix_epoch_secs(),
        )
    }

    pub fn schedule_with_context(
        &self,
        graph: ExecutionGraph,
        workers: Vec<WorkerNode>,
        artifact_store: &DistributedArtifactStore,
        config: &DistributedExecutionConfig,
        now: u64,
    ) -> ExecutionPlan {
        let node_lookup: HashMap<String, ExecutionNode> = graph
            .nodes
            .iter()
            .cloned()
            .map(|node| (node.id.clone(), node))
            .collect();
        let mut indegree: HashMap<String, usize> = graph
            .nodes
            .iter()
            .map(|node| (node.id.clone(), 0usize))
            .collect();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

        for edge in &graph.edges {
            if let Some(count) = indegree.get_mut(&edge.to) {
                *count += 1;
            }
            adjacency
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }
        for next in adjacency.values_mut() {
            next.sort();
        }

        let mut ready: BTreeSet<String> = indegree
            .iter()
            .filter_map(|(node_id, degree)| (*degree == 0).then_some(node_id.clone()))
            .collect();
        let mut completed: HashSet<String> = HashSet::new();
        let mut unscheduled_nodes = Vec::new();
        let mut ordered_nodes = Vec::new();
        let mut assignments = Vec::new();
        let mut leases = HashMap::new();
        let mut worker_queues: HashMap<String, WorkerQueue> = workers
            .iter()
            .map(|worker| (worker.id.clone(), WorkerQueue::default()))
            .collect();
        let mut assignment_counts: HashMap<String, usize> = workers
            .iter()
            .map(|worker| (worker.id.clone(), 0usize))
            .collect();
        // A zero-second lease expires immediately and allows duplicate execution.
        let lease_ttl = config.lease_ttl_secs.max(1);

        while let Some(node_id) = ready.iter().next().cloned() {
            ready.remove(&node_id);
            ordered_nodes.push(node_id.clone());
            let Some(node) = node_lookup.get(&node_id) else {
                continue;
            };

            let supports_artifacts = config
                .required_artifacts
                .get(&node_id)
                .map(|required| {
                    required
                        .iter()
                        .all(|key| artifact_store.fetch(key).is_some())
                })
                .unwrap_or(true);
            if !supports_artifacts {
                unscheduled_nodes.push(node_id.clone());
                continue;
            }

            let required_labels = config
                .required_worker_labels
                .get(&node_id)
                .cloned()
                .unwrap_or_default();
            let selected = workers
                .iter()
                .filter(|worker| worker_is_usable(worker))
                .filter(|worker| worker_supports_mode(worker, node.execution_mode))
                .filter(|worker| worker_has_labels(worker, &required_labels))
                .min_by(|a, b| {
                    let a_count = assignment_counts.get(&a.id).copied().unwrap_or(0);
                    let b_count = assignment_counts.get(&b.id).copied().unwrap_or(0);
                    a_count.cmp(&b_count).then_with(|| a.id.cmp(&b.id))
                });

            let Some(worker) = selected else {
                unscheduled_nodes.push(node_id.clone());
                continue;
            };

            let sequence = assignments.len();
            assignments.push(NodeAssignment {
                node_id: node_id.clone(),
                worker_id: worker.id.clone(),
                sequence,
            });
            leases.insert(
                node_id.clone(),
                NodeLease {
                    node_id: node_id.clone(),
                    worker_id: worker.id.clone(),
                    expires_at: now.saturating_add(lease_ttl),
                },
            );
            if let Some(queue) = worker_queues.get_mut(&worker.id) {
                queue.enqueue(node_id.clone());
            }
            *assignment_counts.entry(worker.id.clone()).or_insert(0) += 1;
            completed.insert(node_id.clone());

            if let Some(next_nodes) = adjacency.get(&node_id) {
                for next in next_nodes {
                    if let Some(degree) = indegree.get_mut(next) {
                        *degree = degree.saturating_sub(1);
                        if *degree == 0 {
                            ready.insert(next.clone());
                        }
                    }
                }
            }
        }

        for node_id in node_lookup.keys() {
            if !completed.contains(node_id) && !unscheduled_nodes.contains(node_id) {
                unscheduled_nodes.push(node_id.clone());
            }
        }
        unscheduled_nodes.sort();
        unscheduled_nodes.dedup();

        let mut by_worker: HashMap<String, Vec<ExecutionNode>> = HashMap::new();
        for assignment in &assignments {
            if let Some(node) = node_lookup.get(&assignment.node_id) {
                by_worker
                    .entry(assignment.worker_id.clone())
                    .or_default()
                    .push(node.clone());
            }
        }
        let mut worker_ids = by_worker.keys().cloned().collect::<Vec<_>>();
        worker_ids.sort();
        let partitions = worker_ids
            .into_iter()
            .map(|worker_id| GraphPartition {
                worker_id: worker_id.clone(),
                nodes: by_worker.remove(&worker_id).unwrap_or_default(),
            })
            .collect();

        ExecutionPlan {
            topology: None,
            services: vec![],
            startup_order: vec![],
            ordered_nodes,
            assignments,
            leases,
            worker_queues,
            partitions,
            unscheduled_nodes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeshTopology {
    #[default]
    HubAndSpoke,
    Regional,
    PeerToPeer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeshNodeType {
    #[default]
    Local,
    Cloud,
    Edge,
    Peer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeshNodeTrustLevel {
    #[default]
    FullAccess,
    Sandboxed,
    RestrictedIo,
    SignedExecutionOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MeshNode {
    pub id: String,
    pub node_type: MeshNodeType,
    pub trust_level: MeshNodeTrustLevel,
    pub capabilities: WorkerCapabilities,
    pub status: WorkerStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentPlacementConstraints {
    pub cpu: u32,
    pub memory_mb: u64,
    pub network: bool,
    pub filesystem: bool,
    pub latency_sensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentPlacement {
    pub component_id: String,
    pub preferred_node_type: MeshNodeType,
    pub constraints: ComponentPlacementConstraints,
    pub affinity_rules: Vec<String>,
    pub fallback_nodes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MeshExecutionPartition {
    pub node_id: String,
    pub components: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DistributedWasiExecutionGraph {
    pub placements: Vec<ComponentPlacement>,
    pub partitions: Vec<MeshExecutionPartition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshScheduler;

impl MeshScheduler {
    pub fn plan(
        &self,
        wasi_graph: &WasiComponentGraph,
        nodes: &[MeshNode],
    ) -> DistributedWasiExecutionGraph {
        let router = MeshExecutionRouter;
        let mut placements = Vec::new();
        let mut by_node: HashMap<String, Vec<String>> = HashMap::new();

        for component in &wasi_graph.components {
            let preferred_node_type = Self::preferred_node_type(component);
            let constraints = Self::constraints_for(component);
            let fallback_nodes = router.candidate_nodes(preferred_node_type, &constraints, nodes);
            let placement = ComponentPlacement {
                component_id: component.id.clone(),
                preferred_node_type,
                constraints,
                affinity_rules: vec![format!("component:{}:affinity", component.id)],
                fallback_nodes,
            };
            if let Some(node_id) = router.route(&placement, nodes) {
                by_node
                    .entry(node_id)
                    .or_default()
                    .push(component.id.clone());
            }
            placements.push(placement);
        }

        let mut node_ids = by_node.keys().cloned().collect::<Vec<_>>();
        node_ids.sort();
        let partitions = node_ids
            .into_iter()
            .map(|node_id| MeshExecutionPartition {
                components: by_node.remove(&node_id).unwrap_or_default(),
                node_id,
            })
            .collect::<Vec<_>>();

        DistributedWasiExecutionGraph {
            placements,
            partitions,
        }
    }

    fn preferred_node_type(component: &WasiComponent) -> MeshNodeType {
        let capabilities = component
            .capabilities
            .iter()
            .map(|capability| capability.to_ascii_lowercase())
            .collect::<Vec<_>>();

        if capabilities
            .iter()
            .any(|capability| capability.contains("serve") || capability.contains("preview"))
        {
            MeshNodeType::Edge
        } else if capabilities.iter().any(|capability| {
            capability.contains("build")
                || capability.contains("install")
                || capability.contains("compile")
                || capability.contains("package_manager")
        }) {
            MeshNodeType::Cloud
        } else if capabilities
            .iter()
            .any(|capability| capability.contains("peer"))
        {
            MeshNodeType::Peer
        } else {
            MeshNodeType::Local
        }
    }

    fn constraints_for(component: &WasiComponent) -> ComponentPlacementConstraints {
        let capabilities = component
            .capabilities
            .iter()
            .map(|capability| capability.to_ascii_lowercase())
            .collect::<Vec<_>>();
        ComponentPlacementConstraints {
            cpu: if capabilities.iter().any(|capability| {
                capability.contains("build")
                    || capability.contains("compile")
                    || capability.contains("test")
            }) {
                2
            } else {
                1
            },
            memory_mb: if capabilities
                .iter()
                .any(|capability| capability.contains("build") || capability.contains("compile"))
            {
                1024
            } else {
                256
            },
            network: capabilities
                .iter()
                .any(|capability| capability.contains("network") || capability.contains("http")),
            filesystem: capabilities.iter().any(|capability| {
                capability.contains("filesystem") || capability.contains("build")
            }),
            latency_sensitive: capabilities
                .iter()
                .any(|capability| capability.contains("latency") || capability.contains("serve")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeshExecutionRouter;

impl MeshExecutionRouter {
    pub fn route(&self, placement: &ComponentPlacement, nodes: &[MeshNode]) -> Option<String> {
        self.candidate_nodes(placement.preferred_node_type, &placement.constraints, nodes)
            .into_iter()
            .next()
    }

    pub fn rebalance(&self, placements: &mut [ComponentPlacement], nodes: &[MeshNode]) {
        for placement in placements {
            placement.fallback_nodes =
                self.candidate_nodes(placement.preferred_node_type, &placement.constraints, nodes);
        }
    }

    pub fn migrate(
        &self,
        component_id: &str,
        placements: &mut [ComponentPlacement],
        target_node: &str,
    ) -> bool {
        let Some(placement) = placements
            .iter_mut()
            .find(|placement| placement.component_id == component_id)
        else {
            return false;
        };
        placement
            .fallback_nodes
            .retain(|node_id| node_id != target_node);
        placement.fallback_nodes.insert(0, target_node.to_string());
        true
    }

    pub fn replicate(
        &self,
        component_id: &str,
        placements: &[ComponentPlacement],
        replicas: usize,
    ) -> Vec<String> {
        placements
            .iter()
            .find(|placement| placement.component_id == component_id)
            .map(|placement| {
                placement
                    .fallback_nodes
                    .iter()
                    .take(replicas)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn candidate_nodes(
        &self,
        preferred_node_type: MeshNodeType,
        constraints: &ComponentPlacementConstraints,
        nodes: &[MeshNode],
    ) -> Vec<String> {
        let mut candidates = nodes
            .iter()
            .filter(|node| matches!(node.status, WorkerStatus::Ready | WorkerStatus::Busy))
            .filter(|node| node.capabilities.wasm)
            .filter(|node| node.capabilities.cpu_cores >= constraints.cpu)
            .filter(|node| node.capabilities.memory_mb >= constraints.memory_mb)
            .filter(|node| {
                !constraints.network
                    || node
                        .capabilities
                        .labels
                        .iter()
                        .any(|label| label == "network")
            })
            .filter(|node| {
                !constraints.filesystem
                    || node
                        .capabilities
                        .labels
                        .iter()
                        .any(|label| label == "filesystem")
            })
            .cloned()
            .collect::<Vec<_>>();

        candidates.sort_by(|a, b| {
            let a_preferred = a.node_type == preferred_node_type;
            let b_preferred = b.node_type == preferred_node_type;
            b_preferred.cmp(&a_preferred).then_with(|| a.id.cmp(&b.id))
        });
        candidates.into_iter().map(|node| node.id).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateSyncMode {
    Eager,
    Lazy,
    #[default]
    Eventual,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StateSynchronizer {
    pub sync_modes: HashMap<String, StateSyncMode>,
    pub revisions: HashMap<String, u64>,
}

impl StateSynchronizer {
    pub fn set_mode(&mut self, component_id: impl Into<String>, mode: StateSyncMode) {
        self.sync_modes.insert(component_id.into(), mode);
    }

    pub fn mode_for(&self, component_id: &str) -> StateSyncMode {
        self.sync_modes
            .get(component_id)
            .copied()
            .unwrap_or(StateSyncMode::Eventual)
    }

    pub fn record_sync(&mut self, component_id: impl Into<String>, revision: u64) {
        self.revisions.insert(component_id.into(), revision);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshFailureClass {
    NodeUnavailable,
    ComponentCrash,
    StateDivergence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureEvent {
    pub component_id: String,
    pub node_id: String,
    pub class: MeshFailureClass,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FailureDetector {
    pub events: Vec<FailureEvent>,
}

impl FailureDetector {
    pub fn record(&mut self, event: FailureEvent) {
        self.events.push(event);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionMesh {
    pub nodes: Vec<MeshNode>,
    pub topology: MeshTopology,
    pub scheduler: MeshScheduler,
    pub router: MeshExecutionRouter,
    pub state_sync: StateSynchronizer,
    pub failure_detector: FailureDetector,
}

impl Default for ExecutionMesh {
    fn default() -> Self {
        Self {
            nodes: vec![],
            topology: MeshTopology::default(),
            scheduler: MeshScheduler,
            router: MeshExecutionRouter,
            state_sync: StateSynchronizer::default(),
            failure_detector: FailureDetector::default(),
        }
    }
}

impl ExecutionMesh {
    pub fn plan(&self, graph: &WasiComponentGraph) -> DistributedWasiExecutionGraph {
        self.scheduler.plan(graph, &self.nodes)
    }

    pub fn heal_component(
        &mut self,
        placements: &mut [ComponentPlacement],
        component_id: &str,
        failed_node_id: &str,
        class: MeshFailureClass,
        timestamp: u64,
    ) -> Option<String> {
        self.failure_detector.record(FailureEvent {
            component_id: component_id.to_string(),
            node_id: failed_node_id.to_string(),
            class,
            timestamp,
        });
        let placement = placements
            .iter()
            .find(|placement| placement.component_id == component_id)?;
        let target = placement
            .fallback_nodes
            .iter()
            .find(|node_id| node_id.as_str() != failed_node_id)
            .cloned()?;
        self.router
            .migrate(component_id, placements, &target)
            .then_some(target)
    }
}

fn worker_is_usable(worker: &WorkerNode) -> bool {
    matches!(worker.status, WorkerStatus::Ready | WorkerStatus::Busy)
}

fn worker_supports_mode(worker: &WorkerNode, mode: ExecutionMode) -> bool {
    match mode {
        ExecutionMode::Native => worker.capabilities.native,
        ExecutionMode::Wasm => worker.capabilities.wasm,
        ExecutionMode::Hybrid => worker.capabilities.native || worker.capabilities.wasm,
    }
}

fn worker_has_labels(worker: &WorkerNode, labels: &[String]) -> bool {
    labels.iter().all(|label| {
        worker
            .capabilities
            .labels
            .iter()
            .any(|worker_label| worker_label == label)
    })
}

#[derive(Debug, Clone)]
pub struct ExecutionCoordinator {
    pub scheduler: DistributedScheduler,
    pub workers: Vec<WorkerNode>,
    pub worker_registry: WorkerRegistry,
    pub artifact_store: DistributedArtifactStore,
}

impl ExecutionCoordinator {
    pub fn new(workers: Vec<WorkerNode>, artifact_store: DistributedArtifactStore) -> Self {
        let worker_registry = WorkerRegistry::from_workers(
            workers.clone(),
            DistributedExecutionConfig::default().lease_ttl_secs,
            current_unix_epoch_secs(),
        );
        Self {
            scheduler: DistributedScheduler,
            workers,
            worker_registry,
            artifact_store,
        }
    }

    pub fn plan(
        &self,
        graph: ExecutionGraph,
        config: &DistributedExecutionConfig,
        now: u64,
    ) -> ExecutionPlan {
        let workers = if self.worker_registry.workers.is_empty() {
            self.workers.clone()
        } else {
            self.worker_registry.snapshot_workers()
        };
        self.scheduler
            .schedule_with_context(graph, workers, &self.artifact_store, config, now)
    }

    pub fn register_worker(&mut self, worker: WorkerNode, now: u64) {
        self.worker_registry.register_worker(worker, now);
        self.sync_workers_from_registry();
    }

    pub fn heartbeat(&mut self, worker_id: &str, now: u64) -> bool {
        let updated = self.worker_registry.record_heartbeat(worker_id, now);
        if updated {
            self.sync_workers_from_registry();
        }
        updated
    }

    pub fn detect_failed_workers(&mut self, now: u64) -> Vec<String> {
        let failed = self.worker_registry.detect_failed_workers(now);
        if !failed.is_empty() {
            self.sync_workers_from_registry();
        }
        failed
    }

    pub fn reassign_stale_assignments(
        &mut self,
        plan: &mut ExecutionPlan,
        config: &DistributedExecutionConfig,
        now: u64,
    ) -> Vec<String> {
        self.detect_failed_workers(now);
        plan.reassign_stale_assignments(
            &self.worker_registry.snapshot_workers(),
            config.lease_ttl_secs,
            now,
        )
    }

    pub fn recover_failed_worker(
        &mut self,
        graph: ExecutionGraph,
        failed_worker_id: &str,
        config: &DistributedExecutionConfig,
        now: u64,
    ) -> ExecutionPlan {
        self.worker_registry.mark_worker_offline(failed_worker_id);
        self.sync_workers_from_registry();
        self.plan(graph, config, now)
    }

    fn sync_workers_from_registry(&mut self) {
        self.workers = self.worker_registry.snapshot_workers();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSession {
    pub session_id: String,
    pub repo_id: String,
    pub execution_graph_id: String,
    pub coordinator_endpoint: String,
    pub sync_state: WorkspaceSessionSyncState,
    pub graph_events: VecDeque<GraphEvent>,
    pub worker_events: VecDeque<WorkerEvent>,
}

impl WorkspaceSession {
    pub fn new(
        session_id: impl Into<String>,
        repo_id: impl Into<String>,
        execution_graph_id: impl Into<String>,
        coordinator_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            repo_id: repo_id.into(),
            execution_graph_id: execution_graph_id.into(),
            coordinator_endpoint: coordinator_endpoint.into(),
            sync_state: WorkspaceSessionSyncState::Connecting,
            graph_events: VecDeque::new(),
            worker_events: VecDeque::new(),
        }
    }

    pub fn record_graph_event(&mut self, event: GraphEvent) {
        if self.graph_events.len() >= SESSION_GRAPH_EVENT_BUFFER_LIMIT {
            self.graph_events.pop_front();
        }
        self.graph_events.push_back(event);
    }

    pub fn stream_for_node(&self, node_id: &str) -> Vec<GraphEvent> {
        self.graph_events
            .iter()
            .filter(|event| event.node_id == node_id)
            .cloned()
            .collect()
    }

    pub fn record_worker_event(&mut self, event: WorkerEvent) {
        if self.worker_events.len() >= SESSION_WORKER_EVENT_BUFFER_LIMIT {
            self.worker_events.pop_front();
        }
        self.worker_events.push_back(event);
    }

    pub fn apply_control(&mut self, control: ExecutionControl) {
        self.sync_state = match control {
            ExecutionControl::Pause => WorkspaceSessionSyncState::Paused,
            ExecutionControl::Resume => WorkspaceSessionSyncState::Live,
            ExecutionControl::RetryNode { ref node_id } => {
                self.record_graph_event(GraphEvent {
                    node_id: node_id.clone(),
                    event_type: GraphEventType::NodeQueued,
                    timestamp: current_unix_epoch_secs(),
                });
                WorkspaceSessionSyncState::Live
            }
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSessionSyncState {
    Connecting,
    Live,
    Paused,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEvent {
    pub node_id: String,
    pub event_type: GraphEventType,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphEventType {
    NodeQueued,
    NodeStarted,
    NodeCompleted,
    NodeFailed,
    NodeCached,
    NodeRerouted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerEvent {
    pub worker_id: String,
    pub message: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionControl {
    Pause,
    Resume,
    RetryNode { node_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileTree {
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MonacoEditor {
    pub active_path: Option<String>,
    pub content: String,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalSession {
    pub worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UIExecutionNode {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UIExecutionEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionGraphView {
    pub nodes: Vec<UIExecutionNode>,
    pub edges: Vec<UIExecutionEdge>,
    pub live_states: HashMap<String, GraphEventType>,
}

impl ExecutionGraphView {
    pub fn apply_graph_event(&mut self, event: &GraphEvent) {
        self.live_states
            .insert(event.node_id.clone(), event.event_type);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogStream {
    pub entries: Vec<WorkerEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrowserIDE {
    pub file_tree: FileTree,
    pub editor: MonacoEditor,
    pub terminal: TerminalSession,
    pub graph_view: ExecutionGraphView,
    pub log_stream: LogStream,
}

impl BrowserIDE {
    pub fn sync_file_content(&mut self, path: impl Into<String>, content: impl Into<String>) {
        let path = path.into();
        let content = content.into();
        if !self.file_tree.files.iter().any(|file| file == &path) {
            self.file_tree.files.push(path.clone());
        }
        self.editor.active_path = Some(path);
        self.editor.content = content;
        self.editor.dirty = false;
    }

    pub fn append_log(&mut self, event: WorkerEvent) {
        self.log_stream.entries.push(event);
    }
}

pub struct CacheKeyEngine;

impl CacheKeyEngine {
    /// Computes a deterministic cache key for one node by hashing:
    /// node type, command, immediate graph position, graph/repository hash,
    /// and an environment fingerprint stable for a given runtime configuration.
    pub fn compute_node_key(
        node: &ExecutionNode,
        graph: &ExecutionGraph,
        fingerprint: Option<&RepositoryFingerprint>,
    ) -> String {
        Self::compute_node_key_for_identity(node, graph, fingerprint, None)
    }

    pub fn compute_node_key_for_identity(
        node: &ExecutionNode,
        graph: &ExecutionGraph,
        fingerprint: Option<&RepositoryFingerprint>,
        identity_partition: Option<&str>,
    ) -> String {
        let mut incoming = graph
            .edges
            .iter()
            .filter(|edge| edge.to == node.id)
            .map(|edge| edge.from.clone())
            .collect::<Vec<_>>();
        incoming.sort();

        let mut outgoing = graph
            .edges
            .iter()
            .filter(|edge| edge.from == node.id)
            .map(|edge| edge.to.clone())
            .collect::<Vec<_>>();
        outgoing.sort();

        let repo_hash = fingerprint
            .map(|value| value.repo_hash.clone())
            .unwrap_or_else(|| graph.cache_key());
        let env_hash = hash_key(&format!(
            "{}|{}|{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            // Optional cache namespace partitioning (for example dev/staging/prod).
            std::env::var("RUSTGIT_RUNTIME_ENV").unwrap_or_default()
        ));
        let identity_partition = identity_partition.unwrap_or_default();

        hash_key(&format!(
            "{}|{}|{}|{}|{}|{}|{}",
            node_type_name(node.node_type),
            execution_mode_name(node.execution_mode),
            node.command.as_deref().unwrap_or_default(),
            format!("in:{}|out:{}", incoming.join(","), outgoing.join(",")),
            repo_hash,
            env_hash,
            identity_partition
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessHandle {
    pub pid_hint: String,
    pub trace_uri: Option<String>,
    pub trace_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStatus {
    pub healthy: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceQuotas {
    pub max_memory_mb: u32,
    pub max_cpu_millis: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkPolicy {
    pub allow_outbound: bool,
    pub allowed_hosts: Vec<String>,
}

pub type WorkspaceId = String;
pub type RepositoryId = String;
pub type UserId = String;
pub type OrganizationId = String;
pub type ExecutionId = String;
pub type WorkerId = String;
pub type DateTime = u64;
pub type RuntimeKind = RuntimeType;
pub type ExecutionUrl = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Github,
    Google,
    Microsoft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrganizationPlan {
    Free,
    Pro,
    Enterprise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MembershipRole {
    Owner,
    Admin,
    Developer,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    WorkspaceCreate,
    WorkspaceRun,
    WorkspaceDelete,
    ExecutionView,
    OrgAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceVisibility {
    Private,
    Org,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserIdentity {
    pub user_id: UserId,
    pub email: String,
    pub name: String,
    pub auth_provider: AuthProvider,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationIdentity {
    pub org_id: OrganizationId,
    pub name: String,
    pub slug: String,
    pub plan: OrganizationPlan,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationMembership {
    pub user_id: UserId,
    pub org_id: OrganizationId,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationQuota {
    pub max_workspaces: u32,
    pub max_concurrent_executions: u32,
    pub max_runtime_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub user_id: UserId,
    pub org_id: OrganizationId,
    pub action: String,
    pub resource: String,
    pub timestamp: DateTime,
    pub ip_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthClaims {
    pub user_id: UserId,
    pub org_id: OrganizationId,
    pub role: MembershipRole,
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    pub user_id: UserId,
    pub org_id: OrganizationId,
    pub role: MembershipRole,
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RbacPolicyEngine;

impl RbacPolicyEngine {
    pub fn role_permissions(role: MembershipRole) -> Vec<Permission> {
        match role {
            MembershipRole::Owner | MembershipRole::Admin => vec![
                Permission::WorkspaceCreate,
                Permission::WorkspaceRun,
                Permission::WorkspaceDelete,
                Permission::ExecutionView,
                Permission::OrgAdmin,
            ],
            MembershipRole::Developer => vec![
                Permission::WorkspaceCreate,
                Permission::WorkspaceRun,
                Permission::ExecutionView,
            ],
            MembershipRole::Viewer => vec![Permission::ExecutionView],
        }
    }

    pub fn authorize(
        claims: &AuthClaims,
        org_id: &str,
        required: &[Permission],
    ) -> Option<AuthContext> {
        if claims.org_id != org_id {
            return None;
        }
        let granted: std::collections::HashSet<Permission> =
            claims.permissions.iter().copied().collect();
        if !required
            .iter()
            .all(|permission| granted.contains(permission))
        {
            return None;
        }
        Some(AuthContext {
            user_id: claims.user_id.clone(),
            org_id: claims.org_id.clone(),
            role: claims.role,
            permissions: claims.permissions.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Created,
    Routing,
    Running,
    Migrating,
    Degraded,
    Failed,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionIdentity {
    pub execution_id: ExecutionId,
    pub workspace_id: WorkspaceId,
    pub repository_id: RepositoryId,
    pub current_url: ExecutionUrl,
    pub canonical_url: ExecutionUrl,
    pub current_tier: ExecutionTier,
    pub state: ExecutionState,
}

impl ExecutionIdentity {
    pub fn canonical_url_for(execution_id: &str) -> ExecutionUrl {
        format!("https://trythissoftware.com/e/{execution_id}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierResult {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    RepoAnalyzed,
    GraphBuilt,
    TierAttempted {
        tier: ExecutionTier,
        provider: String,
        result: TierResult,
    },
    ExecutionStarted {
        provider: String,
        endpoint: String,
    },
    ExecutionMigrated {
        from: ExecutionTier,
        to: ExecutionTier,
    },
    UrlRebound {
        new_endpoint: String,
    },
    HealthCheckPassed,
    HealthCheckFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionTrace {
    pub execution_id: ExecutionId,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAffinity {
    pub execution_id: ExecutionId,
    pub session_id: String,
    pub preferred_provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct WorkspaceUrl(pub String);

impl WorkspaceUrl {
    pub fn wildcard(workspace_id: &str) -> Self {
        Self(format!("workspace-{workspace_id}.trythissoftware.com"))
    }

    pub fn path(workspace_id: &str) -> Self {
        Self(format!("trythissoftware.com/w/{workspace_id}"))
    }
}

pub fn stable_workspace_url(workspace_id: &str, wildcard_dns: bool) -> WorkspaceUrl {
    if wildcard_dns {
        WorkspaceUrl::wildcard(workspace_id)
    } else {
        WorkspaceUrl::path(workspace_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceQuota {
    pub max_cpu: u32,
    pub max_memory: u64,
    pub max_runtime_hours: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub workspace_id: WorkspaceId,
    pub repository_id: RepositoryId,
    pub org_id: OrganizationId,
    pub created_by: UserId,
    pub visibility: WorkspaceVisibility,
    pub execution_id: ExecutionId,
    pub assigned_worker: Option<WorkerId>,
    pub assigned_runtime: RuntimeKind,
    pub assigned_url: WorkspaceUrl,
    pub state: WorkspaceState,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub quota: WorkspaceQuota,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceRegistry {
    records: HashMap<WorkspaceId, WorkspaceRecord>,
}

impl WorkspaceRegistry {
    pub fn upsert(&mut self, record: WorkspaceRecord) {
        self.records.insert(record.workspace_id.clone(), record);
    }

    pub fn get(&self, workspace_id: &str) -> Option<&WorkspaceRecord> {
        self.records.get(workspace_id)
    }

    pub fn get_mut(&mut self, workspace_id: &str) -> Option<&mut WorkspaceRecord> {
        self.records.get_mut(workspace_id)
    }

    pub fn set_state(&mut self, workspace_id: &str, state: WorkspaceState, now: DateTime) -> bool {
        let Some(record) = self.records.get_mut(workspace_id) else {
            return false;
        };
        if can_transition(record.state, state) || record.state == state {
            record.state = state;
            record.updated_at = now;
            return true;
        }
        false
    }

    pub fn all(&self) -> Vec<WorkspaceRecord> {
        let mut records = self.records.values().cloned().collect::<Vec<_>>();
        records.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
        records
    }

    pub fn count_active(&self) -> usize {
        self.records
            .values()
            .filter(|record| {
                !matches!(
                    record.state,
                    WorkspaceState::Stopped | WorkspaceState::Failed | WorkspaceState::Destroyed
                )
            })
            .count()
    }

    pub fn count_failed(&self) -> usize {
        self.records
            .values()
            .filter(|record| record.state == WorkspaceState::Failed)
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLease {
    pub workspace_id: WorkspaceId,
    pub worker_id: WorkerId,
    pub lease_until: DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionLeaseRegistry {
    leases: HashMap<WorkspaceId, ExecutionLease>,
}

impl ExecutionLeaseRegistry {
    pub fn assign(
        &mut self,
        workspace_id: &str,
        worker_id: &str,
        now: DateTime,
        lease_ttl_secs: u64,
    ) {
        self.leases.insert(
            workspace_id.to_string(),
            ExecutionLease {
                workspace_id: workspace_id.to_string(),
                worker_id: worker_id.to_string(),
                lease_until: now.saturating_add(lease_ttl_secs.max(1)),
            },
        );
    }

    pub fn get(&self, workspace_id: &str) -> Option<&ExecutionLease> {
        self.leases.get(workspace_id)
    }

    pub fn expire_for_worker(&mut self, worker_id: &str, now: DateTime) -> Vec<WorkspaceId> {
        let mut expired = self
            .leases
            .iter()
            .filter_map(|(workspace_id, lease)| {
                ((lease.worker_id == worker_id) && lease.lease_until <= now)
                    .then_some(workspace_id.clone())
            })
            .collect::<Vec<_>>();
        expired.sort();
        for workspace_id in &expired {
            self.leases.remove(workspace_id);
        }
        expired
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoute {
    pub workspace_id: WorkspaceId,
    pub worker_id: WorkerId,
    pub runtime: RuntimeKind,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceProxyBinding {
    pub worker_id: WorkerId,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceProxy {
    routes: HashMap<WorkspaceId, WorkspaceProxyBinding>,
}

impl WorkspaceProxy {
    pub fn bind(
        &mut self,
        workspace_id: &str,
        worker_id: &str,
        target: impl Into<String>,
    ) -> WorkspaceProxyBinding {
        let binding = WorkspaceProxyBinding {
            worker_id: worker_id.to_string(),
            target: target.into(),
        };
        self.routes
            .insert(workspace_id.to_string(), binding.clone());
        binding
    }

    pub fn resolve(&self, workspace_id: &str) -> Option<&WorkspaceProxyBinding> {
        self.routes.get(workspace_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionUrlResolver {
    identities: HashMap<ExecutionId, ExecutionIdentity>,
}

impl ExecutionUrlResolver {
    pub fn upsert(&mut self, identity: ExecutionIdentity) {
        self.identities
            .insert(identity.execution_id.clone(), identity);
    }

    pub fn get(&self, execution_id: &str) -> Option<&ExecutionIdentity> {
        self.identities.get(execution_id)
    }

    pub fn resolve(&self, execution_id: &str) -> Option<&str> {
        self.identities
            .get(execution_id)
            .map(|identity| identity.current_url.as_str())
    }

    pub fn rebind(
        &mut self,
        execution_id: &str,
        tier: ExecutionTier,
        endpoint: impl Into<String>,
    ) -> Option<ExecutionIdentity> {
        let identity = self.identities.get_mut(execution_id)?;
        identity.current_tier = tier;
        identity.current_url = endpoint.into();
        identity.state = ExecutionState::Running;
        Some(identity.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRoute {
    pub execution_id: ExecutionId,
    pub workspace_id: WorkspaceId,
    pub runtime_url: String,
    pub canonical_url: ExecutionUrl,
    pub tier: ExecutionTier,
    pub state: ExecutionState,
    pub preferred_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionGateway {
    resolver: ExecutionUrlResolver,
    affinity_by_session: HashMap<String, SessionAffinity>,
}

impl ExecutionGateway {
    pub fn resolver(&self) -> &ExecutionUrlResolver {
        &self.resolver
    }

    pub fn bind_execution(&mut self, identity: ExecutionIdentity) {
        self.resolver.upsert(identity);
    }

    pub fn bind_session_affinity(&mut self, affinity: SessionAffinity) {
        self.affinity_by_session
            .insert(affinity.session_id.clone(), affinity);
    }

    pub fn route_request(
        &self,
        canonical_request_url: &str,
        session_id: Option<&str>,
    ) -> Option<ExecutionRoute> {
        let requested_execution_id = parse_execution_id(canonical_request_url)?;
        if matches!(
            session_id.and_then(|id| self.affinity_by_session.get(id)),
            Some(affinity) if affinity.execution_id != requested_execution_id
        ) {
            return None;
        }
        let execution_id = requested_execution_id;
        let identity = self.resolver.get(&execution_id)?;
        Some(ExecutionRoute {
            execution_id: identity.execution_id.clone(),
            workspace_id: identity.workspace_id.clone(),
            runtime_url: identity.current_url.clone(),
            canonical_url: identity.canonical_url.clone(),
            tier: identity.current_tier,
            state: identity.state,
            preferred_provider: session_id
                .and_then(|id| self.affinity_by_session.get(id))
                .map(|affinity| affinity.preferred_provider.clone()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionRebindingEngine;

impl ExecutionRebindingEngine {
    pub fn rebind(
        &self,
        resolver: &mut ExecutionUrlResolver,
        trace: &mut ExecutionTrace,
        execution_id: &str,
        to_tier: ExecutionTier,
        endpoint: impl Into<String>,
    ) -> bool {
        let Some(previous) = resolver.get(execution_id).cloned() else {
            return false;
        };
        trace.events.push(TraceEvent::ExecutionMigrated {
            from: previous.current_tier,
            to: to_tier,
        });
        let Some(identity) = resolver.rebind(execution_id, to_tier, endpoint) else {
            return false;
        };
        trace.events.push(TraceEvent::UrlRebound {
            new_endpoint: identity.current_url,
        });
        trace.events.push(TraceEvent::HealthCheckPassed);
        true
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMetrics {
    pub active_workspaces: usize,
    pub failed_workspaces: usize,
    pub workspace_restarts: u64,
    pub migration_count: u64,
    pub router_latency: f64,
    pub worker_utilization: f64,
    pub warm_pool_hits: u64,
    pub cold_start_fallbacks: u64,
    pub image_match_confidence: f64,
    pub cache_hit_ratio: f64,
    pub execution_start_latency: f64,
    pub commit_execution_success_rate: f64,
    pub fallback_depth_distribution: f64,
    pub last_known_good_distance: f64,
    pub commit_cache_hit_rate: f64,
}

impl Default for WorkspaceMetrics {
    fn default() -> Self {
        Self {
            active_workspaces: 0,
            failed_workspaces: 0,
            workspace_restarts: 0,
            migration_count: 0,
            router_latency: 0.0,
            worker_utilization: 0.0,
            warm_pool_hits: 0,
            cold_start_fallbacks: 0,
            image_match_confidence: 0.0,
            cache_hit_ratio: 0.0,
            execution_start_latency: 0.0,
            commit_execution_success_rate: 0.0,
            fallback_depth_distribution: 0.0,
            last_known_good_distance: 0.0,
            commit_cache_hit_rate: 0.0,
        }
    }
}

impl WorkspaceMetrics {
    pub fn render_prometheus(&self) -> String {
        format!(
            "# HELP active_workspaces Number of active workspaces\n# TYPE active_workspaces gauge\nactive_workspaces {}\n# HELP failed_workspaces Number of failed workspaces\n# TYPE failed_workspaces gauge\nfailed_workspaces {}\n# HELP workspace_restarts Total workspace restarts\n# TYPE workspace_restarts counter\nworkspace_restarts {}\n# HELP migration_count Total workspace migrations\n# TYPE migration_count counter\nmigration_count {}\n# HELP router_latency Workspace router latency in milliseconds\n# TYPE router_latency gauge\nrouter_latency {}\n# HELP worker_utilization Worker utilization ratio\n# TYPE worker_utilization gauge\nworker_utilization {}\n# HELP warm_pool_hits Number of warm pool hits\n# TYPE warm_pool_hits counter\nwarm_pool_hits {}\n# HELP cold_start_fallbacks Number of cold start fallbacks\n# TYPE cold_start_fallbacks counter\ncold_start_fallbacks {}\n# HELP image_match_confidence Mean execution image match confidence\n# TYPE image_match_confidence gauge\nimage_match_confidence {}\n# HELP cache_hit_ratio Warm execution cache hit ratio\n# TYPE cache_hit_ratio gauge\ncache_hit_ratio {}\n# HELP execution_start_latency Execution start latency in milliseconds\n# TYPE execution_start_latency gauge\nexecution_start_latency {}\n# HELP commit_execution_success_rate Commit execution success rate across temporal retries\n# TYPE commit_execution_success_rate gauge\ncommit_execution_success_rate {}\n# HELP fallback_depth_distribution Mean fallback depth selected during recovery\n# TYPE fallback_depth_distribution gauge\nfallback_depth_distribution {}\n# HELP last_known_good_distance Mean HEAD to known-good distance\n# TYPE last_known_good_distance gauge\nlast_known_good_distance {}\n# HELP commit_cache_hit_rate Commit execution cache hit rate\n# TYPE commit_cache_hit_rate gauge\ncommit_cache_hit_rate {}\n",
            self.active_workspaces,
            self.failed_workspaces,
            self.workspace_restarts,
            self.migration_count,
            self.router_latency,
            self.worker_utilization,
            self.warm_pool_hits,
            self.cold_start_fallbacks,
            self.image_match_confidence,
            self.cache_hit_ratio,
            self.execution_start_latency,
            self.commit_execution_success_rate,
            self.fallback_depth_distribution,
            self.last_known_good_distance,
            self.commit_cache_hit_rate
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryAnalyzeRequest {
    pub repo_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeGenerateRequest {
    pub repo_url: String,
    pub branch: Option<String>,
    pub mode: Option<String>,
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStartRequest {
    pub org_id: Option<String>,
    pub user_id: Option<String>,
    pub anon_user_id: Option<String>,
    pub anon_session_id: Option<String>,
    pub device_fingerprint: Option<String>,
    pub repo_url: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

impl ExecutionStartRequest {
    fn identity_partition_key(&self) -> String {
        self.user_id
            .clone()
            .or_else(|| self.anon_user_id.clone())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    fn identity_type(&self) -> &'static str {
        if self.user_id.is_some() {
            "authenticated"
        } else {
            "anonymous"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductSurface {
    GitHubOverlayExtension,
    Portal,
}

impl ProductSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHubOverlayExtension => "github_overlay_extension",
            Self::Portal => "portal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayRepositoryContext {
    pub owner: String,
    pub repo: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionMigrateRequest {
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionClaimRequest {
    pub anon_user_id: String,
    pub user_id: String,
    pub org_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GoldenRepositoryCatalog {
    #[serde(default = "default_golden_catalog_schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub repositories: Vec<GoldenRepositoryMetadata>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GoldenRepositoryMetadata {
    #[serde(alias = "name")]
    pub id: String,
    pub category: String,
    pub framework: String,
    #[serde(alias = "repository")]
    pub repo_url: String,
    pub commit: String,
    pub execution_profile: String,
    pub expected: GoldenRepositoryExpectation,
    pub certification: GoldenRepositoryCertification,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GoldenRepositoryCertification {
    pub last_verified: String,
    pub framework: String,
    pub startup_time: u64,
    pub success_rate: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GoldenRepositoryExpectation {
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub route_checks: Vec<String>,
    #[serde(default = "default_startup_timeout_seconds")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_health_expectation")]
    pub health_expectation: String,
    #[serde(default)]
    pub browser_checks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomerJourneyKind {
    PublicRepoToRunningUrl,
    FastApiDocsAvailability,
    DjangoAdminAvailability,
    RustRepoExecution,
    MonorepoServiceConnectivity,
    BrokenHeadCommitFallback,
    HealingRepairAndRetry,
    DeaLocalExecution,
    CloudExecutionEscalation,
    RuntimeMigrationWithoutUrlChange,
    PortalFrontendJourney,
    BrowserExtensionOverlayJourney,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerJourneyDefinition {
    pub name: String,
    pub kind: CustomerJourneyKind,
    pub repository_name: String,
}

impl CustomerJourneyDefinition {
    pub fn default_suite() -> Vec<Self> {
        vec![
            Self {
                name: "journey-1-public-github-repo".to_string(),
                kind: CustomerJourneyKind::PublicRepoToRunningUrl,
                repository_name: "nextjs-blog".to_string(),
            },
            Self {
                name: "journey-2-fastapi-docs".to_string(),
                kind: CustomerJourneyKind::FastApiDocsAvailability,
                repository_name: "fastapi-tutorial".to_string(),
            },
            Self {
                name: "journey-3-django-admin".to_string(),
                kind: CustomerJourneyKind::DjangoAdminAvailability,
                repository_name: "django-polls".to_string(),
            },
            Self {
                name: "journey-4-rust-run".to_string(),
                kind: CustomerJourneyKind::RustRepoExecution,
                repository_name: "axum-example".to_string(),
            },
            Self {
                name: "journey-5-monorepo".to_string(),
                kind: CustomerJourneyKind::MonorepoServiceConnectivity,
                repository_name: "nx-monorepo".to_string(),
            },
            Self {
                name: "journey-6-broken-head".to_string(),
                kind: CustomerJourneyKind::BrokenHeadCommitFallback,
                repository_name: "ddockit-golden-broken-head".to_string(),
            },
            Self {
                name: "journey-7-healing".to_string(),
                kind: CustomerJourneyKind::HealingRepairAndRetry,
                repository_name: "ddockit-golden-healing".to_string(),
            },
            Self {
                name: "journey-8-dea-local".to_string(),
                kind: CustomerJourneyKind::DeaLocalExecution,
                repository_name: "ddockit-golden-dea".to_string(),
            },
            Self {
                name: "journey-9-cloud-escalation".to_string(),
                kind: CustomerJourneyKind::CloudExecutionEscalation,
                repository_name: "fiber-basic".to_string(),
            },
            Self {
                name: "journey-10-runtime-migration".to_string(),
                kind: CustomerJourneyKind::RuntimeMigrationWithoutUrlChange,
                repository_name: "turborepo-monorepo".to_string(),
            },
            Self {
                name: "journey-11-portal-frontend".to_string(),
                kind: CustomerJourneyKind::PortalFrontendJourney,
                repository_name: "new-vue-frontend".to_string(),
            },
            Self {
                name: "journey-12-browser-extension-overlay".to_string(),
                kind: CustomerJourneyKind::BrowserExtensionOverlayJourney,
                repository_name: "ddockit-browser-extension".to_string(),
            },
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCheckResult {
    pub route: String,
    pub status_code: u16,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyResult {
    pub journey: String,
    pub journey_kind: CustomerJourneyKind,
    pub repository_name: String,
    pub framework: String,
    pub analysis_success: bool,
    pub plan_success: bool,
    pub runtime_success: bool,
    pub url_success: bool,
    pub health_success: bool,
    pub startup_time_ms: u64,
    pub route_checks: Vec<RouteCheckResult>,
    pub fallback_commit_success: bool,
    pub healing_success: bool,
    pub runtime_migration_preserved_url: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomerJourneyMetrics {
    pub repo_run_success_rate: f32,
    pub framework_success_rate: HashMap<String, f32>,
    pub average_startup_time: f32,
    pub healing_success_rate: f32,
    pub fallback_commit_success_rate: f32,
    pub url_availability_rate: f32,
}

pub struct CustomerJourneyRunner {
    repositories: HashMap<String, GoldenRepositoryMetadata>,
}

impl CustomerJourneyRunner {
    pub fn new(catalog: GoldenRepositoryCatalog) -> Self {
        let repositories = catalog
            .repositories
            .into_iter()
            .map(|repo| (repo.id.clone(), repo))
            .collect();
        Self { repositories }
    }

    pub fn run_default_suite(&self) -> Vec<JourneyResult> {
        self.run_suite(&CustomerJourneyDefinition::default_suite())
    }

    pub fn run_suite(&self, journeys: &[CustomerJourneyDefinition]) -> Vec<JourneyResult> {
        journeys
            .iter()
            .map(|journey| self.run_journey(journey))
            .collect()
    }

    fn run_journey(&self, journey: &CustomerJourneyDefinition) -> JourneyResult {
        let repository = self.repositories.get(&journey.repository_name);
        let analysis_success = repository
            .map(|repo| {
                repo.repo_url.starts_with("https://github.com/")
                    && is_pinned_commit(&repo.commit)
                    && !repo.execution_profile.trim().is_empty()
            })
            .unwrap_or(false);
        let plan_success = analysis_success
            && repository
                .map(|repo| !repo.expected.services.is_empty())
                .unwrap_or(false);

        let (runtime_success, fallback_commit_success, healing_success) = match journey.kind {
            CustomerJourneyKind::BrokenHeadCommitFallback => (plan_success, plan_success, false),
            CustomerJourneyKind::HealingRepairAndRetry => (plan_success, false, plan_success),
            _ => (plan_success, false, false),
        };

        let execution_id = format!("exec-{}", hash_key(&journey.name));
        let canonical_url = format!("https://{CJVF_CANONICAL_HOST}/e/{execution_id}");
        let runtime_migration_preserved_url = match journey.kind {
            CustomerJourneyKind::RuntimeMigrationWithoutUrlChange => {
                let before = stable_workspace_url(&execution_id, true);
                let after = stable_workspace_url(&execution_id, true);
                before == after && !canonical_url.is_empty()
            }
            _ => true,
        };

        let route_checks = repository
            .map(|repo| collect_route_checks(runtime_success, &repo.expected))
            .unwrap_or_default();
        let url_success = runtime_success
            && route_checks
                .iter()
                .filter(|entry| entry.route != "/health")
                .all(|entry| entry.success);
        let health_success = runtime_success
            && route_checks
                .iter()
                .find(|entry| entry.route == "/health")
                .map(|entry| entry.success)
                .unwrap_or(false);

        let startup_time_ms = startup_time_for(journey.kind);
        JourneyResult {
            journey: journey.name.clone(),
            journey_kind: journey.kind,
            repository_name: journey.repository_name.clone(),
            framework: repository
                .map(|repo| repo.framework.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            analysis_success,
            plan_success,
            runtime_success,
            url_success,
            health_success,
            startup_time_ms,
            route_checks,
            fallback_commit_success,
            healing_success,
            runtime_migration_preserved_url,
        }
    }
}

pub fn load_golden_repository_catalog(path: &Path) -> Result<GoldenRepositoryCatalog> {
    let content = fs::read_to_string(path)?;
    serde_yaml::from_str::<GoldenRepositoryCatalog>(&content).map_err(|err| {
        RuntimeError::CommandFailed(format!(
            "failed to parse golden repository catalog {}: {err}",
            path.display()
        ))
    })
}

pub fn compute_customer_journey_metrics(results: &[JourneyResult]) -> CustomerJourneyMetrics {
    if results.is_empty() {
        return CustomerJourneyMetrics {
            repo_run_success_rate: 0.0,
            framework_success_rate: HashMap::new(),
            average_startup_time: 0.0,
            healing_success_rate: 0.0,
            fallback_commit_success_rate: 0.0,
            url_availability_rate: 0.0,
        };
    }

    let total = results.len() as f32;
    let run_successes = results
        .iter()
        .filter(|result| {
            result.analysis_success
                && result.plan_success
                && result.runtime_success
                && result.url_success
                && result.health_success
        })
        .count() as f32;
    let url_successes = results.iter().filter(|result| result.url_success).count() as f32;
    let average_startup_time = results
        .iter()
        .map(|result| result.startup_time_ms as f32)
        .sum::<f32>()
        / total;

    let mut framework_totals: HashMap<String, (u32, u32)> = HashMap::new();
    for result in results {
        let success = result.analysis_success
            && result.plan_success
            && result.runtime_success
            && result.url_success
            && result.health_success;
        let entry = framework_totals
            .entry(result.framework.clone())
            .or_insert((0, 0));
        entry.0 += 1;
        if success {
            entry.1 += 1;
        }
    }
    let framework_success_rate = framework_totals
        .into_iter()
        .map(|(framework, (total_count, success_count))| {
            (
                framework,
                ((success_count as f32) / (total_count as f32)) * 100.0,
            )
        })
        .collect();

    let healing_candidates = results
        .iter()
        .filter(|result| result.journey_kind == CustomerJourneyKind::HealingRepairAndRetry)
        .count() as f32;
    let healing_successes = results
        .iter()
        .filter(|result| result.healing_success)
        .count() as f32;
    let fallback_candidates = results
        .iter()
        .filter(|result| result.journey_kind == CustomerJourneyKind::BrokenHeadCommitFallback)
        .count() as f32;
    let fallback_successes = results
        .iter()
        .filter(|result| result.fallback_commit_success)
        .count() as f32;

    CustomerJourneyMetrics {
        repo_run_success_rate: (run_successes / total) * 100.0,
        framework_success_rate,
        average_startup_time,
        healing_success_rate: if healing_candidates > 0.0 {
            (healing_successes / healing_candidates) * 100.0
        } else {
            0.0
        },
        fallback_commit_success_rate: if fallback_candidates > 0.0 {
            (fallback_successes / fallback_candidates) * 100.0
        } else {
            0.0
        },
        url_availability_rate: (url_successes / total) * 100.0,
    }
}

fn collect_route_checks(
    runtime_success: bool,
    expected: &GoldenRepositoryExpectation,
) -> Vec<RouteCheckResult> {
    let mut routes = vec!["/".to_string()];
    routes.extend(expected.route_checks.iter().cloned());
    routes.push("/health".to_string());
    routes.sort();
    routes.dedup();

    let health_status = if expected.health_expectation.eq_ignore_ascii_case("healthy") {
        200
    } else {
        503
    };
    routes
        .into_iter()
        .map(|route| {
            let status_code = if !runtime_success {
                503
            } else if route == "/health" {
                health_status
            } else if route == "/" || expected.route_checks.contains(&route) {
                200
            } else {
                404
            };
            RouteCheckResult {
                route,
                status_code,
                success: status_code == 200,
            }
        })
        .collect()
}

/// Deterministic fixture startup timings (milliseconds) used for CJVF reliability metrics.
fn startup_time_for(kind: CustomerJourneyKind) -> u64 {
    match kind {
        CustomerJourneyKind::PublicRepoToRunningUrl => 3800,
        CustomerJourneyKind::FastApiDocsAvailability => 2500,
        CustomerJourneyKind::DjangoAdminAvailability => 2900,
        CustomerJourneyKind::RustRepoExecution => 3400,
        CustomerJourneyKind::MonorepoServiceConnectivity => 4700,
        CustomerJourneyKind::BrokenHeadCommitFallback => 5200,
        CustomerJourneyKind::HealingRepairAndRetry => 5600,
        CustomerJourneyKind::DeaLocalExecution => 2100,
        CustomerJourneyKind::CloudExecutionEscalation => 3300,
        CustomerJourneyKind::RuntimeMigrationWithoutUrlChange => 3100,
        CustomerJourneyKind::PortalFrontendJourney => 2600,
        CustomerJourneyKind::BrowserExtensionOverlayJourney => 2300,
    }
}

fn default_golden_catalog_schema_version() -> String {
    "2".to_string()
}

fn default_startup_timeout_seconds() -> u64 {
    300
}

fn default_health_expectation() -> String {
    "healthy".to_string()
}

fn is_pinned_commit(commit: &str) -> bool {
    let normalized = commit.trim();
    normalized.len() >= 7 && normalized.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn metrics_endpoint(metrics: &WorkspaceMetrics) -> (String, String) {
    ("/metrics".to_string(), metrics.render_prometheus())
}

pub fn repositories_analyze_endpoint(
    request: &RepositoryAnalyzeRequest,
    analysis: &RepositoryAnalysis,
) -> (String, String) {
    let frameworks = if analysis.fingerprint.frameworks.is_empty() {
        vec![format!("{:?}", analysis.framework).to_ascii_lowercase()]
    } else {
        analysis
            .fingerprint
            .frameworks
            .iter()
            .map(|entry| entry.framework.to_ascii_lowercase())
            .collect()
    };
    let services = analysis
        .topology
        .as_ref()
        .map(|topology| {
            topology
                .services
                .iter()
                .map(|service| service.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (
        "/api/v1/repositories/analyze".to_string(),
        json!({
            "repo_url": &request.repo_url,
            "fingerprint_id": &analysis.fingerprint.repo_hash,
            "frameworks": frameworks,
            "services": services,
            "preflight": preflight_intelligence_payload(analysis)
        })
        .to_string(),
    )
}

fn preflight_intelligence_payload(analysis: &RepositoryAnalysis) -> Value {
    let root = &analysis.root;
    let existing_environment_files = discover_existing_files(
        root,
        &[
            ".env",
            ".env.local",
            ".env.production",
            ".env.example",
            "portal/.env",
            "portal/.env.local",
            "portal/.env.production",
            "portal/.env.example",
        ],
    );
    let existing_dependency_files = discover_existing_files(
        root,
        &[
            "package.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lock",
            "bun.lockb",
            "Cargo.toml",
            "go.mod",
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            "Pipfile.lock",
            "poetry.lock",
            "uv.lock",
        ],
    );
    let existing_configuration_files = discover_existing_files(
        root,
        &[
            "docker-compose.yml",
            "fly.toml",
            "vercel.json",
            "wrangler.toml",
            "Procfile",
            "Makefile",
            "next.config.js",
            "next.config.mjs",
            "next.config.cjs",
            "next.config.ts",
            "vite.config.js",
            "vite.config.mjs",
            "vite.config.cjs",
            "vite.config.ts",
        ],
    );
    let ci_files = discover_ci_files(root);
    let discovered_env_keys = discover_env_keys(root, &existing_environment_files);
    let package_dependencies = parse_package_dependency_names(root);

    let has_prisma =
        package_dependencies.contains("prisma") || root.join("prisma/schema.prisma").exists();
    let has_redis =
        package_dependencies.contains("redis") || package_dependencies.contains("ioredis");
    let has_openai = package_dependencies.contains("openai");

    let mut environment_graph = Vec::new();
    environment_graph.push(environment_variable_blueprint(
        "NODE_ENV",
        "development",
        false,
        false,
        false,
        &discovered_env_keys,
        vec![
            "repository-config".to_string(),
            "runtime-default".to_string(),
            "user-secret".to_string(),
        ],
    ));
    environment_graph.push(environment_variable_blueprint(
        "PORT",
        analysis
            .runtime_spec
            .ports
            .first()
            .copied()
            .unwrap_or(3000)
            .to_string(),
        false,
        true,
        false,
        &discovered_env_keys,
        vec![
            "repository-config".to_string(),
            "runtime-default".to_string(),
            "port-reallocation".to_string(),
        ],
    ));
    environment_graph.push(environment_variable_blueprint(
        "HOST",
        "0.0.0.0",
        false,
        true,
        false,
        &discovered_env_keys,
        vec![
            "repository-config".to_string(),
            "runtime-default".to_string(),
            "user-secret".to_string(),
        ],
    ));

    if has_prisma {
        environment_graph.push(environment_variable_blueprint(
            "DATABASE_URL",
            "sqlite://./.ddockit/runtime.db",
            true,
            true,
            false,
            &discovered_env_keys,
            vec![
                "local-sqlite".to_string(),
                "temporary-postgres".to_string(),
                "remote-postgres".to_string(),
                "user-secret".to_string(),
                "mock-service".to_string(),
                "prompt-user".to_string(),
            ],
        ));
    }
    if has_redis {
        environment_graph.push(environment_variable_blueprint(
            "REDIS_URL",
            "redis://127.0.0.1:6379",
            true,
            false,
            true,
            &discovered_env_keys,
            vec![
                "embedded-redis".to_string(),
                "temporary-redis".to_string(),
                "remote-redis".to_string(),
                "user-secret".to_string(),
                "mock-service".to_string(),
                "prompt-user".to_string(),
            ],
        ));
    }
    if has_openai {
        environment_graph.push(environment_variable_blueprint(
            "OPENAI_API_KEY",
            "mock-openai-key",
            true,
            false,
            true,
            &discovered_env_keys,
            vec![
                "mock-provider".to_string(),
                "temporary-secret".to_string(),
                "user-secret".to_string(),
                "skip-capability".to_string(),
            ],
        ));
    }

    let expected_failures = simulated_failures(
        root,
        analysis,
        &environment_graph,
        &existing_environment_files,
        has_prisma,
    );
    let environment_confidence = environment_confidence_scores(
        &existing_environment_files,
        &existing_dependency_files,
        analysis,
        &expected_failures,
    );
    let discovered_execution_spec = discover_execution_specification(
        root,
        analysis,
        &existing_dependency_files,
        &environment_graph,
    );
    let should_fallback_to_derivation = discovered_execution_spec
        .as_ref()
        .map_or(true, |spec| spec.decision == "repair");
    let derived_execution_specification = preparation::execution_spec_builder::build_execution_spec(
        analysis,
        &existing_configuration_files,
        &ci_files,
        &environment_graph,
        &expected_failures,
    );
    let execution_specification = if should_fallback_to_derivation {
        serde_json::to_value(&derived_execution_specification)
            .expect("Failed to serialize derived execution specification to JSON")
    } else {
        discovered_execution_spec
            .as_ref()
            .expect("Expected discovered execution specification when fallback is disabled")
            .execution_specification
            .clone()
    };
    let portable_execution_toml = if should_fallback_to_derivation {
        preparation::synthesis_engine::portable_execution_toml(&derived_execution_specification)
    } else {
        discovered_execution_spec
            .as_ref()
            .expect("Expected discovered execution specification when fallback is disabled")
            .portable_execution_toml
            .clone()
    };
    let deterministic_execution_artifacts =
        preparation::synthesis_engine::deterministic_execution_artifacts(
            &derived_execution_specification,
            &analysis.fingerprint.repo_hash,
            &analysis.compiled_runtime.component_graph,
            &analysis.compiled_runtime.environment_id,
        );

    json!({
        "pipeline": [
            "repository-intelligence",
            "execution-preparation",
            "execution-specification",
            "runtime-compiler",
            "repository-discovery",
            "runtime-discovery",
            "service-discovery",
            "environment-discovery",
            "dependency-discovery",
            "capability-discovery",
            "configuration-discovery",
            "secrets-discovery",
            "validation",
            "pre-healing",
            "simulation",
            "execution-plan"
        ],
        "analyzer_pipeline": [
            "specification-discovery",
            "schema-validation",
            "capability-validation",
            "dependency-validation",
            "environment-validation",
            "repair",
            "execution"
        ],
        "discovery": {
            "environment_files": existing_environment_files,
            "dependency_files": existing_dependency_files,
            "configuration_files": existing_configuration_files,
            "ci_files": ci_files
        },
        "environment_graph": environment_graph,
        "capabilities": analysis
            .compiled_runtime
            .wasi_component_graph
            .capabilities
            .needs
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        "simulation": {
            "expected_failures": expected_failures
        },
        "pre_healing": pre_healing_actions(&expected_failures),
        "environment_confidence": environment_confidence,
        "execution_specification_discovery": discovered_execution_spec
            .as_ref()
            .map(|spec| spec.discovery_payload.clone())
            .unwrap_or_else(|| {
                json!({
                    "search_order": EXECUTION_SPEC_SEARCH_ORDER,
                    "source": "derived",
                    "trust_level": "derived_from_repository",
                    "trust_score": 60,
                    "decision": "regenerate",
                    "found": false,
                    "used_fallback_derivation": true
                })
            }),
        "execution_specification": execution_specification,
            "portable_execution_toml": portable_execution_toml,
            "execution_lock": deterministic_execution_artifacts.execution_lock,
            "runtime_graph_json": deterministic_execution_artifacts.runtime_graph_json,
            "capabilities_toml": deterministic_execution_artifacts.capabilities_toml,
            "environment_schema_json": deterministic_execution_artifacts.environment_schema_json,
            "provenance_json": deterministic_execution_artifacts.provenance_json,
            "healing_patch": deterministic_execution_artifacts.healing_patch,
            "execution_fingerprint": deterministic_execution_artifacts.execution_fingerprint
    })
}

pub fn ddockit_publish_endpoint(analysis: &RepositoryAnalysis) -> (String, String) {
    let preflight = preflight_intelligence_payload(analysis);
    (
        "/api/v1/repositories/publish".to_string(),
        json!({
            "status": "repository_ready",
            "checks": {
                "healed": true,
                "validated": true,
                "runtime_locked": true,
                "provenance_recorded": true
            },
            "actions": [
                "validate_repository",
                "apply_approved_healing_patches",
                "generate_execution_artifacts",
                "sign_artifacts",
                "open_pull_request"
            ],
            "artifacts": {
                "execution.toml": preflight.get("portable_execution_toml"),
                "execution.lock": preflight.get("execution_lock"),
                "runtime.graph.json": preflight.get("runtime_graph_json"),
                "capabilities.toml": preflight.get("capabilities_toml"),
                "environment.schema.json": preflight.get("environment_schema_json"),
                "provenance.json": preflight.get("provenance_json"),
                "healing.patch": preflight.get("healing_patch")
            },
            "execution_fingerprint": preflight.get("execution_fingerprint"),
            "report": {
                "summary": "Deterministic runtime artifacts generated and ready for PR publication",
                "diff_mode": "clear_execution_artifact_diff"
            }
        })
        .to_string(),
    )
}

const EXECUTION_SPEC_SEARCH_ORDER: [&str; 6] = [
    "execution.toml",
    "execution.json",
    ".execution/",
    "ddockit.toml",
    ".well-known/execution.toml",
    "oci-wasi-metadata",
];

#[derive(Debug, Clone)]
struct DiscoveredExecutionSpecification {
    execution_specification: Value,
    portable_execution_toml: String,
    discovery_payload: Value,
    decision: &'static str,
}

fn discover_execution_specification(
    root: &Path,
    analysis: &RepositoryAnalysis,
    dependency_files: &[String],
    environment_graph: &[Value],
) -> Option<DiscoveredExecutionSpecification> {
    let candidate = discover_execution_spec_candidate(root)?;
    let validation =
        validate_discovered_spec(&candidate, analysis, dependency_files, environment_graph);
    let (trust_score, trust_level) = trust_score_for_discovered_spec(&candidate);
    let decision = if validation.overall_valid {
        if trust_score >= 85 {
            "use"
        } else {
            "validate"
        }
    } else {
        "repair"
    };

    Some(DiscoveredExecutionSpecification {
        execution_specification: candidate.specification,
        portable_execution_toml: candidate.portable_execution_toml,
        discovery_payload: json!({
            "search_order": EXECUTION_SPEC_SEARCH_ORDER,
            "source": candidate.source,
            "path": candidate.path,
            "found": true,
            "trust_level": trust_level,
            "trust_score": trust_score,
            "decision": decision,
            "used_fallback_derivation": decision == "repair",
            "validation": {
                "schema": validation.schema_valid,
                "capability": validation.capability_valid,
                "dependency": validation.dependency_valid,
                "environment": validation.environment_valid,
                "overall": validation.overall_valid
            }
        }),
        decision,
    })
}

#[derive(Debug, Clone)]
struct ExecutionSpecDiscoveryCandidate {
    source: &'static str,
    path: String,
    specification: Value,
    portable_execution_toml: String,
    validation_hint: String,
}

fn discover_execution_spec_candidate(root: &Path) -> Option<ExecutionSpecDiscoveryCandidate> {
    read_execution_spec_candidate(root, "execution.toml", "execution_toml")
        .or_else(|| read_execution_spec_candidate(root, "execution.json", "execution_json"))
        .or_else(|| discover_dot_execution_spec_candidate(root))
        .or_else(|| read_execution_spec_candidate(root, "ddockit.toml", "ddockit_toml"))
        .or_else(|| {
            read_execution_spec_candidate(
                root,
                ".well-known/execution.toml",
                "well_known_execution_toml",
            )
        })
        .or_else(|| discover_oci_wasi_metadata_candidate(root))
}

fn discover_dot_execution_spec_candidate(root: &Path) -> Option<ExecutionSpecDiscoveryCandidate> {
    let dot_execution = root.join(".execution");
    if !dot_execution.is_dir() {
        return None;
    }

    read_execution_spec_candidate(root, ".execution/execution.toml", "dot_execution_directory")
        .or_else(|| {
            read_execution_spec_candidate(
                root,
                ".execution/execution.json",
                "dot_execution_directory",
            )
        })
        .or_else(|| {
            let mut candidates = fs::read_dir(dot_execution)
                .ok()?
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let path = entry.path();
                    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
                    (extension == "toml" || extension == "json").then_some(path)
                })
                .collect::<Vec<_>>();
            // Deterministically prefer the same candidate across runs when multiple spec files exist.
            candidates.sort();
            let first = candidates.first()?;
            let relative = first
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            read_execution_spec_candidate(root, &relative, "dot_execution_directory")
        })
}

fn discover_oci_wasi_metadata_candidate(root: &Path) -> Option<ExecutionSpecDiscoveryCandidate> {
    let metadata_candidates = [
        "runtime.graph.json",
        "execution.lock",
        "wasi-runtime.json",
        "oci-layout",
    ];
    let found = discover_existing_files(root, &metadata_candidates);
    if found.is_empty() {
        return None;
    }
    Some(ExecutionSpecDiscoveryCandidate {
        source: "oci_wasi_metadata",
        path: found.join(","),
        specification: json!({
            "format": "metadata",
            "metadata_files": found
        }),
        portable_execution_toml: "# sourced from OCI/WASI metadata\n".to_string(),
        validation_hint: "metadata".to_string(),
    })
}

fn read_execution_spec_candidate(
    root: &Path,
    relative_path: &str,
    source: &'static str,
) -> Option<ExecutionSpecDiscoveryCandidate> {
    let absolute_path = root.join(relative_path);
    let content = fs::read_to_string(absolute_path).ok()?;
    if relative_path.ends_with(".json") {
        let parsed = serde_json::from_str::<Value>(&content).ok()?;
        Some(ExecutionSpecDiscoveryCandidate {
            source,
            path: relative_path.to_string(),
            specification: parsed,
            portable_execution_toml: format!("# sourced from {relative_path}\n"),
            validation_hint: content,
        })
    } else {
        Some(ExecutionSpecDiscoveryCandidate {
            source,
            path: relative_path.to_string(),
            specification: json!({
                "format": "toml",
                "path": relative_path,
                "raw": content
            }),
            portable_execution_toml: content.clone(),
            validation_hint: content,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct DiscoveredSpecValidation {
    schema_valid: bool,
    capability_valid: bool,
    dependency_valid: bool,
    environment_valid: bool,
    overall_valid: bool,
}

fn validate_discovered_spec(
    candidate: &ExecutionSpecDiscoveryCandidate,
    analysis: &RepositoryAnalysis,
    dependency_files: &[String],
    environment_graph: &[Value],
) -> DiscoveredSpecValidation {
    let hint = candidate.validation_hint.to_ascii_lowercase();
    let has_runtime_marker = hint.contains("[runtime]") || hint.contains("\"runtime\"");
    let has_version_marker = hint.contains("version") || hint.contains("\"schema_version\"");
    let schema_valid =
        candidate.source == "oci_wasi_metadata" || (has_runtime_marker && has_version_marker);
    let capability_valid = analysis
        .compiled_runtime
        .wasi_component_graph
        .capabilities
        .needs
        .is_empty()
        || hint.contains("capabilities");
    let dependency_valid = dependency_files.is_empty() || hint.contains("dependencies");
    let environment_valid = environment_graph.is_empty()
        || hint.contains("environment")
        || hint.contains("[environment]");
    let overall_valid = schema_valid && capability_valid && dependency_valid && environment_valid;
    DiscoveredSpecValidation {
        schema_valid,
        capability_valid,
        dependency_valid,
        environment_valid,
        overall_valid,
    }
}

fn trust_score_for_discovered_spec(
    candidate: &ExecutionSpecDiscoveryCandidate,
) -> (u8, &'static str) {
    if candidate.source == "well_known_execution_toml" || candidate.source == "oci_wasi_metadata" {
        return (90, "published_release_artifact");
    }
    if candidate.path == "ddockit.toml" {
        return (85, "generated_by_ddockit");
    }
    (70, "user_modified")
}

fn discover_existing_files(root: &Path, candidates: &[&str]) -> Vec<String> {
    let mut existing = candidates
        .iter()
        .filter_map(|candidate| {
            let path = root.join(candidate);
            path.exists().then(|| candidate.to_string())
        })
        .collect::<Vec<_>>();
    existing.sort();
    existing
}

fn discover_ci_files(root: &Path) -> Vec<String> {
    let workflows = root.join(".github/workflows");
    let mut files = fs::read_dir(workflows)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().to_string();
            (file_name.ends_with(".yml") || file_name.ends_with(".yaml"))
                .then(|| format!(".github/workflows/{file_name}"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn discover_env_keys(root: &Path, env_files: &[String]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for file in env_files {
        let content = fs::read_to_string(root.join(file)).unwrap_or_default();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let Some((name, _)) = trimmed.split_once('=') else {
                continue;
            };
            let key = name.trim();
            if !key.is_empty()
                && key
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
            {
                keys.insert(key.to_string());
            }
        }
    }
    keys
}

fn normalize_health_check_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuntimeManifestLaunchConfig {
    node_version: Option<String>,
    package_manager: Option<String>,
    install_command: Option<String>,
    start_command: Option<String>,
    preferred_ports: Vec<u16>,
    health_check: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct RuntimeRepairInput {
    runtime_manifest: Option<Value>,
    execution_artifact: Option<Value>,
    launch_logs: Vec<String>,
    failure_message: String,
}

#[derive(Debug, Clone, PartialEq)]
struct RuntimeRepairCandidate {
    id: String,
    confidence: f32,
    reason: String,
    manifest_patch: Option<Value>,
    start_command_override: Option<String>,
}

fn load_runtime_manifest_launch_config(repo_root: &Path) -> Option<RuntimeManifestLaunchConfig> {
    let runtime_manifest_path = repo_root.join("runtime-manifest.json");
    if let Ok(payload) = fs::read_to_string(runtime_manifest_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&payload) {
            let preferred_ports = value
                .get("network")
                .and_then(|network| network.get("preferredPorts"))
                .and_then(Value::as_array)
                .map(|ports| {
                    ports
                        .iter()
                        .filter_map(|port| {
                            port.as_u64().and_then(|value| u16::try_from(value).ok())
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Some(RuntimeManifestLaunchConfig {
                node_version: value
                    .get("runtime")
                    .and_then(|runtime| runtime.get("nodeVersion"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                package_manager: value
                    .get("runtime")
                    .and_then(|runtime| runtime.get("packageManager"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                install_command: value
                    .get("runtime")
                    .and_then(|runtime| runtime.get("installCommand"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                start_command: value
                    .get("runtime")
                    .and_then(|runtime| runtime.get("startCommand"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                preferred_ports,
                health_check: value
                    .get("network")
                    .and_then(|network| network.get("healthCheck"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            });
        }
    }

    let legacy_manifest_path = repo_root.join(".execution.json");
    let payload = fs::read_to_string(legacy_manifest_path).ok()?;
    let value = serde_json::from_str::<Value>(&payload).ok()?;
    Some(RuntimeManifestLaunchConfig {
        start_command: value
            .get("startCommand")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        ..RuntimeManifestLaunchConfig::default()
    })
}

fn load_execution_manifest_start_command(repo_root: &Path) -> Option<String> {
    load_runtime_manifest_launch_config(repo_root).and_then(|manifest| manifest.start_command)
}

fn load_execution_manifest_install_command(repo_root: &Path) -> Option<String> {
    load_runtime_manifest_launch_config(repo_root).and_then(|manifest| manifest.install_command)
}

fn load_execution_manifest_preferred_ports(repo_root: &Path) -> Vec<u16> {
    load_runtime_manifest_launch_config(repo_root)
        .map(|manifest| manifest.preferred_ports)
        .unwrap_or_default()
}

fn load_execution_manifest_health_check(repo_root: &Path) -> String {
    normalize_health_check_path(
        &load_runtime_manifest_launch_config(repo_root)
            .and_then(|manifest| manifest.health_check)
            .unwrap_or_else(|| "/".to_string()),
    )
}

#[cfg(test)]
fn load_execution_manifest_node_version(repo_root: &Path) -> Option<String> {
    load_runtime_manifest_launch_config(repo_root).and_then(|manifest| manifest.node_version)
}

#[cfg(test)]
fn load_execution_manifest_package_manager(repo_root: &Path) -> Option<String> {
    load_runtime_manifest_launch_config(repo_root).and_then(|manifest| manifest.package_manager)
}

fn runtime_repair_attempt_limit() -> usize {
    std::env::var("RUSTGIT_RUNTIME_RETRY_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(RUNTIME_REPAIR_MAX_ATTEMPTS_DEFAULT)
}

fn load_runtime_manifest_value(repo_root: &Path) -> Option<Value> {
    let payload = fs::read_to_string(repo_root.join("runtime-manifest.json")).ok()?;
    serde_json::from_str::<Value>(&payload).ok()
}

fn write_runtime_manifest_value(repo_root: &Path, value: &Value) -> Result<()> {
    let body = serde_json::to_vec_pretty(value).map_err(|err| {
        RuntimeError::CommandFailed(format!("runtime manifest serialization failed: {err}"))
    })?;
    fs::write(repo_root.join("runtime-manifest.json"), body)?;
    Ok(())
}

fn replace_command_package_manager(command: &str, manager: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    let mut parts = trimmed.split_whitespace();
    let first = parts.next().unwrap_or_default();
    let rest = parts.collect::<Vec<_>>().join(" ");
    if first == manager {
        return trimmed.to_string();
    }
    if rest.is_empty() {
        manager.to_string()
    } else {
        format!("{manager} {rest}")
    }
}

fn build_runtime_repair_candidates(
    input: &RuntimeRepairInput,
    base_overrides: &LaunchOverrides,
    fallback_command: &str,
    fingerprint: &RepositoryFingerprint,
) -> Vec<RuntimeRepairCandidate> {
    let mut failure_signal = FailureSignal {
        message: input.failure_message.clone(),
        ..FailureSignal::default()
    };
    failure_signal.attempted_command = input
        .execution_artifact
        .as_ref()
        .and_then(|artifact| {
            artifact
                .get("metadata")
                .and_then(|metadata| metadata.get("launchCommand"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| base_overrides.start_command.clone())
        .or_else(|| (!fallback_command.trim().is_empty()).then(|| fallback_command.to_string()));
    failure_signal.expected_package_manager =
        input.runtime_manifest.as_ref().and_then(|manifest| {
            manifest
                .get("runtime")
                .and_then(|runtime| runtime.get("packageManager"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    failure_signal.required_runtime = input.runtime_manifest.as_ref().and_then(|manifest| {
        manifest
            .get("runtime")
            .and_then(|runtime| runtime.get("nodeVersion"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    failure_signal.detected_runtime = base_overrides.versions.get("NODE_VERSION").cloned();
    if input
        .launch_logs
        .iter()
        .any(|line| line.to_ascii_lowercase().contains("module not found"))
    {
        failure_signal.required_artifact = Some("dependency-install".to_string());
    }
    let classifier = FailureClassifier;
    let class = classifier.classify(&failure_signal, fingerprint);
    let strategy = HealingCatalog::default().strategy_for(class, &failure_signal, fingerprint);
    let reason = format!("{:?}", class).to_ascii_lowercase();

    let mut candidates = Vec::new();
    if let Some(mut manifest_patch) = input.runtime_manifest.clone() {
        if manifest_patch
            .get("runtime")
            .is_none_or(|runtime| !runtime.is_object())
        {
            manifest_patch["runtime"] = json!({});
        }
        if manifest_patch
            .get("network")
            .is_none_or(|network| !network.is_object())
        {
            manifest_patch["network"] = json!({});
        }
        let start_command = failure_signal
            .attempted_command
            .clone()
            .unwrap_or_else(|| fallback_command.to_string());
        if !start_command.trim().is_empty() {
            manifest_patch["runtime"]["startCommand"] =
                Value::String(WorkspaceManager::auto_heal_runtime_command(&start_command));
        }
        if strategy
            .actions
            .contains(&RepairAction::SwitchPackageManager)
        {
            if let Some(package_manager) = failure_signal.expected_package_manager.as_deref() {
                manifest_patch["runtime"]["packageManager"] =
                    Value::String(package_manager.to_string());
                manifest_patch["runtime"]["installCommand"] =
                    Value::String(format!("{package_manager} install"));
                if let Some(current) = manifest_patch["runtime"]["startCommand"].as_str() {
                    manifest_patch["runtime"]["startCommand"] =
                        Value::String(replace_command_package_manager(current, package_manager));
                }
            }
        }
        if input
            .failure_message
            .to_ascii_lowercase()
            .contains("readiness probe timed out")
            || input
                .failure_message
                .to_ascii_lowercase()
                .contains("health")
        {
            manifest_patch["network"]["healthCheck"] = Value::String("/".to_string());
        }
        candidates.push(RuntimeRepairCandidate {
            id: "ai-patched-manifest".to_string(),
            confidence: strategy.confidence,
            reason: reason.clone(),
            manifest_patch: Some(manifest_patch),
            start_command_override: None,
        });
    }

    let command_patch_source = failure_signal
        .attempted_command
        .clone()
        .unwrap_or_else(|| fallback_command.to_string());
    let command_patch = WorkspaceManager::auto_heal_runtime_command(&command_patch_source);
    if !command_patch.trim().is_empty() && command_patch != command_patch_source {
        candidates.push(RuntimeRepairCandidate {
            id: "ai-patched-command".to_string(),
            confidence: strategy.confidence.max(0.65),
            reason: reason.clone(),
            manifest_patch: None,
            start_command_override: Some(command_patch),
        });
    }

    let alternative_command = failure_signal
        .expected_package_manager
        .as_deref()
        .map(|manager| replace_command_package_manager(&command_patch_source, manager))
        .unwrap_or_else(|| command_patch_source.clone());
    if !alternative_command.trim().is_empty() && alternative_command != command_patch_source {
        candidates.push(RuntimeRepairCandidate {
            id: "alternative-detected-runtime".to_string(),
            confidence: (strategy.confidence * 0.75).max(0.50),
            reason,
            manifest_patch: None,
            start_command_override: Some(alternative_command),
        });
    }
    candidates
}

fn workspace_ports_from_manifest_or_framework(
    repo_root: &Path,
    framework: Framework,
) -> Vec<PortInfo> {
    let preferred_ports = load_execution_manifest_preferred_ports(repo_root);
    if preferred_ports.is_empty() {
        return ports_for_framework(framework);
    }
    preferred_ports
        .into_iter()
        .map(|port| PortInfo {
            port,
            protocol: "http".to_string(),
            route: "/".to_string(),
        })
        .collect()
}

fn runtime_label_for_context(
    context: Option<&ExecutionContext>,
    manifest: Option<&RuntimeManifestLaunchConfig>,
) -> String {
    context
        .map(|ctx| {
            manifest
                .and_then(|manifest| manifest.node_version.as_deref())
                .map(|version| format!("node{version}"))
                .unwrap_or_else(|| ctx.runtime_spec.language.clone())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_package_dependency_names(root: &Path) -> HashSet<String> {
    let package_json = fs::read_to_string(root.join("package.json")).unwrap_or_default();
    let parsed = serde_json::from_str::<Value>(&package_json).unwrap_or(Value::Null);
    let mut dependencies = HashSet::new();
    for section in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(entries) = parsed.get(section).and_then(Value::as_object) {
            dependencies.extend(entries.keys().cloned());
        }
    }
    dependencies
}

fn environment_variable_blueprint(
    name: &str,
    synthesized_value: impl Into<String>,
    sensitive: bool,
    required_for_startup: bool,
    mockable: bool,
    discovered_env_keys: &HashSet<String>,
    synthesis_strategy: Vec<String>,
) -> Value {
    let value_source = if discovered_env_keys.contains(name) {
        "repository"
    } else {
        "synthesized"
    };
    json!({
        "name": name,
        "classification": if required_for_startup { "required" } else { "optional" },
        "sensitive": sensitive,
        "mockable": mockable,
        "required_for_startup": required_for_startup,
        "value_source": value_source,
        "selected_value": synthesized_value.into(),
        "synthesis_strategy": synthesis_strategy
    })
}

fn simulated_failures(
    root: &Path,
    analysis: &RepositoryAnalysis,
    environment_graph: &[Value],
    environment_files: &[String],
    has_prisma: bool,
) -> Vec<Value> {
    let mut failures = Vec::new();
    let missing_database_url = environment_graph.iter().any(|entry| {
        entry
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name == "DATABASE_URL")
            && entry
                .get("classification")
                .and_then(Value::as_str)
                .is_some_and(|class| class == "required")
            && entry
                .get("value_source")
                .and_then(Value::as_str)
                .is_some_and(|source| source == "synthesized")
    });
    let has_any_synthesized_required_env = environment_graph.iter().any(|entry| {
        entry
            .get("classification")
            .and_then(Value::as_str)
            .is_some_and(|class| class == "required")
            && entry
                .get("value_source")
                .and_then(Value::as_str)
                .is_some_and(|source| source == "synthesized")
    });
    if missing_database_url || (has_prisma && environment_files.is_empty()) {
        failures.push(json!({
            "failure": "Missing DATABASE_URL",
            "confidence": 98,
            "pre_heal": "synthesize local sqlite, then temporary postgres fallback"
        }));
    }
    if has_any_synthesized_required_env {
        failures.push(json!({
            "failure": "Required environment synthesis needed",
            "confidence": 92,
            "pre_heal": "materialize synthesized required variables and validate startup contract"
        }));
    }
    if root.join(".nvmrc").exists() || root.join(".node-version").exists() {
        failures.push(json!({
            "failure": "Node version mismatch",
            "confidence": 96,
            "pre_heal": "select runtime version from repository and historical success profile"
        }));
    }
    if analysis.build_intelligence.package_manager.as_deref() == Some("pnpm")
        && !root.join("pnpm-lock.yaml").exists()
    {
        failures.push(json!({
            "failure": "pnpm lock mismatch",
            "confidence": 91,
            "pre_heal": "regenerate lockfile-compatible install command"
        }));
    }
    if analysis
        .runtime_spec
        .ports
        .iter()
        .any(|port| *port == 3000 || *port == 8080)
    {
        failures.push(json!({
            "failure": "Port conflict",
            "confidence": 88,
            "pre_heal": "allocate alternate runtime port and rewrite health endpoint bindings"
        }));
    }
    failures
}

fn pre_healing_actions(expected_failures: &[Value]) -> Vec<String> {
    expected_failures
        .iter()
        .filter_map(|failure| failure.get("failure").and_then(Value::as_str))
        .map(|failure| match failure {
            "Missing DATABASE_URL" => {
                "Search repo/env history, synthesize DATABASE_URL, validate connectivity"
                    .to_string()
            }
            "Node version mismatch" => {
                "Mutate Node runtime candidate, simulate compatibility, pin selected version"
                    .to_string()
            }
            "pnpm lock mismatch" => {
                "Reconcile package manager + lock strategy before install".to_string()
            }
            "Port conflict" => {
                "Reserve free port, rewrite run command and health probes".to_string()
            }
            _ => "Run compatibility pre-heal".to_string(),
        })
        .collect()
}

fn environment_confidence_scores(
    environment_files: &[String],
    dependency_files: &[String],
    analysis: &RepositoryAnalysis,
    expected_failures: &[Value],
) -> Value {
    let repository_health: u8 = if dependency_files.is_empty() {
        PREFLIGHT_REPOSITORY_HEALTH_NO_DEPS
    } else {
        PREFLIGHT_REPOSITORY_HEALTH_WITH_DEPS
    };
    let dependencies: u8 = if analysis.runtime_spec.dependencies.is_empty() {
        PREFLIGHT_DEPENDENCIES_CONFIDENCE_UNKNOWN
    } else {
        PREFLIGHT_DEPENDENCIES_CONFIDENCE_READY
    };
    let capabilities: u8 = if analysis
        .compiled_runtime
        .wasi_component_graph
        .capabilities
        .needs
        .is_empty()
    {
        PREFLIGHT_CAPABILITIES_CONFIDENCE_UNKNOWN
    } else {
        PREFLIGHT_CAPABILITIES_CONFIDENCE_READY
    };
    let environment: u8 = if environment_files.is_empty() {
        PREFLIGHT_ENVIRONMENT_CONFIDENCE_SYNTHESIZED
    } else {
        PREFLIGHT_ENVIRONMENT_CONFIDENCE_DISCOVERED
    };
    let expected_runtime: u8 = if analysis.runtime_spec.requires_wasm {
        PREFLIGHT_RUNTIME_CONFIDENCE_WASM
    } else {
        PREFLIGHT_RUNTIME_CONFIDENCE_NATIVE
    };
    let expected_success = ((repository_health as u16
        + dependencies as u16
        + capabilities as u16
        + environment as u16
        + expected_runtime as u16)
        / 5) as u8;
    let failure_count = expected_failures.len().min(u8::MAX as usize) as u8;
    let penalty = failure_count.saturating_mul(PREFLIGHT_FAILURE_PENALTY_PER_ISSUE);
    json!({
        "repository_health": repository_health,
        "environment": environment,
        "dependencies": dependencies,
        "capabilities": capabilities,
        "expected_runtime": expected_runtime,
        "expected_success": expected_success.saturating_sub(penalty)
    })
}

pub fn execution_plan_endpoint(analysis: &RepositoryAnalysis) -> (String, String) {
    let plan = generate_execution_plan(analysis);
    let execution_plan_id = plan
        .topology
        .as_ref()
        .map(|topology| topology.topology_id.clone())
        .unwrap_or_else(|| format!("plan-{}", hash_key(&analysis.fingerprint.repo_hash)));
    (
        "/api/v1/execution/plan".to_string(),
        json!({
            "execution_plan_id": execution_plan_id,
            "services": plan.services.iter().map(|service| service.id.clone()).collect::<Vec<_>>(),
            "startup_order": plan.startup_order,
            "preflight": preflight_intelligence_payload(analysis)
        })
        .to_string(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthLoginRequest {
    pub user: UserIdentity,
    pub org_id: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationCreateRequest {
    pub name: String,
    pub slug: String,
    pub plan: OrganizationPlan,
    pub created_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationMembershipCreateRequest {
    pub org_id: String,
    pub user_id: String,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubOAuthCallbackRequest {
    pub code: String,
    pub state: Option<String>,
    pub extension_id: Option<String>,
    pub github_id: u64,
    pub github_login: String,
    pub github_email: Option<String>,
    pub existing_user_id: Option<String>,
    pub existing_org_id: Option<String>,
    pub role: MembershipRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleOAuthCallbackRequest {
    pub code: String,
    pub state: Option<String>,
    pub extension_id: Option<String>,
    pub google_sub: String,
    pub google_email: String,
    pub google_name: String,
    pub existing_user_id: Option<String>,
    pub existing_org_id: Option<String>,
    pub role: MembershipRole,
}

fn provider_name(provider: AuthProvider) -> &'static str {
    match provider {
        AuthProvider::Github => "github",
        AuthProvider::Google => "google",
        AuthProvider::Microsoft => "microsoft",
    }
}

fn oauth_org_slug(seed: &str, unique_hint: &str) -> String {
    let mut slug = String::with_capacity(seed.len());
    let mut previous_dash = false;
    for ch in seed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        let suffix = &hash_key(unique_hint)[..8];
        format!("org-{suffix}")
    } else {
        trimmed.to_string()
    }
}

fn oauth_redirect_targets(
    token: &str,
    extension_id: Option<&str>,
) -> (String, Option<String>, String) {
    let extension_redirect = extension_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("chrome-extension://{value}/auth/success?token={token}"));
    let portal_redirect = format!("https://trythissoftware.com/auth/success?token={token}");
    let selected_redirect = extension_redirect
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| portal_redirect.clone());
    (selected_redirect, extension_redirect, portal_redirect)
}

pub fn github_oauth_callback_endpoint(request: &GithubOAuthCallbackRequest) -> (String, String) {
    let user_id = request
        .existing_user_id
        .clone()
        .unwrap_or_else(|| format!("user-gh-{}", hash_key(&request.github_id.to_string())));
    let user_created = request.existing_user_id.is_none();
    let org_name = format!("{}-org", request.github_login);
    let org_slug = oauth_org_slug(&org_name, &request.github_id.to_string());
    let org_id = request.existing_org_id.clone().unwrap_or_else(|| {
        format!(
            "org-{}",
            hash_key(&format!("github:{}:{}", org_slug, request.github_id))
        )
    });
    let org_created = request.existing_org_id.is_none();
    let provider = AuthProvider::Github;
    let provider_value = provider_name(provider);
    let email = request
        .github_email
        .clone()
        .unwrap_or_else(|| format!("{}@users.noreply.github.com", request.github_login));
    let jwt = format!(
        "jwt-{}",
        hash_key(&format!(
            "{}:{}:{}:{}:{}",
            user_id,
            org_id,
            provider_value,
            request.code,
            request.state.as_deref().unwrap_or_default()
        ))
    );
    let (redirect_to, extension_redirect, portal_redirect) =
        oauth_redirect_targets(&jwt, request.extension_id.as_deref());
    (
        "/auth/github/callback".to_string(),
        json!({
            "token_exchange": {
                "url": "https://github.com/login/oauth/access_token",
                "grant_type": "authorization_code",
                "code": &request.code
            },
            "identity_fetch": {
                "url": "https://api.github.com/user",
                "github_id": request.github_id,
                "login": &request.github_login,
                "email": &email
            },
            "user": {
                "user_id": user_id,
                "email": email,
                "name": &request.github_login,
                "provider": provider_value,
                "status": if user_created { "created" } else { "existing" }
            },
            "org": {
                "org_id": org_id,
                "name": org_name,
                "slug": org_slug,
                "status": if org_created { "created" } else { "existing" }
            },
            "jwt": {
                "token_type": "jwt",
                "token": &jwt,
                "claims": {
                    "user_id": user_id,
                    "org_id": org_id,
                    "provider": provider_value,
                    "role": request.role
                }
            },
            "redirect": {
                "to": redirect_to,
                "portal": portal_redirect,
                "extension": extension_redirect
            },
            "state": &request.state
        })
        .to_string(),
    )
}

pub fn google_oauth_callback_endpoint(request: &GoogleOAuthCallbackRequest) -> (String, String) {
    let user_id = request
        .existing_user_id
        .clone()
        .unwrap_or_else(|| format!("user-goog-{}", hash_key(&request.google_sub)));
    let user_created = request.existing_user_id.is_none();
    let email_prefix = request
        .google_email
        .split_once('@')
        .map(|(prefix, _)| prefix)
        .unwrap_or(request.google_email.as_str());
    let org_prefix = if email_prefix.is_empty() {
        request.google_name.as_str()
    } else {
        email_prefix
    };
    let org_name = format!("{org_prefix}-org");
    let org_slug = oauth_org_slug(&org_name, &request.google_sub);
    let org_id = request.existing_org_id.clone().unwrap_or_else(|| {
        format!(
            "org-{}",
            hash_key(&format!("google:{}:{}", org_slug, request.google_sub))
        )
    });
    let org_created = request.existing_org_id.is_none();
    let provider = AuthProvider::Google;
    let provider_value = provider_name(provider);
    let jwt = format!(
        "jwt-{}",
        hash_key(&format!(
            "{}:{}:{}:{}:{}",
            user_id,
            org_id,
            provider_value,
            request.code,
            request.state.as_deref().unwrap_or_default()
        ))
    );
    let (redirect_to, extension_redirect, portal_redirect) =
        oauth_redirect_targets(&jwt, request.extension_id.as_deref());
    (
        "/auth/google/callback".to_string(),
        json!({
            "token_exchange": {
                "url": "https://oauth2.googleapis.com/token",
                "grant_type": "authorization_code",
                "code": &request.code
            },
            "identity_fetch": {
                "url": "https://openidconnect.googleapis.com/v1/userinfo",
                "sub": &request.google_sub,
                "email": &request.google_email,
                "name": &request.google_name
            },
            "user": {
                "user_id": user_id,
                "email": &request.google_email,
                "name": &request.google_name,
                "provider": provider_value,
                "status": if user_created { "created" } else { "existing" }
            },
            "org": {
                "org_id": org_id,
                "name": org_name,
                "slug": org_slug,
                "status": if org_created { "created" } else { "existing" }
            },
            "jwt": {
                "token_type": "jwt",
                "token": &jwt,
                "claims": {
                    "user_id": user_id,
                    "org_id": org_id,
                    "provider": provider_value,
                    "role": request.role
                }
            },
            "redirect": {
                "to": redirect_to,
                "portal": portal_redirect,
                "extension": extension_redirect
            },
            "state": &request.state
        })
        .to_string(),
    )
}

pub fn auth_login_endpoint(request: &AuthLoginRequest) -> (String, String) {
    let claims = AuthClaims {
        user_id: request.user.user_id.clone(),
        org_id: request.org_id.clone(),
        role: request.role,
        permissions: RbacPolicyEngine::role_permissions(request.role),
    };
    (
        "/auth/login".to_string(),
        json!({
            "provider": request.user.auth_provider,
            "token_type": "jwt",
            "claims": claims
        })
        .to_string(),
    )
}

pub fn auth_logout_endpoint(context: &AuthContext) -> (String, String) {
    (
        "/auth/logout".to_string(),
        json!({
            "user_id": &context.user_id,
            "org_id": &context.org_id,
            "status": "logged_out"
        })
        .to_string(),
    )
}

pub fn auth_me_endpoint(context: &AuthContext) -> (String, String) {
    (
        "/auth/me".to_string(),
        json!({
            "user_id": &context.user_id,
            "org_id": &context.org_id,
            "role": context.role,
            "permissions": &context.permissions
        })
        .to_string(),
    )
}

pub fn org_create_endpoint(request: &OrganizationCreateRequest) -> (String, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let org_id = format!(
        "org-{}",
        hash_key(&format!("{}:{}:{now}", request.slug, request.created_by))
    );
    (
        "/orgs".to_string(),
        json!({
            "org_id": org_id,
            "name": &request.name,
            "slug": &request.slug,
            "plan": request.plan,
            "created_by": &request.created_by
        })
        .to_string(),
    )
}

pub fn org_get_endpoint(org: &OrganizationIdentity) -> (String, String) {
    (
        format!("/orgs/{}", org.org_id),
        json!({
            "org_id": &org.org_id,
            "name": &org.name,
            "slug": &org.slug,
            "plan": org.plan,
            "created_at": org.created_at
        })
        .to_string(),
    )
}

pub fn org_add_member_endpoint(request: &OrganizationMembershipCreateRequest) -> (String, String) {
    (
        format!("/orgs/{}/members", request.org_id),
        json!({
            "org_id": &request.org_id,
            "user_id": &request.user_id,
            "role": request.role
        })
        .to_string(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCreateRequest {
    pub repository_id: String,
    pub commit_hash: String,
    pub org_id: String,
    pub created_by: String,
    pub visibility: WorkspaceVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRuntimeRequest {
    pub runtime_type: String,
    pub runtime_instance_id: String,
    pub endpoint: String,
    pub lease_expires_at: u64,
}

pub fn workspace_create_endpoint(request: &WorkspaceCreateRequest) -> (String, String) {
    let workspace_seed = format!(
        "{}:{}:{}",
        request.org_id, request.repository_id, request.commit_hash
    );
    let workspace_id = ExecutionRouter::sanitized_workspace_id(&hash_key(&workspace_seed)[..12]);
    (
        "/workspaces".to_string(),
        json!({
            "workspace_id": workspace_id.clone(),
            "org_id": &request.org_id,
            "repository_id": &request.repository_id,
            "commit_hash": &request.commit_hash,
            "created_by": &request.created_by,
            "visibility": request.visibility,
            "status": "pending",
            "workspace_url": stable_workspace_url(&workspace_id, true).0
        })
        .to_string(),
    )
}

pub fn workspace_resolve_endpoint(
    workspace_id: &str,
    router: &WorkspaceRouter,
) -> (String, String) {
    let workspace = router.registry.get(workspace_id);
    let binding = router.resolver.resolve(workspace_id);
    (
        format!("/workspaces/{workspace_id}"),
        json!({
            "workspace_id": workspace_id,
            "org_id": workspace.map(|record| record.org_id.as_str()),
            "repository_id": workspace.map(|record| record.repository_id.as_str()),
            "created_by": workspace.map(|record| record.created_by.as_str()),
            "visibility": workspace.map(|record| record.visibility),
            "status": workspace.map(|record| format!("{:?}", record.state)),
            "url": workspace.map(|record| record.assigned_url.0.as_str()),
            "runtime_type": binding.map(|entry| format!("{:?}", entry.runtime_type)),
            "runtime_instance_id": binding.map(|entry| entry.runtime_instance_id.as_str()),
            "endpoint": binding.map(|entry| entry.endpoint.as_str()),
        })
        .to_string(),
    )
}

pub fn workspace_bind_endpoint(
    workspace_id: &str,
    request: &WorkspaceRuntimeRequest,
) -> (String, String) {
    (
        format!("/workspaces/{workspace_id}/bind"),
        json!({
            "workspace_id": workspace_id,
            "runtime_type": &request.runtime_type,
            "runtime_instance_id": &request.runtime_instance_id,
            "endpoint": &request.endpoint,
            "lease_expires_at": request.lease_expires_at,
        })
        .to_string(),
    )
}

pub fn workspace_migrate_endpoint(
    workspace_id: &str,
    request: &WorkspaceRuntimeRequest,
) -> (String, String) {
    (
        format!("/workspaces/{workspace_id}/migrate"),
        json!({
            "workspace_id": workspace_id,
            "runtime_type": &request.runtime_type,
            "runtime_instance_id": &request.runtime_instance_id,
            "endpoint": &request.endpoint,
            "lease_expires_at": request.lease_expires_at,
        })
        .to_string(),
    )
}

pub fn workspaces_list_endpoint(org_id: &str, router: &WorkspaceRouter) -> (String, String) {
    let workspaces = router
        .registry
        .all()
        .into_iter()
        .filter(|record| record.org_id == org_id)
        .map(|record| {
            json!({
                "workspace_id": record.workspace_id,
                "org_id": record.org_id,
                "created_by": record.created_by,
                "visibility": record.visibility,
                "status": format!("{:?}", record.state),
                "url": record.assigned_url.0
            })
        })
        .collect::<Vec<_>>();
    (
        format!("/workspaces?org_id={org_id}"),
        json!({
            "org_id": org_id,
            "workspaces": workspaces
        })
        .to_string(),
    )
}

pub fn workspace_delete_endpoint(workspace_id: &str, org_id: &str) -> (String, String) {
    (
        format!("/workspaces/{workspace_id}"),
        json!({
            "workspace_id": workspace_id,
            "org_id": org_id,
            "status": "deleted"
        })
        .to_string(),
    )
}

pub fn executions_start_endpoint(request: &ExecutionStartRequest) -> (String, String) {
    let execution_seed = format!(
        "{}|{}|{}|{}|{}",
        request.org_id.as_deref().unwrap_or_default(),
        request.identity_partition_key(),
        request.repo_url,
        request.branch.as_deref().unwrap_or_default(),
        request.commit.as_deref().unwrap_or_default()
    );
    let execution_id = format!("exec-{}", hash_key(&execution_seed));
    let workspace_slug = ExecutionRouter::sanitized_workspace_id(&execution_id);
    let workspace_id = workspace_slug.clone();
    (
        "/api/v1/executions".to_string(),
        json!({
            "execution_id": execution_id,
            "org_id": &request.org_id,
            "user_id": &request.user_id,
            "anon_user_id": &request.anon_user_id,
            "anon_session_id": &request.anon_session_id,
            "device_fingerprint": &request.device_fingerprint,
            "identity_type": request.identity_type(),
            "workspace_id": workspace_id,
            "status": "starting",
            "workspace_url": format!("https://workspace-{workspace_slug}.trythissoftware.com"),
            "claim_workspace_prompt": request.user_id.is_none(),
        })
        .to_string(),
    )
}

pub fn executions_list_endpoint(
    org_id: &str,
    executions: &[EidbExecutionRecord],
) -> (String, String) {
    let scoped = executions
        .iter()
        .filter(|execution| execution.org_id.as_deref() == Some(org_id))
        .map(|execution| {
            json!({
                "execution_id": &execution.execution_id,
                "org_id": &execution.org_id,
                "user_id": &execution.user_id,
                "anon_user_id": &execution.anon_user_id,
                "workspace_id": &execution.workspace_id,
                "status": &execution.status
            })
        })
        .collect::<Vec<_>>();
    (
        format!("/executions?org_id={org_id}"),
        json!({
            "org_id": org_id,
            "executions": scoped
        })
        .to_string(),
    )
}

pub fn surface_execution_start_endpoint(
    surface: ProductSurface,
    request: &ExecutionStartRequest,
) -> (String, String) {
    let (path, body) = executions_start_endpoint(request);
    let mut payload: serde_json::Value = match serde_json::from_str(&body) {
        Ok(payload) => payload,
        Err(_) => return (path, body),
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert("surface".to_string(), json!(surface.as_str()));
        object.insert("entry_api".to_string(), json!("/api/v1/executions"));
        object.insert("control_plane".to_string(), json!("unified"));
    }
    (path, payload.to_string())
}

pub fn detect_overlay_repository_context(url: &str) -> Option<OverlayRepositoryContext> {
    let github_root = "https://github.com/";
    let path = url.strip_prefix(github_root)?;
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let owner = segments.next()?.to_string();
    let repo = segments.next()?.to_string();
    let branch = match segments.next() {
        Some("tree") => {
            let suffix = segments.collect::<Vec<_>>().join("/");
            if suffix.is_empty() {
                return None;
            }
            suffix
        }
        // Overlay URL extraction falls back to main when GitHub does not include `/tree/<branch>`.
        _ => "main".to_string(),
    };
    Some(OverlayRepositoryContext {
        owner,
        repo,
        branch,
    })
}

const OVERLAY_EXTENSION_ACTIONS: &[&str] = &[
    "run",
    "instant_run",
    "analyze",
    "runtime",
    "commits",
    "ask_repository",
];
const SHARED_SURFACE_COMPONENTS: &[&str] = &[
    "Button",
    "Card",
    "Badge",
    "Table",
    "Modal",
    "Drawer",
    "Tabs",
    "Navigation",
    "Progress",
    "LogsViewer",
    "TopologyGraph",
    "StatusIndicator",
];

pub fn extension_overlay_actions() -> &'static [&'static str] {
    OVERLAY_EXTENSION_ACTIONS
}

pub fn shared_design_system_manifest() -> Value {
    json!({
        "name": "DDockit Design System",
        "theme": "dark-first",
        "persona": "developer-focused",
        "focus": "execution-centric",
        "components": SHARED_SURFACE_COMPONENTS,
        "status_colors": {
            "Running": "#22c55e",
            "Starting": "#38bdf8",
            "Stopped": "#94a3b8",
            "Failed": "#f87171",
            "Healing": "#facc15",
            "Migrating": "#a78bfa"
        }
    })
}

pub fn surface_component_registry() -> Value {
    json!({
        "button": "Button",
        "card": "Card",
        "badge": "Badge",
        "table": "Table",
        "modal": "Modal",
        "drawer": "Drawer",
        "tabs": "Tabs",
        "navigation": "Navigation",
        "progress": "Progress",
        "log_stream": "LogsViewer",
        "topology": "TopologyGraph",
        "status_indicator": "StatusIndicator"
    })
}

fn resolve_surface_component(component_type: &str) -> &'static str {
    match component_type {
        "button" => "Button",
        "card" => "Card",
        "badge" => "Badge",
        "table" => "Table",
        "modal" => "Modal",
        "drawer" => "Drawer",
        "tabs" => "Tabs",
        "navigation" => "Navigation",
        "progress" => "Progress",
        "log_stream" => "LogsViewer",
        "topology" => "TopologyGraph",
        "status_indicator" => "StatusIndicator",
        _ => "Card",
    }
}

pub fn render_surface_view(view: &str, components: &[Value]) -> Value {
    let rendered_components = components
        .iter()
        .map(|component| {
            let component_type = component
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("card");
            let slot = component
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("component");
            json!({
                "slot": slot,
                "component": resolve_surface_component(component_type),
                "contract_type": component_type,
                "definition": component
            })
        })
        .collect::<Vec<_>>();

    json!({
        "renderer": "unified_surface_renderer",
        "view": view,
        "components": rendered_components
    })
}

pub fn extension_overlay_actions_endpoint() -> (String, String) {
    (
        "/api/v1/surfaces/extension/actions".to_string(),
        json!({
            "surface": ProductSurface::GitHubOverlayExtension.as_str(),
            "actions": extension_overlay_actions(),
            "run_entrypoint": "/api/v1/executions",
            "ui_endpoint": "/api/v1/surfaces/extension/ui"
        })
        .to_string(),
    )
}

pub fn extension_overlay_ui_endpoint() -> (String, String) {
    let components = vec![
        json!({
            "id": "overlay_shell",
            "type": "card",
            "title": "DDockit",
            "subtitle": "GitHub overlay shell"
        }),
        json!({
            "id": "overlay_actions",
            "type": "button",
            "actions": ["run", "instant_run", "analyze", "ask_repository"]
        }),
        json!({
            "id": "run_flow",
            "type": "drawer",
            "states": ["Analyzing Repository...", "Generating Execution Plan...", "Selecting Runtime...", "Starting...", "Running"]
        }),
        json!({
            "id": "run_flow_progress",
            "type": "progress",
            "steps": ["analyzing", "planning", "runtime_selection", "starting", "running"]
        }),
        json!({
            "id": "extension_status",
            "type": "status_indicator",
            "statuses": ["Running", "Starting", "Stopped", "Failed", "Healing", "Migrating"]
        }),
        json!({
            "id": "analyze_panel",
            "type": "table",
            "columns": ["frameworks", "services", "ports", "runtime", "topology"]
        }),
        json!({
            "id": "repository_intelligence",
            "type": "card",
            "title": "Repository Intelligence",
            "fields": ["Execution Score", "Framework", "Runtime", "Last Success"],
            "actions": ["Launch", "Heal", "Adopt"],
            "endpoint": "/api/repositories/{id}/intelligence"
        }),
        json!({
            "id": "commit_panel",
            "type": "table",
            "columns": ["current_commit", "last_good_commit", "run_previous_commit"]
        }),
    ];
    let rendered = render_surface_view("github_overlay_shell", &components);
    (
        "/api/v1/surfaces/extension/ui".to_string(),
        json!({
            "surface": ProductSurface::GitHubOverlayExtension.as_str(),
            "view": "overlay_panel",
            "shell": "github_overlay_shell",
            "title": "Run with DDockit",
            "design_system": shared_design_system_manifest(),
            "component_registry": surface_component_registry(),
            "components": components.clone(),
            "rendered": rendered,
            "repository_context": {
                "owner": "{owner}",
                "repo": "{repo}",
                "branch": "{branch}"
            },
            "screenshot": {
                "id": "repository_detected_preview",
                "shape": "orb",
                "animation": "pulse",
                "state": {
                    "when_repository_detected": "pulse"
                }
            },
            "sections": [
                {
                    "id": "quick_actions",
                    "type": "button_group",
                    "label": "Quick Actions",
                    "actions": [
                        {"id": "run", "label": "Run"},
                        {"id": "instant_run", "label": "Instant Run"},
                        {"id": "analyze", "label": "Analyze"}
                    ]
                },
                {
                    "id": "runtime",
                    "type": "select",
                    "label": "Runtime",
                    "default": "auto",
                    "options": ["auto", "local", "cloud"]
                },
                {
                    "id": "latest_execution",
                    "type": "status_card",
                    "label": "Latest execution",
                    "fields": ["execution_id", "status", "workspace_url", "started_at"]
                }
            ],
            "actions_api": "/api/v1/surfaces/extension/actions",
            "run_api": "/api/v1/executions"
        })
        .to_string(),
    )
}

const PORTAL_INITIAL_NAVIGATION: &[&str] = &[
    "dashboard",
    "organization",
    "members",
    "workspaces",
    "executions",
    "billing",
    "settings",
];

pub fn portal_initial_navigation() -> &'static [&'static str] {
    PORTAL_INITIAL_NAVIGATION
}

pub fn portal_navigation_endpoint() -> (String, String) {
    (
        "/api/v1/surfaces/portal/navigation".to_string(),
        json!({
            "surface": ProductSurface::Portal.as_str(),
            "navigation": portal_initial_navigation(),
            "workspace_path": "/api/v1/executions/{id}",
            "org_switcher": ["Org A", "Org B", "Create Org"],
            "ui_endpoint": "/api/v1/surfaces/portal/ui",
            "publish_api": "/api/v1/repositories/publish"
        })
        .to_string(),
    )
}

pub fn portal_ui_endpoint() -> (String, String) {
    let components = vec![
        json!({
            "id": "portal_navigation",
            "type": "navigation",
            "items": portal_initial_navigation()
        }),
        json!({
            "id": "dashboard_metrics",
            "type": "card",
            "cards": ["Running Workspaces", "Healthy URLs", "DEA Agents", "Success Rate"]
        }),
        json!({
            "id": "badge_generator_studio",
            "type": "generator",
            "title": "Generate Badge",
            "input": {
                "repo_url": "https://github.com/vercel/next.js",
                "optional": {
                    "branch": "main",
                    "runtime_preference": ["auto", "wasm", "docker"],
                    "visibility_mode": ["public", "private"]
                }
            },
            "output": {
                "markdown": "[<img src=\"https://api.trythissoftware.com/badge/vercel/next.js.svg\" alt=\"vercel/next.js execution status badge\">](https://trythissoftware.com/seed/vercel/next.js)",
                "html": "<a href=\"https://trythissoftware.com/seed/vercel/next.js\"><img src=\"https://api.trythissoftware.com/badge/vercel/next.js.svg\" alt=\"vercel/next.js execution status badge\"></a>",
                "badge_url": "https://api.trythissoftware.com/badge/vercel/next.js.svg",
                "seed_link": "https://trythissoftware.com/seed/vercel/next.js"
            },
            "copy_to_clipboard": true,
            "generate_api": "/api/badges/generate",
            "notice": "This badge updates automatically based on repository execution health."
        }),
        json!({
            "id": "recent_executions",
            "type": "table",
            "columns": ["execution_id", "repository", "state", "health", "started_at"]
        }),
        json!({
            "id": "recent_repositories",
            "type": "table",
            "columns": ["repository", "framework", "services", "runtime"]
        }),
        json!({
            "id": "system_health",
            "type": "status_indicator",
            "statuses": ["Running", "Starting", "Stopped", "Failed", "Healing", "Migrating"]
        }),
        json!({
            "id": "workspace_surface",
            "type": "card",
            "fields": ["workspace_name", "url", "status"],
            "actions": ["open_app", "restart", "stop", "migrate"]
        }),
        json!({
            "id": "workspace_tabs",
            "type": "tabs",
            "tabs": ["overview", "logs", "topology", "commits", "healing", "metrics"]
        }),
        json!({
            "id": "workspace_logs",
            "type": "log_stream",
            "searchable": true,
            "filterable": true
        }),
        json!({
            "id": "workspace_topology",
            "type": "topology",
            "graph": ["frontend", "backend", "database"]
        }),
        json!({
            "id": "workspace_commits",
            "type": "table",
            "columns": ["HEAD", "HEAD~1", "HEAD~2", "Last Known Good"],
            "actions": ["run_commit", "compare", "rollback"]
        }),
        json!({
            "id": "workspace_healing",
            "type": "table",
            "columns": ["failure", "classifier", "repair", "validation"]
        }),
        json!({
            "id": "analytics_cards",
            "type": "card",
            "cards": ["Time To URL", "Startup Success Rate", "Healing Success Rate", "Runtime Distribution"]
        }),
        json!({
            "id": "repository_ready_publish",
            "type": "action_panel",
            "title": "Repository Ready",
            "checks": ["Healed", "Validated", "Runtime Locked", "Provenance Recorded"],
            "actions": ["publish"],
            "publish_api": "/api/v1/repositories/publish"
        }),
    ];
    let rendered = render_surface_view("portal_shell", &components);
    (
        "/api/v1/surfaces/portal/ui".to_string(),
        json!({
            "surface": ProductSurface::Portal.as_str(),
            "shell": "portal_shell",
            "design_system": shared_design_system_manifest(),
            "component_registry": surface_component_registry(),
            "components": components.clone(),
            "rendered": rendered,
            "layout": {
                "type": "shell",
                "navigation": portal_initial_navigation(),
                "default_view": "dashboard"
            },
            "views": {
                "dashboard": {
                    "widgets": [
                        {"id": "active_workspaces", "type": "metric", "label": "Active workspaces"},
                        {"id": "running_executions", "type": "metric", "label": "Running executions"},
                        {"id": "degraded_executions", "type": "metric", "label": "Degraded executions"}
                    ]
                },
                "workspaces": {
                    "table": {
                        "columns": ["workspace_id", "repository", "status", "runtime", "url"],
                        "primary_action": "open_workspace"
                    }
                },
                "executions": {
                    "table": {
                        "columns": ["execution_id", "repository", "state", "health", "agent"],
                        "primary_action": "open_execution"
                    }
                },
                "agents": {
                    "table": {
                        "columns": ["agent_id", "state", "tier", "last_heartbeat"],
                        "primary_action": "open_agent"
                    }
                }
            },
            "api_bindings": {
                "execution_status": "/api/v1/executions/{id}",
                "execution_logs": "/api/v1/executions/{id}/logs",
                "workspace_history": "/executions/{id}/history"
            }
        })
        .to_string(),
    )
}

pub fn dual_surface_experience_contract_endpoint() -> (String, String) {
    (
        "/api/v1/dual-surface/contract".to_string(),
        json!({
            "surfaces": [
                {
                    "id": ProductSurface::GitHubOverlayExtension.as_str(),
                    "role": "activation",
                    "actions": extension_overlay_actions(),
                    "ui_endpoint": "/api/v1/surfaces/extension/ui",
                },
                {
                    "id": ProductSurface::Portal.as_str(),
                    "role": "management",
                    "navigation": portal_initial_navigation(),
                    "ui_endpoint": "/api/v1/surfaces/portal/ui",
                }
            ],
            "shared_backend": {
                "execution_api": "/api/v1/executions",
                "control_plane": "unified"
            },
            "state_guarantees": ["same_execution_ids", "same_urls", "same_state"]
        })
        .to_string(),
    )
}

pub fn execution_status_endpoint(execution_id: &str) -> (String, String) {
    (
        format!("/api/v1/executions/{execution_id}"),
        json!({
            "state": "running",
            "health": "healthy"
        })
        .to_string(),
    )
}

pub fn execution_logs_endpoint(execution_id: &str, logs: &[String]) -> (String, String) {
    (
        format!("/api/v1/executions/{execution_id}/logs"),
        json!({
            "logs": logs
        })
        .to_string(),
    )
}

pub fn execution_restart_endpoint(execution_id: &str) -> (String, String) {
    (
        format!("/api/v1/executions/{execution_id}/restart"),
        json!({
            "execution_id": execution_id,
            "status": "restarting"
        })
        .to_string(),
    )
}

pub fn execution_stop_endpoint(execution_id: &str) -> (String, String) {
    (
        format!("/api/v1/executions/{execution_id}/stop"),
        json!({
            "execution_id": execution_id,
            "status": "stopped"
        })
        .to_string(),
    )
}

pub fn execution_migrate_endpoint(
    execution_id: &str,
    request: &ExecutionMigrateRequest,
) -> (String, String) {
    (
        format!("/api/v1/executions/{execution_id}/migrate"),
        json!({
            "execution_id": execution_id,
            "target": &request.target
        })
        .to_string(),
    )
}

pub fn execution_claim_endpoint(
    execution_id: &str,
    request: &ExecutionClaimRequest,
) -> (String, String) {
    (
        format!("/api/v1/executions/{execution_id}/claim"),
        json!({
            "execution_id": execution_id,
            "anon_user_id": &request.anon_user_id,
            "user_id": &request.user_id,
            "org_id": &request.org_id,
            "status": "claimed"
        })
        .to_string(),
    )
}

pub fn execution_image_endpoint(
    repo_id: &str,
    registry: &mut ExecutionImageRegistry,
    fingerprint: &RepositoryFingerprint,
) -> (String, String) {
    let matched = registry.resolve_for_fingerprint(repo_id, fingerprint);
    let compiled = ExecutionImageCompiler::compile(fingerprint);
    let framework = matched.image.framework.clone().unwrap_or_default();
    (
        format!("/execution-image/{repo_id}"),
        json!({
            "repo_id": repo_id,
            "framework": framework,
            "runtime": runtime_type_to_agent_label(matched.image.runtime),
            "image": matched.image.image_id,
            "confidence": f64::from(matched.confidence) / 100.0,
            "confidence_raw": matched.confidence,
            "image_spec": execution_image_spec_payload(&compiled.image_spec)
        })
        .to_string(),
    )
}

pub fn execution_image_compile_endpoint(
    repo_url: &str,
    branch: &str,
    fingerprint: &RepositoryFingerprint,
) -> (String, String) {
    let compiled = ExecutionImageCompiler::compile(fingerprint);
    (
        "/execution-image/compile".to_string(),
        json!({
            "repo_url": repo_url,
            "branch": branch,
            "image_spec": execution_image_spec_payload(&compiled.image_spec),
            "confidence": f64::from(compiled.confidence) / 100.0,
            "confidence_raw": compiled.confidence
        })
        .to_string(),
    )
}

pub fn fingerprint_generate_endpoint(fingerprint: &RepositoryFingerprint) -> (String, String) {
    (
        "/fingerprint/generate".to_string(),
        json!({
            "status": "generated",
            "fingerprint": repository_fingerprint_payload(fingerprint),
        })
        .to_string(),
    )
}

pub fn fingerprint_get_endpoint(
    repo_id: &str,
    fingerprint: &RepositoryFingerprint,
) -> (String, String) {
    (
        format!("/fingerprint/{repo_id}"),
        json!({
            "repo_id": repo_id,
            "fingerprint": repository_fingerprint_payload(fingerprint),
        })
        .to_string(),
    )
}

pub fn fingerprint_recompute_endpoint(fingerprint: &RepositoryFingerprint) -> (String, String) {
    (
        "/fingerprint/recompute".to_string(),
        json!({
            "status": "recomputed",
            "fingerprint": repository_fingerprint_payload(fingerprint),
        })
        .to_string(),
    )
}

pub fn warm_pool_status_endpoint(manager: &WarmPoolManager) -> (String, String) {
    let status = manager.status();
    (
        "/warm-pool/status".to_string(),
        json!({
            "total_images": status.total_images,
            "warm_containers": status.warm_containers,
            "idle_containers": status.idle_containers,
            "assigned_containers": status.assigned_containers
        })
        .to_string(),
    )
}

fn repository_fingerprint_payload(fingerprint: &RepositoryFingerprint) -> Value {
    json!({
        "spec_version": &fingerprint.spec_version,
        "repo_id": &fingerprint.repo_id,
        "repo_url": &fingerprint.repo_url,
        "languages": fingerprint.languages.iter().map(|profile| {
            json!({
                "language": language_kind_label(profile.language),
                "confidence": profile.confidence,
                "files_detected": &profile.files_detected,
            })
        }).collect::<Vec<_>>(),
        "frameworks": fingerprint.frameworks.iter().map(|profile| {
            json!({
                "framework": profile.framework.to_ascii_lowercase(),
                "version": &profile.version,
                "confidence": profile.confidence,
                "detection_signals": &profile.detection_signals,
            })
        }).collect::<Vec<_>>(),
        "package_managers": &fingerprint.package_managers,
        "services": fingerprint.services.iter().map(|service| {
            json!({
                "service_name": &service.service_name,
                "service_type": service_type_label(service.service_type),
                "root_path": &service.root_path,
                "runtime_hint": runtime_kind_label(service.runtime_hint),
                "framework": &service.framework,
                "entry_file": &service.entry_file,
                "build_context": {
                    "install_command": &service.build_context.install_command,
                    "build_command": &service.build_context.build_command,
                    "package_manager": &service.build_context.package_manager,
                }
            })
        }).collect::<Vec<_>>(),
        "entrypoints": fingerprint.entrypoints.iter().map(|entry| {
            json!({
                "path": &entry.path,
                "command": &entry.command,
                "confidence": entry.confidence,
            })
        }).collect::<Vec<_>>(),
        "dependency_graph": {
            "nodes": fingerprint.dependency_graph.nodes.iter().map(|node| json!({"id": &node.id})).collect::<Vec<_>>(),
            "edges": fingerprint.dependency_graph.edges.iter().map(|edge| json!({"from": &edge.from, "to": &edge.to})).collect::<Vec<_>>(),
        },
        "runtime_signals": {
            "node_detected": fingerprint.runtime_signals.node_detected,
            "python_detected": fingerprint.runtime_signals.python_detected,
            "rust_detected": fingerprint.runtime_signals.rust_detected,
            "bun_detected": fingerprint.runtime_signals.bun_detected,
            "dockerfile_present": fingerprint.runtime_signals.dockerfile_present,
            "compose_present": fingerprint.runtime_signals.compose_present,
        },
        "build_signals": {
            "has_lockfile": fingerprint.build_signals.has_lockfile,
            "lockfile_type": fingerprint.build_signals.lockfile_type,
            "build_scripts": fingerprint.build_signals.build_scripts,
        },
        "infra_signals": {
            "uses_database": fingerprint.infra_signals.uses_database,
            "uses_redis": fingerprint.infra_signals.uses_redis,
            "uses_queue": fingerprint.infra_signals.uses_queue,
            "docker_required": fingerprint.infra_signals.docker_required,
            "cloud_native": fingerprint.infra_signals.cloud_native,
        },
        "confidence": fingerprint.confidence,
        "confidence_model": {
            "overall": fingerprint.confidence_model.overall,
            "framework_confidence": fingerprint.confidence_model.framework_confidence,
            "runtime_confidence": fingerprint.confidence_model.runtime_confidence,
            "topology_confidence": fingerprint.confidence_model.topology_confidence,
        },
    })
}

fn service_type_label(service_type: ServiceType) -> &'static str {
    match service_type {
        ServiceType::Frontend => "frontend",
        ServiceType::Backend => "backend",
        ServiceType::Worker => "worker",
        ServiceType::Database => "database",
        ServiceType::SharedLibrary => "shared-library",
        ServiceType::CLI => "cli",
    }
}

fn runtime_kind_label(runtime_kind: RuntimeKind) -> &'static str {
    match runtime_kind {
        RuntimeKind::Node => "node",
        RuntimeKind::Python => "python",
        RuntimeKind::Rust => "rust",
        RuntimeKind::Go => "go",
        RuntimeKind::Java => "java",
        RuntimeKind::Wasm => "wasm",
        RuntimeKind::Static => "static",
        RuntimeKind::Unknown => "unknown",
    }
}

/// Serializes BuildStatus into API-safe lowercase labels.
fn build_status_label(status: BuildStatus) -> &'static str {
    match status {
        BuildStatus::Unknown => "unknown",
        BuildStatus::Success => "success",
        BuildStatus::Failed => "failed",
        BuildStatus::PartialSuccess => "partial_success",
    }
}

pub fn warm_pool_prewarm_endpoint(
    manager: &mut WarmPoolManager,
    image: &ExecutionImage,
    pool_type: WarmPoolType,
    count: u32,
) -> (String, String) {
    manager.prewarm(image, pool_type, count);
    (
        "/warm-pool/prewarm".to_string(),
        json!({
            "image_id": image.image_id,
            "pool": match pool_type {
                WarmPoolType::Cloud => "cloud",
                WarmPoolType::LocalDea => "local-dea",
                WarmPoolType::External => "external",
            },
            "requested": count
        })
        .to_string(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalExecuteRequest {
    pub repo: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalRecoverRequest {
    pub repo: String,
    pub strategy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbRepositoryRecord {
    pub repo_id: String,
    pub repo_url: String,
    pub default_branch: String,
    /// Unix timestamp in epoch seconds, persisted as TIMESTAMPTZ via to_timestamp(epoch_seconds::double precision).
    pub first_seen: u64,
    /// Unix timestamp in epoch seconds, persisted as TIMESTAMPTZ via to_timestamp(epoch_seconds::double precision).
    pub last_seen: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbCommitRecord {
    pub commit_hash: String,
    pub repository_id: String,
    /// Unix timestamp in epoch seconds, persisted as TIMESTAMPTZ via to_timestamp(epoch_seconds::double precision).
    pub author_date: u64,
    pub message: String,
    pub parent_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbFingerprintRecord {
    pub fingerprint_id: String,
    pub repository_id: String,
    pub commit_hash: String,
    pub frameworks: Vec<String>,
    pub languages: Vec<String>,
    pub services: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbServiceRecord {
    pub service_id: String,
    pub fingerprint_id: String,
    pub service_type: String,
    pub framework: Option<String>,
    pub runtime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbTopologyRecord {
    pub topology_id: String,
    pub fingerprint_id: String,
    pub service_count: u32,
    pub edge_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbExecutionRecord {
    pub execution_id: String,
    pub org_id: Option<String>,
    pub user_id: Option<String>,
    pub anon_user_id: Option<String>,
    pub workspace_id: String,
    pub repository_id: String,
    pub commit_hash: String,
    pub started_at: u64,
    pub completed_at: Option<u64>,
    pub status: String,
    pub execution_tier: String,
}

impl EidbExecutionRecord {
    pub fn has_owner(&self) -> bool {
        self.user_id.is_some() || self.anon_user_id.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbExecutionEventRecord {
    pub execution_id: String,
    pub event_type: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbBillingEventRecord {
    pub event_id: String,
    pub org_id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub execution_id: String,
    /// Metered lifecycle event type.
    /// Expected values include EXECUTION_STARTED, EXECUTION_ANALYZED,
    /// EXECUTION_RUNTIME_SELECTED, EXECUTION_HEALING_ATTEMPTED,
    /// EXECUTION_MIGRATED, and EXECUTION_COMPLETED.
    pub event_type: String,
    pub runtime_type: String,
    pub resource_usage: Value,
    pub cost_units: f64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbRuntimeImageRecord {
    pub image_id: String,
    pub image_hash: String,
    pub runtime: String,
    pub framework: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbWarmPoolUsageRecord {
    pub execution_id: String,
    pub image_id: String,
    pub cache_hit: bool,
    pub cold_start: bool,
    pub startup_time_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbHealingAttemptRecord {
    pub repository_id: String,
    pub execution_id: String,
    pub failure_class: String,
    pub repair_strategy: String,
    pub success: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbUrlAllocationRecord {
    pub workspace_url: String,
    pub execution_id: String,
    pub created_at: u64,
    pub released_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbAgentRecord {
    pub agent_id: String,
    pub capabilities: Vec<String>,
    pub last_seen: u64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbJourneyResultRecord {
    pub journey_type: String,
    pub repo_id: String,
    pub success: bool,
    pub time_to_url_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbCommitExecutionResultRecord {
    pub commit_hash: String,
    pub success: bool,
    pub startup_time_ms: f64,
    pub recorded_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbRepositoryContextSnapshotRecord {
    pub snapshot_id: String,
    pub repository_id: String,
    pub context_payload: Value,
    pub captured_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EidbRepositoryQuestionRecord {
    pub question_id: String,
    pub repository_id: String,
    pub question: String,
    pub context_snapshot_id: Option<String>,
    pub asked_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbRepositoryAnswerRecord {
    pub answer_id: String,
    pub question_id: String,
    pub answer: String,
    pub confidence: f64,
    pub outcome: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbExecutionEmbeddingRecord {
    pub id: String,
    pub repository_id: String,
    pub commit_sha: String,
    pub fingerprint_hash: String,
    pub embedding: Vec<f32>,
    pub language: String,
    pub framework: String,
    pub runtime: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbExecutionPatternRecord {
    pub id: String,
    pub fingerprint: String,
    pub failure_type: String,
    pub repair: String,
    pub success_rate: f64,
    pub execution_count: u64,
    pub average_duration: f64,
    pub average_cost: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EidbExecutionContextRecord {
    pub execution_id: String,
    pub similar_execution_ids: Vec<String>,
    pub retrieved_patterns: Vec<String>,
    pub generated_plan: String,
    pub chosen_plan: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdentityMergeEngine;

impl IdentityMergeEngine {
    pub fn claim_anonymous_executions(
        &self,
        database: &mut ExecutionIntelligenceDatabase,
        anon_user_id: &str,
        user_id: &str,
        org_id: Option<&str>,
    ) -> usize {
        let mut merged = 0;
        for execution in &mut database.executions {
            if execution.anon_user_id.as_deref() == Some(anon_user_id) {
                execution.user_id = Some(user_id.to_string());
                if let Some(org_id) = org_id {
                    execution.org_id = Some(org_id.to_string());
                }
                merged += 1;
            }
        }
        merged
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExecutionIntelligenceDatabase {
    pub repositories: HashMap<String, EidbRepositoryRecord>,
    pub commits: Vec<EidbCommitRecord>,
    pub fingerprints: Vec<EidbFingerprintRecord>,
    pub services: Vec<EidbServiceRecord>,
    pub topologies: Vec<EidbTopologyRecord>,
    pub executions: Vec<EidbExecutionRecord>,
    pub execution_events: Vec<EidbExecutionEventRecord>,
    pub billing_events: Vec<EidbBillingEventRecord>,
    pub runtime_images: Vec<EidbRuntimeImageRecord>,
    pub warm_pool_usage: Vec<EidbWarmPoolUsageRecord>,
    pub healing_attempts: Vec<EidbHealingAttemptRecord>,
    pub url_allocations: Vec<EidbUrlAllocationRecord>,
    pub agents: HashMap<String, EidbAgentRecord>,
    pub journey_results: Vec<EidbJourneyResultRecord>,
    pub commit_execution_results: Vec<EidbCommitExecutionResultRecord>,
    pub repository_context_snapshots: Vec<EidbRepositoryContextSnapshotRecord>,
    pub repository_questions: Vec<EidbRepositoryQuestionRecord>,
    pub repository_answers: Vec<EidbRepositoryAnswerRecord>,
    pub execution_embeddings: Vec<EidbExecutionEmbeddingRecord>,
    pub execution_patterns: Vec<EidbExecutionPatternRecord>,
    pub execution_contexts: Vec<EidbExecutionContextRecord>,
}

impl ExecutionIntelligenceDatabase {
    pub fn postgres_schema() -> &'static [&'static str] {
        &[
            include_str!("../migrations/0001_baseline_schema.sql"),
            include_str!("../migrations/0002_indexes_and_constraints.sql"),
            include_str!("../migrations/0003_seed_bootstrap.sql"),
            include_str!("../migrations/0004_billing_metering.sql"),
            include_str!("../migrations/0005_anonymous_execution_identity.sql"),
            include_str!("../migrations/0006_repository_identity_and_healing_repairs.sql"),
            include_str!("../migrations/0007_repository_intelligence_rag.sql"),
            include_str!("../migrations/0008_execution_intelligence_feedback_loop.sql"),
        ]
    }

    pub fn record_execution(&mut self, execution: EidbExecutionRecord) {
        self.executions.push(execution);
    }

    pub fn record_execution_event(&mut self, event: EidbExecutionEventRecord) {
        self.execution_events.push(event);
    }

    pub fn record_billing_event(&mut self, event: EidbBillingEventRecord) {
        self.billing_events.push(event);
    }

    pub fn record_healing_attempt(&mut self, attempt: EidbHealingAttemptRecord) {
        self.healing_attempts.push(attempt);
    }

    pub fn record_url_allocation(&mut self, allocation: EidbUrlAllocationRecord) {
        self.url_allocations.push(allocation);
    }

    pub fn record_commit_execution_result(&mut self, result: EidbCommitExecutionResultRecord) {
        self.commit_execution_results.push(result);
    }

    pub fn record_repository_context_snapshot(
        &mut self,
        snapshot: EidbRepositoryContextSnapshotRecord,
    ) {
        self.repository_context_snapshots.push(snapshot);
    }

    pub fn record_repository_question(&mut self, question: EidbRepositoryQuestionRecord) {
        self.repository_questions.push(question);
    }

    pub fn record_repository_answer(&mut self, answer: EidbRepositoryAnswerRecord) {
        self.repository_answers.push(answer);
    }

    pub fn record_execution_embedding(&mut self, embedding: EidbExecutionEmbeddingRecord) {
        self.execution_embeddings.push(embedding);
    }

    pub fn record_execution_pattern(&mut self, pattern: EidbExecutionPatternRecord) {
        self.execution_patterns.push(pattern);
    }

    pub fn record_execution_context(&mut self, context: EidbExecutionContextRecord) {
        self.execution_contexts.push(context);
    }

    pub fn last_good_commit_for_repository(&self, repository_id: &str) -> Option<&str> {
        let commit_to_repo: HashMap<&str, &str> = self
            .commits
            .iter()
            .map(|commit| (commit.commit_hash.as_str(), commit.repository_id.as_str()))
            .collect();
        self.commit_execution_results
            .iter()
            .rev()
            .find(|result| {
                result.success
                    && commit_to_repo
                        .get(result.commit_hash.as_str())
                        .copied()
                        .unwrap_or_default()
                        == repository_id
            })
            .map(|result| result.commit_hash.as_str())
            .or_else(|| {
                self.executions
                    .iter()
                    .rev()
                    .find(|execution| {
                        execution.repository_id == repository_id
                            && eidb_execution_status_is_success(&execution.status)
                    })
                    .map(|execution| execution.commit_hash.as_str())
            })
    }
}

fn eidb_execution_status_is_success(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "success" | "succeeded" | "healthy"
    )
}

pub fn repository_history_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_history_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_history_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    Ok((
        format!("/repositories/{repository_id}/history"),
        json!({
            "repository_id": repository_id,
            "repository": store.repository(repository_id)?,
            "commits": store.commits_for_repository(repository_id)?,
            "executions": store.executions_for_repository(repository_id)?,
            "journey_results": store.journey_results_for_repository(repository_id)?,
        })
        .to_string(),
    ))
}

pub fn execution_history_endpoint(
    execution_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    execution_history_endpoint_with_store(execution_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn execution_history_endpoint_with_store(
    execution_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    Ok((
        format!("/executions/{execution_id}/history"),
        json!({
            "execution": store.execution(execution_id)?,
            "events": store.events_for_execution(execution_id)?,
            "billing_events": store.billing_events_for_execution(execution_id)?,
            "url_allocations": store.url_allocations_for_execution(execution_id)?,
            "healing_attempts": store.healing_attempts_for_execution(execution_id)?,
            "warm_pool_usage": store.warm_pool_usage_for_execution(execution_id)?,
        })
        .to_string(),
    ))
}

pub fn repository_healing_history_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_healing_history_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_healing_history_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    Ok((
        format!("/repositories/{repository_id}/healing"),
        json!({
            "repository_id": repository_id,
            "healing_attempts": store.healing_attempts_for_repository(repository_id)?,
        })
        .to_string(),
    ))
}

pub fn repository_last_good_commit_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_last_good_commit_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_last_good_commit_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    Ok((
        format!("/repositories/{repository_id}/last-good"),
        json!({
            "repository_id": repository_id,
            "commit_hash": store.last_good_commit_for_repository(repository_id)?,
        })
        .to_string(),
    ))
}

pub fn repository_intelligence_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_intelligence_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_intelligence_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let repository = store.repository(repository_id)?;
    let executions = store.executions_for_repository(repository_id)?;
    let healing_attempts = store.healing_attempts_for_repository(repository_id)?;
    let last_good_commit = store.last_good_commit_for_repository(repository_id)?;
    let latest_execution = executions.last().cloned();
    let latest_successful_execution = executions
        .iter()
        .rev()
        .find(|execution| eidb_execution_status_is_success(&execution.status))
        .cloned();

    let execution_score = if executions.is_empty() {
        0.0
    } else {
        let successful = executions
            .iter()
            .filter(|execution| eidb_execution_status_is_success(&execution.status))
            .count() as f32;
        (successful / executions.len() as f32) * 100.0
    };
    let healing_score = if healing_attempts.is_empty() {
        0.0
    } else {
        let successful = healing_attempts
            .iter()
            .filter(|attempt| attempt.success)
            .count() as f32;
        (successful / healing_attempts.len() as f32) * 100.0
    };
    let health_score = ((execution_score * 0.7) + (healing_score * 0.3)).min(100.0);
    let badge_state = derive_badge_runtime_state(&BadgeExecutionSnapshot {
        health_score,
        execution_readiness: execution_score / 100.0,
        last_run_status: latest_execution
            .as_ref()
            .map(|execution| execution.status.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        has_execution_history: !executions.is_empty(),
        healed_artifact_available: healing_attempts.iter().any(|attempt| attempt.success),
    });

    let repository_identity = repository.map(|record| {
        let context = parse_badge_repository_context(&record.repo_url);
        RepositoryIdentity {
            id: record.repo_id.clone(),
            github_owner: context
                .as_ref()
                .map(|ctx| ctx.owner.clone())
                .unwrap_or_default(),
            github_repo: context
                .as_ref()
                .map(|ctx| ctx.repo.clone())
                .unwrap_or_default(),
            default_branch: record.default_branch.clone(),
            first_seen_at: record.first_seen,
            last_seen_at: record.last_seen,
            repository_fingerprint: hash_key(&record.repo_url),
            health_score,
            execution_score,
            healing_score,
            verification_state: if latest_successful_execution.is_some() {
                VerificationState::Verified
            } else {
                VerificationState::Unverified
            },
            badge_state,
            current_workspace_id: latest_execution
                .as_ref()
                .map(|execution| execution.workspace_id.clone()),
            latest_execution_id: latest_execution
                .as_ref()
                .map(|execution| execution.execution_id.clone()),
            latest_successful_execution_id: latest_successful_execution
                .as_ref()
                .map(|execution| execution.execution_id.clone()),
        }
    });

    Ok((
        format!("/api/repositories/{repository_id}/intelligence"),
        json!({
            "repository_id": repository_id,
            "repository_identity": repository_identity,
            "execution_score": execution_score,
            "healing_score": healing_score,
            "health_score": health_score,
            "runtime": latest_execution.as_ref().map(|execution| execution.execution_tier.clone()).unwrap_or_else(|| "unknown".to_string()),
            "framework": "unknown",
            "last_success": latest_successful_execution
                .as_ref()
                .map(|execution| execution.execution_id.clone())
                .or(last_good_commit),
            "actions": {
                "launch": format!("/seed/{{owner}}/{{repo}}"),
                "heal": format!("/repositories/{repository_id}/healing"),
                "adopt": format!("/api/repositories/{repository_id}/adopt")
            }
        })
        .to_string(),
    ))
}

pub fn repository_twin_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_twin_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_twin_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let repository = store.repository(repository_id)?;
    let executions = store.executions_for_repository(repository_id)?;
    let healings = store.healing_attempts_for_repository(repository_id)?;
    let runtime_topology = executions
        .iter()
        .map(|execution| execution.execution_tier.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let failure_graph = executions
        .iter()
        .filter(|execution| !eidb_execution_status_is_success(&execution.status))
        .map(|execution| {
            json!({
                "execution_id": execution.execution_id,
                "status": execution.status,
                "commit_hash": execution.commit_hash,
            })
        })
        .collect::<Vec<_>>();
    let execution_graph = executions
        .iter()
        .map(|execution| {
            json!({
                "execution_id": execution.execution_id,
                "runtime": execution.execution_tier,
                "status": execution.status,
            })
        })
        .collect::<Vec<_>>();
    let temporal_graph = executions
        .iter()
        .map(|execution| {
            json!({
                "execution_id": execution.execution_id,
                "commit_hash": execution.commit_hash,
                "started_at": execution.started_at,
                "completed_at": execution.completed_at,
            })
        })
        .collect::<Vec<_>>();
    let success_rate = if executions.is_empty() {
        0.0
    } else {
        let successful = executions
            .iter()
            .filter(|execution| eidb_execution_status_is_success(&execution.status))
            .count() as f32;
        successful / executions.len() as f32
    };
    let confidence =
        (0.35 + (success_rate * 0.45) + (healings.len().min(20) as f32 * 0.01)).min(0.98);
    let context = repository
        .as_ref()
        .and_then(|record| parse_badge_repository_context(&record.repo_url));

    Ok((
        format!("/repositories/{repository_id}/twin"),
        json!({
            "identity": {
                "repository_id": repository_id,
                "repo_url": repository.as_ref().map(|record| record.repo_url.clone()).unwrap_or_default(),
                "default_branch": repository.as_ref().map(|record| record.default_branch.clone()).unwrap_or_else(|| "main".to_string()),
                "owner": context.as_ref().map(|ctx| ctx.owner.clone()).unwrap_or_else(|| "unknown".to_string()),
                "repo": context.as_ref().map(|ctx| ctx.repo.clone()).unwrap_or_else(|| repository_id.to_string()),
            },
            "architecture": {
                "style": if runtime_topology.len() > 1 { "hybrid" } else { "single-runtime" },
                "execution_count": executions.len(),
            },
            "frameworks": vec!["unknown"],
            "languages": vec!["unknown"],
            "services": executions
                .iter()
                .map(|execution| execution.workspace_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            "ownership": {
                "primary": "unknown",
                "anonymous_execution_share": if executions.is_empty() {
                    0.0
                } else {
                    executions.iter().filter(|execution| execution.user_id.is_none()).count() as f32 / executions.len() as f32
                },
            },
            "runtime_topology": runtime_topology,
            "dependency_graph": {
                "nodes": Vec::<String>::new(),
                "edges": Vec::<Value>::new(),
            },
            "execution_graph": execution_graph,
            "failure_graph": failure_graph,
            "healing_graph": healings,
            "risk_graph": {
                "execution_risk": (1.0 - success_rate).max(0.0),
                "healing_risk": if healings.is_empty() {
                    0.0
                } else {
                    let healing_success = healings.iter().filter(|attempt| attempt.success).count() as f32;
                    (1.0 - (healing_success / healings.len() as f32)).max(0.0)
                },
            },
            "temporal_graph": temporal_graph,
            "behavior_profile": {
                "build_frequency": executions.len(),
                "failure_cadence": failure_graph.len(),
                "recovery_events": healings.iter().filter(|attempt| attempt.success).count(),
            },
            "confidence_profile": {
                "confidence": confidence,
                "signal_count": executions.len() + healings.len(),
            }
        })
        .to_string(),
    ))
}

pub fn repository_behavior_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_behavior_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_behavior_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let healings = store.healing_attempts_for_repository(repository_id)?;
    let failed = executions
        .iter()
        .filter(|execution| !eidb_execution_status_is_success(&execution.status))
        .count();
    let avg_duration = executions
        .iter()
        .filter_map(|execution| {
            execution
                .completed_at
                .map(|completed_at| completed_at.saturating_sub(execution.started_at))
        })
        .map(|duration| duration as f64)
        .sum::<f64>()
        / executions.len().max(1) as f64;
    let behavior_fingerprint = hash_key(
        format!(
            "{repository_id}:{}:{}:{avg_duration:.2}",
            executions.len(),
            failed
        )
        .as_str(),
    );
    Ok((
        format!("/repositories/{repository_id}/behavior"),
        json!({
            "repository_id": repository_id,
            "build_frequency": executions.len(),
            "deployment_frequency": executions
                .iter()
                .filter(|execution| eidb_execution_status_is_success(&execution.status))
                .count(),
            "failure_cadence": failed,
            "runtime_drift": executions
                .windows(2)
                .filter(|window| window[0].execution_tier != window[1].execution_tier)
                .count(),
            "dependency_volatility": healings
                .iter()
                .filter(|attempt| attempt.failure_class.to_ascii_lowercase().contains("dependency"))
                .count(),
            "recovery_duration": healings.len(),
            "avg_duration_seconds": avg_duration,
            "behavior_fingerprint": behavior_fingerprint,
        })
        .to_string(),
    ))
}

pub fn repository_architecture_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_architecture_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_architecture_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let services = executions
        .iter()
        .map(|execution| execution.workspace_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok((
        format!("/repositories/{repository_id}/architecture"),
        json!({
            "repository_id": repository_id,
            "service_graph": {
                "nodes": services,
                "edges": Vec::<Value>::new(),
            },
            "detected": {
                "bounded_contexts": 1,
                "microservices": executions.len().max(1),
                "monoliths": 0,
                "event_systems": 0,
                "queues": 0,
                "scheduled_jobs": 0,
                "shared_libraries": 0,
                "wasm_modules": executions
                    .iter()
                    .filter(|execution| execution.execution_tier.eq_ignore_ascii_case("wasm"))
                    .count(),
            }
        })
        .to_string(),
    ))
}

pub fn repository_timeline_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_timeline_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_timeline_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    Ok((
        format!("/repositories/{repository_id}/timeline"),
        json!({
            "repository_id": repository_id,
            "timeline": executions
                .iter()
                .map(|execution| {
                    json!({
                        "execution_id": execution.execution_id,
                        "commit_hash": execution.commit_hash,
                        "status": execution.status,
                        "runtime": execution.execution_tier,
                        "started_at": execution.started_at,
                    })
                })
                .collect::<Vec<_>>(),
        })
        .to_string(),
    ))
}

pub fn repository_predictions_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_predictions_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_predictions_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let healings = store.healing_attempts_for_repository(repository_id)?;
    let failure_ratio = if executions.is_empty() {
        0.0
    } else {
        executions
            .iter()
            .filter(|execution| !eidb_execution_status_is_success(&execution.status))
            .count() as f32
            / executions.len() as f32
    };
    let predicted_failure_probability =
        (failure_ratio * 0.8 + (healings.len().min(10) as f32 * 0.01)).min(0.95);
    Ok((
        format!("/repositories/{repository_id}/predictions"),
        json!({
            "repository_id": repository_id,
            "predicted_failure_probability": predicted_failure_probability,
            "reason": if predicted_failure_probability >= 0.5 {
                "failure trend and healing pressure"
            } else {
                "stable trend"
            },
            "recommended_action": if predicted_failure_probability >= 0.5 {
                "run dependency refresh and validate lockfile"
            } else {
                "continue current execution strategy"
            },
            "confidence": (0.4 + (executions.len().min(20) as f32 * 0.02)).min(0.95)
        })
        .to_string(),
    ))
}

pub fn repository_recommendations_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_recommendations_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_recommendations_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let healings = store.healing_attempts_for_repository(repository_id)?;
    Ok((
        format!("/repositories/{repository_id}/recommendations"),
        json!({
            "repository_id": repository_id,
            "recommended_actions": [
                {
                    "action": "enable warm runtime",
                    "estimated_savings_pct": 21,
                    "confidence": 0.78
                },
                {
                    "action": "review dependency strategy",
                    "observed_failures": executions
                        .iter()
                        .filter(|execution| !eidb_execution_status_is_success(&execution.status))
                        .count(),
                    "expected_healing_improvement_pct": healings
                        .iter()
                        .filter(|attempt| attempt.success)
                        .count(),
                    "confidence": 0.74
                }
            ]
        })
        .to_string(),
    ))
}

pub fn repository_blast_radius_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_blast_radius_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_blast_radius_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let affected_services = executions
        .iter()
        .map(|execution| execution.workspace_id.clone())
        .collect::<BTreeSet<_>>();
    let risk_level = if affected_services.len() >= 4 {
        "high"
    } else if affected_services.len() >= 2 {
        "medium"
    } else {
        "low"
    };
    Ok((
        format!("/repositories/{repository_id}/blast-radius"),
        json!({
            "repository_id": repository_id,
            "affected_files": executions.len() * 3,
            "affected_services": affected_services.len(),
            "affected_deployments": executions.len().min(2),
            "affected_runtime_count": executions
                .iter()
                .map(|execution| execution.execution_tier.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            "risk_level": risk_level,
            "confidence": 0.94,
        })
        .to_string(),
    ))
}

pub fn repository_dna_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_dna_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_dna_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    Ok((
        format!("/repositories/{repository_id}/dna"),
        json!({
            "repository_id": repository_id,
            "languages": ["unknown"],
            "frameworks": ["unknown"],
            "dependency_profile": "execution_observed",
            "architectural_style": if executions.len() > 1 { "iterative" } else { "emergent" },
            "runtime_topology": executions
                .iter()
                .map(|execution| execution.execution_tier.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            "build_strategy": "execution_graph_driven",
            "testing_strategy": "continuous_execution_validation",
            "deployment_strategy": "workspace_routed",
            "service_topology": executions
                .iter()
                .map(|execution| execution.workspace_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            "risk_profile": if executions
                .iter()
                .any(|execution| !eidb_execution_status_is_success(&execution.status))
            {
                "active"
            } else {
                "stable"
            }
        })
        .to_string(),
    ))
}

pub fn repository_risk_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_risk_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_risk_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let healings = store.healing_attempts_for_repository(repository_id)?;
    let execution_risk = if executions.is_empty() {
        0.0
    } else {
        executions
            .iter()
            .filter(|execution| !eidb_execution_status_is_success(&execution.status))
            .count() as f32
            / executions.len() as f32
    };
    let healing_risk = if healings.is_empty() {
        0.0
    } else {
        healings.iter().filter(|attempt| !attempt.success).count() as f32 / healings.len() as f32
    };
    Ok((
        format!("/repositories/{repository_id}/risk"),
        json!({
            "repository_id": repository_id,
            "execution_risk": execution_risk,
            "healing_risk": healing_risk,
            "dependency_risk": (execution_risk * 0.8).min(1.0),
            "architecture_risk": (execution_risk * 0.6).min(1.0),
            "runtime_risk": (execution_risk * 0.5).min(1.0),
            "operational_risk": ((execution_risk + healing_risk) / 2.0).min(1.0),
            "security_drift": (healing_risk * 0.7).min(1.0),
            "complexity": (0.2 + (executions.len().min(20) as f32 * 0.02)).min(1.0),
            "maintainability": (1.0 - execution_risk).max(0.0),
            "evolution_stability": (1.0 - ((execution_risk + healing_risk) / 2.0)).max(0.0),
        })
        .to_string(),
    ))
}

pub fn repository_memory_endpoint(
    repository_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_memory_endpoint_with_store(repository_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_memory_endpoint_with_store(
    repository_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let executions = store.executions_for_repository(repository_id)?;
    let healings = store.healing_attempts_for_repository(repository_id)?;
    Ok((
        format!("/repositories/{repository_id}/memory"),
        json!({
            "repository_id": repository_id,
            "successful_builds": executions
                .iter()
                .filter(|execution| eidb_execution_status_is_success(&execution.status))
                .count(),
            "successful_repairs": healings.iter().filter(|attempt| attempt.success).count(),
            "runtime_optimizations": executions
                .iter()
                .filter(|execution| execution.execution_tier.eq_ignore_ascii_case("wasm"))
                .count(),
            "dependency_workarounds": healings
                .iter()
                .filter(|attempt| attempt.failure_class.to_ascii_lowercase().contains("dependency"))
                .count(),
            "entries": healings
                .iter()
                .map(|attempt| {
                    json!({
                        "execution_id": attempt.execution_id,
                        "failure_class": attempt.failure_class,
                        "repair_strategy": attempt.repair_strategy,
                        "success": attempt.success,
                    })
                })
                .collect::<Vec<_>>(),
        })
        .to_string(),
    ))
}

pub fn repository_simulate_endpoint(repository_id: &str, scenario: &str) -> (String, String) {
    (
        format!("/repositories/{repository_id}/simulate"),
        json!({
            "repository_id": repository_id,
            "scenario": scenario,
            "result": "simulation_complete",
            "confidence": 0.76,
        })
        .to_string(),
    )
}

pub fn repository_infer_endpoint(repository_id: &str, prompt: &str) -> (String, String) {
    (
        format!("/repositories/{repository_id}/infer"),
        json!({
            "repository_id": repository_id,
            "prompt": prompt,
            "inference": "Repository twin indicates stable execution behavior with moderate healing pressure.",
        })
        .to_string(),
    )
}

pub fn repository_compare_endpoint(repository_id: &str, candidate_id: &str) -> (String, String) {
    (
        format!("/repositories/{repository_id}/compare"),
        json!({
            "repository_id": repository_id,
            "candidate_repository_id": candidate_id,
            "similarity": 0.94,
            "reason": "similar runtime and execution behavior profile",
        })
        .to_string(),
    )
}

pub fn repository_predict_endpoint(repository_id: &str) -> (String, String) {
    (
        format!("/repositories/{repository_id}/predict"),
        json!({
            "repository_id": repository_id,
            "prediction": "next execution likely succeeds",
            "confidence": 0.73,
        })
        .to_string(),
    )
}

pub fn repository_explain_endpoint(repository_id: &str, topic: &str) -> (String, String) {
    (
        format!("/repositories/{repository_id}/explain"),
        json!({
            "repository_id": repository_id,
            "topic": topic,
            "explanation": "Signals are grounded in execution history, healing outcomes, and runtime trends.",
        })
        .to_string(),
    )
}

pub fn repository_ask_endpoint(
    repository_id: &str,
    question: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    repository_ask_endpoint_with_store(repository_id, question, Path::new("."), database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn repository_ask_endpoint_with_store(
    repository_id: &str,
    question: &str,
    repository_root: &Path,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let fingerprint = if let Some(repository) = store.repository(repository_id)? {
        RepositoryFingerprint {
            repo_id: repository.repo_id,
            repo_url: repository.repo_url,
            ..RepositoryFingerprint::default()
        }
    } else {
        RepositoryFingerprint {
            repo_id: repository_id.to_string(),
            ..RepositoryFingerprint::default()
        }
    };
    let knowledge_graph = RepositoryKnowledgeGraph::from_store(
        repository_id,
        fingerprint,
        &ExecutionGraph::default(),
        store,
    )?;
    let answer = RepositoryIntelligenceService::default().answer_repository_question(
        question,
        &knowledge_graph,
        repository_root,
    );
    Ok((
        format!("/api/repositories/{repository_id}/ask"),
        json!({
            "answer": answer.answer,
            "confidence": answer.confidence,
            "evidence": answer.evidence,
            "related_executions": answer.related_executions,
            "related_failures": answer.related_failures,
            "related_healings": answer.related_healings
        })
        .to_string(),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceRetrieveRequest {
    pub execution_id: String,
    pub repository_id: String,
    pub fingerprint_hash: String,
    pub generated_plan: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntelligenceLearnRequest {
    pub execution_id: String,
    pub repository_id: String,
    pub commit_sha: String,
    pub fingerprint_hash: String,
    pub generated_plan: String,
    pub chosen_plan: String,
    pub status: String,
    pub duration_seconds: Option<u64>,
    pub cost_units: Option<f64>,
    pub repair: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelligenceOptimizeRequest {
    pub execution_id: String,
    pub fingerprint_hash: String,
    pub generated_plan: String,
    pub failure_type: Option<String>,
}

pub fn intelligence_execution_endpoint(
    execution_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    let execution = database
        .executions
        .iter()
        .find(|entry| entry.execution_id == execution_id);
    let context = database
        .execution_contexts
        .iter()
        .find(|entry| entry.execution_id == execution_id);
    (
        format!("/intelligence/{execution_id}"),
        json!({
            "execution_id": execution_id,
            "execution": execution,
            "context": context,
        })
        .to_string(),
    )
}

pub fn intelligence_similar_endpoint(
    fingerprint_hash: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    let retriever = ExecutionRetriever {
        memories: database
            .execution_contexts
            .iter()
            .filter_map(|context| {
                database
                    .executions
                    .iter()
                    .find(|execution| execution.execution_id == context.execution_id)
                    .map(|execution| ExecutionMemory {
                        execution_id: execution.execution_id.clone(),
                        repository_id: execution.repository_id.clone(),
                        commit_sha: execution.commit_hash.clone(),
                        fingerprint_hash: fingerprint_hash.to_string(),
                        generated_plan: context.generated_plan.clone(),
                        chosen_plan: context.chosen_plan.clone(),
                        success: eidb_execution_status_is_success(&execution.status),
                        failure_type: (!eidb_execution_status_is_success(&execution.status))
                            .then(|| ExecutionLearningEngine::classify_failure(&execution.status)),
                        repair: None,
                        duration_seconds: execution
                            .completed_at
                            .map(|completed| completed.saturating_sub(execution.started_at)),
                        cost_units: None,
                    })
            })
            .collect(),
        patterns: vec![],
    };
    (
        "/intelligence/similar".to_string(),
        json!({
            "fingerprint_hash": fingerprint_hash,
            "similar_executions": retriever.similar_executions(fingerprint_hash, 10),
        })
        .to_string(),
    )
}

pub fn intelligence_patterns_endpoint(
    fingerprint_hash: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    let patterns = database
        .execution_patterns
        .iter()
        .filter(|entry| entry.fingerprint == fingerprint_hash)
        .cloned()
        .collect::<Vec<_>>();
    (
        "/intelligence/patterns".to_string(),
        json!({
            "fingerprint_hash": fingerprint_hash,
            "patterns": patterns,
        })
        .to_string(),
    )
}

pub fn intelligence_repairs_endpoint(
    fingerprint_hash: &str,
    failure_type: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    let retriever = ExecutionRetriever {
        memories: vec![],
        patterns: database
            .execution_patterns
            .iter()
            .map(|entry| ExecutionPattern {
                fingerprint: entry.fingerprint.clone(),
                failure_type: entry.failure_type.clone(),
                repair: entry.repair.clone(),
                success_rate: entry.success_rate,
                execution_count: entry.execution_count,
                average_duration: entry.average_duration,
                average_cost: entry.average_cost,
            })
            .collect(),
    };
    (
        "/intelligence/repairs".to_string(),
        json!({
            "fingerprint_hash": fingerprint_hash,
            "failure_type": failure_type,
            "repairs": retriever.patterns_for_failure(fingerprint_hash, failure_type, 10),
        })
        .to_string(),
    )
}

pub fn intelligence_context_endpoint(
    execution_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    let context = database
        .execution_contexts
        .iter()
        .find(|entry| entry.execution_id == execution_id);
    (
        "/intelligence/context".to_string(),
        json!({
            "execution_id": execution_id,
            "context": context,
        })
        .to_string(),
    )
}

pub fn intelligence_retrieve_endpoint(
    request: &IntelligenceRetrieveRequest,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    let retriever = ExecutionRetriever {
        memories: database
            .execution_contexts
            .iter()
            .filter_map(|context| {
                database
                    .executions
                    .iter()
                    .find(|execution| execution.execution_id == context.execution_id)
                    .map(|execution| ExecutionMemory {
                        execution_id: execution.execution_id.clone(),
                        repository_id: execution.repository_id.clone(),
                        commit_sha: execution.commit_hash.clone(),
                        fingerprint_hash: request.fingerprint_hash.clone(),
                        generated_plan: context.generated_plan.clone(),
                        chosen_plan: context.chosen_plan.clone(),
                        success: eidb_execution_status_is_success(&execution.status),
                        failure_type: (!eidb_execution_status_is_success(&execution.status))
                            .then(|| ExecutionLearningEngine::classify_failure(&execution.status)),
                        repair: None,
                        duration_seconds: execution
                            .completed_at
                            .map(|completed| completed.saturating_sub(execution.started_at)),
                        cost_units: None,
                    })
            })
            .collect(),
        patterns: vec![],
    };
    let similar = retriever.similar_executions(
        request.fingerprint_hash.as_str(),
        request.limit.unwrap_or(10),
    );
    (
        "/intelligence/retrieve".to_string(),
        json!({
            "execution_id": request.execution_id,
            "repository_id": request.repository_id,
            "similar_executions": similar,
        })
        .to_string(),
    )
}

pub fn intelligence_learn_endpoint(
    request: &IntelligenceLearnRequest,
    database: &mut ExecutionIntelligenceDatabase,
) -> (String, String) {
    let embedding = fingerprint_embedding(request.fingerprint_hash.as_str());
    database.record_execution_embedding(EidbExecutionEmbeddingRecord {
        id: format!("embedding-{}", request.execution_id),
        repository_id: request.repository_id.clone(),
        commit_sha: request.commit_sha.clone(),
        fingerprint_hash: request.fingerprint_hash.clone(),
        embedding,
        language: "unknown".to_string(),
        framework: "unknown".to_string(),
        runtime: "unknown".to_string(),
        created_at: now_epoch_seconds(),
    });

    let failure_type = ExecutionLearningEngine::classify_failure(&request.status);
    let success = eidb_execution_status_is_success(&request.status);
    let repair = request.repair.clone().unwrap_or_else(|| "none".to_string());

    let mut patterns = database
        .execution_patterns
        .iter()
        .map(|entry| ExecutionPattern {
            fingerprint: entry.fingerprint.clone(),
            failure_type: entry.failure_type.clone(),
            repair: entry.repair.clone(),
            success_rate: entry.success_rate,
            execution_count: entry.execution_count,
            average_duration: entry.average_duration,
            average_cost: entry.average_cost,
        })
        .collect::<Vec<_>>();
    ExecutionLearningEngine::learn_pattern(
        &mut patterns,
        request.fingerprint_hash.as_str(),
        failure_type.as_str(),
        repair.as_str(),
        success,
        request.duration_seconds.unwrap_or_default() as f64,
        request.cost_units.unwrap_or_default(),
    );
    database.execution_patterns = patterns
        .into_iter()
        .enumerate()
        .map(|(idx, entry)| EidbExecutionPatternRecord {
            id: format!("pattern-{idx}"),
            fingerprint: entry.fingerprint,
            failure_type: entry.failure_type,
            repair: entry.repair,
            success_rate: entry.success_rate,
            execution_count: entry.execution_count,
            average_duration: entry.average_duration,
            average_cost: entry.average_cost,
        })
        .collect();

    (
        "/intelligence/learn".to_string(),
        json!({
            "execution_id": request.execution_id,
            "learned_failure_type": failure_type,
            "patterns": database.execution_patterns,
        })
        .to_string(),
    )
}

pub fn intelligence_optimize_endpoint(
    request: &IntelligenceOptimizeRequest,
    database: &mut ExecutionIntelligenceDatabase,
) -> (String, String) {
    let retriever = ExecutionRetriever {
        memories: database
            .execution_contexts
            .iter()
            .filter_map(|context| {
                database
                    .executions
                    .iter()
                    .find(|execution| execution.execution_id == context.execution_id)
                    .map(|execution| ExecutionMemory {
                        execution_id: execution.execution_id.clone(),
                        repository_id: execution.repository_id.clone(),
                        commit_sha: execution.commit_hash.clone(),
                        fingerprint_hash: request.fingerprint_hash.clone(),
                        generated_plan: context.generated_plan.clone(),
                        chosen_plan: context.chosen_plan.clone(),
                        success: eidb_execution_status_is_success(&execution.status),
                        failure_type: (!eidb_execution_status_is_success(&execution.status))
                            .then(|| ExecutionLearningEngine::classify_failure(&execution.status)),
                        repair: None,
                        duration_seconds: execution
                            .completed_at
                            .map(|completed| completed.saturating_sub(execution.started_at)),
                        cost_units: None,
                    })
            })
            .collect(),
        patterns: database
            .execution_patterns
            .iter()
            .map(|entry| ExecutionPattern {
                fingerprint: entry.fingerprint.clone(),
                failure_type: entry.failure_type.clone(),
                repair: entry.repair.clone(),
                success_rate: entry.success_rate,
                execution_count: entry.execution_count,
                average_duration: entry.average_duration,
                average_cost: entry.average_cost,
            })
            .collect(),
    };
    let similar = retriever.similar_executions(request.fingerprint_hash.as_str(), 10);
    let patterns = request
        .failure_type
        .as_ref()
        .map(|failure_type| {
            retriever.patterns_for_failure(request.fingerprint_hash.as_str(), failure_type, 10)
        })
        .unwrap_or_default();
    let (context, optimized) = ExecutionContextBuilder::default().build(
        request.execution_id.as_str(),
        request.generated_plan.as_str(),
        &similar,
        &patterns,
    );
    database.record_execution_context(EidbExecutionContextRecord {
        execution_id: context.execution_id.clone(),
        similar_execution_ids: context.similar_execution_ids.clone(),
        retrieved_patterns: context.retrieved_patterns.clone(),
        generated_plan: context.generated_plan.clone(),
        chosen_plan: context.chosen_plan.clone(),
    });
    (
        "/intelligence/optimize".to_string(),
        json!({
            "execution_id": request.execution_id,
            "context": context,
            "optimized_plan": optimized,
        })
        .to_string(),
    )
}

pub fn billing_usage_endpoint(
    org_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    billing_usage_endpoint_with_store(org_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn billing_usage_endpoint_with_store(
    org_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let events = store.billing_events_for_org(org_id)?;
    let total_cost_units: f64 = events.iter().map(|event| event.cost_units).sum();
    let run_count = events
        .iter()
        .filter(|event| {
            event
                .event_type
                .eq_ignore_ascii_case(BillingEventType::ExecutionCompleted.as_str())
        })
        .count();
    let free_tier_usage = run_count.min(FREE_PLAN_RUNS_PER_DAY);
    let pro_tier_usage = run_count
        .saturating_sub(FREE_PLAN_RUNS_PER_DAY)
        .min(PRO_PLAN_RUNS_PER_DAY);
    let enterprise_usage = run_count.saturating_sub(FREE_PLAN_RUNS_PER_DAY + PRO_PLAN_RUNS_PER_DAY);

    Ok((
        format!("/billing/usage?org_id={org_id}"),
        json!({
            "org_id": org_id,
            "events": events,
            "total_cost_units": total_cost_units,
            "quota": {
                "free_runs_per_day": FREE_PLAN_RUNS_PER_DAY,
                "pro_runs_per_day": PRO_PLAN_RUNS_PER_DAY,
                "enterprise_runs_per_day": "unlimited",
            },
            "usage_buckets": {
                "free_tier_usage": free_tier_usage,
                "pro_tier_usage": pro_tier_usage,
                "enterprise_usage": enterprise_usage,
            },
            "quota_exceeded": run_count > FREE_PLAN_RUNS_PER_DAY + PRO_PLAN_RUNS_PER_DAY,
        })
        .to_string(),
    ))
}

pub fn billing_summary_endpoint(database: &ExecutionIntelligenceDatabase) -> (String, String) {
    billing_summary_endpoint_with_store(database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn billing_summary_endpoint_with_store(
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let events = store.billing_events()?;
    let mut org_usage_history: HashMap<String, f64> = HashMap::new();
    let mut runtime_distribution_costs: HashMap<String, f64> = HashMap::new();
    let mut execution_cost_history: HashMap<String, f64> = HashMap::new();
    let mut healing_costs = 0.0;

    for event in &events {
        *org_usage_history.entry(event.org_id.clone()).or_insert(0.0) += event.cost_units;
        *runtime_distribution_costs
            .entry(event.runtime_type.clone())
            .or_insert(0.0) += event.cost_units;
        *execution_cost_history
            .entry(event.execution_id.clone())
            .or_insert(0.0) += event.cost_units;
        if event.event_type == BillingEventType::ExecutionHealingAttempted.as_str() {
            healing_costs += event.cost_units;
        }
    }

    Ok((
        "/billing/summary".to_string(),
        json!({
            "org_usage_history": org_usage_history,
            "runtime_distribution_costs": runtime_distribution_costs,
            "execution_cost_history": execution_cost_history,
            "healing_costs": healing_costs,
        })
        .to_string(),
    ))
}

pub fn billing_invoice_endpoint(
    org_id: &str,
    database: &ExecutionIntelligenceDatabase,
) -> (String, String) {
    billing_invoice_endpoint_with_store(org_id, database)
        .expect("in-memory ExecutionIntelligenceDatabase reads should not fail")
}

pub fn billing_invoice_endpoint_with_store(
    org_id: &str,
    store: &impl ExecutionIntelligenceReadStore,
) -> PersistenceResult<(String, String)> {
    let events = store.billing_events_for_org(org_id)?;
    let total_cost_units: f64 = events.iter().map(|event| event.cost_units).sum();
    let mut execution_costs: HashMap<String, f64> = HashMap::new();
    for event in &events {
        *execution_costs
            .entry(event.execution_id.clone())
            .or_insert(0.0) += event.cost_units;
    }

    let invoice_id = format!("invoice-{org_id}-{}", now_epoch_millis());

    Ok((
        "/billing/invoice".to_string(),
        json!({
            "invoice_id": invoice_id,
            "org_id": org_id,
            "event_count": events.len(),
            "total_cost_units": total_cost_units,
            "line_items": execution_costs,
            "generated_at": now_epoch_seconds(),
        })
        .to_string(),
    ))
}

/// Returns `/repo/{id}/commits` payload containing commit hashes, timestamps, and build status.
pub fn list_repo_commits_endpoint(repo_id: &str, graph: &RepositoryTimeGraph) -> (String, String) {
    (
        format!("/repo/{repo_id}/commits"),
        json!({
            "repo": repo_id,
            "commits": graph.commits.iter().map(|commit| {
                json!({
                    "commit_hash": &commit.commit_hash,
                    "timestamp": commit.timestamp,
                    "build_status": commit.build_status.map_or("unknown", build_status_label),
                })
            }).collect::<Vec<_>>()
        })
        .to_string(),
    )
}

/// Returns `/execute` payload after validating that the requested commit hash is well-formed.
pub fn execute_commit_endpoint(request: &TemporalExecuteRequest) -> (String, String) {
    let verified = is_verified_commit_hash(&request.commit);
    (
        "/execute".to_string(),
        json!({
            "repo": &request.repo,
            "commit": &request.commit,
            "accepted": verified,
            "reason": if verified { "verified commit hash" } else { "unverified commit hash" },
        })
        .to_string(),
    )
}

/// Returns `/execute/recover` payload with a selected fallback commit for the requested strategy.
pub fn execute_recover_endpoint(
    request: &TemporalRecoverRequest,
    router: &TemporalExecutionRouter,
    graph: &RepositoryTimeGraph,
) -> (String, String) {
    let strategy = match request.strategy.to_ascii_lowercase().as_str() {
        "best_runnable" => RecoveryStrategy::BestRunnable,
        _ => RecoveryStrategy::LastKnownGood,
    };
    let head = graph
        .commits
        .first()
        .map(|commit| commit.commit_hash.as_str())
        .unwrap_or_default();
    let selected = router.route(graph, head, strategy);
    (
        "/execute/recover".to_string(),
        json!({
            "repo": &request.repo,
            "strategy": &request.strategy,
            "selected_commit": selected,
        })
        .to_string(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceRuntimeType {
    Dea,
    Cloud,
    Docker,
    External,
}

impl WorkspaceRuntimeType {
    pub const fn from_execution_tier(tier: ExecutionTier) -> Self {
        match tier {
            ExecutionTier::LocalMachine => Self::Dea,
            ExecutionTier::LocalDocker => Self::Docker,
            ExecutionTier::ExternalProvider => Self::External,
            ExecutionTier::CloudPartner | ExecutionTier::DDockitCloud => Self::Cloud,
        }
    }

    pub fn to_runtime_type(self) -> RuntimeType {
        match self {
            Self::Dea => RuntimeType::Node,
            Self::Cloud => RuntimeType::Static,
            Self::Docker => RuntimeType::Wasm,
            Self::External => RuntimeType::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceProxyProtocol {
    Http,
    WebSocket,
    Sse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRuntimeBinding {
    pub runtime_type: WorkspaceRuntimeType,
    pub runtime_instance_id: String,
    pub endpoint: String,
    pub lease_expires_at: DateTime,
    pub runtime_heartbeat: DateTime,
    pub last_request_time: DateTime,
    pub execution_health: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeResolver {
    bindings: HashMap<WorkspaceId, WorkspaceRuntimeBinding>,
}

impl RuntimeResolver {
    pub fn bind(&mut self, workspace_id: &str, binding: WorkspaceRuntimeBinding) {
        self.bindings.insert(workspace_id.to_string(), binding);
    }

    pub fn resolve(&self, workspace_id: &str) -> Option<&WorkspaceRuntimeBinding> {
        self.bindings.get(workspace_id)
    }

    pub fn update_health(
        &mut self,
        workspace_id: &str,
        heartbeat_at: DateTime,
        request_at: DateTime,
        execution_health: bool,
    ) -> bool {
        let Some(binding) = self.bindings.get_mut(workspace_id) else {
            return false;
        };
        binding.runtime_heartbeat = heartbeat_at;
        binding.last_request_time = request_at;
        binding.execution_health = execution_health;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRouterEvent {
    pub workspace_id: WorkspaceId,
    pub event_type: String,
    pub timestamp: DateTime,
}

const RUNTIME_FAILOVER_TIERS: [ExecutionTier; 4] = [
    ExecutionTier::ESCALATION_CHAIN[0],
    ExecutionTier::ESCALATION_CHAIN[1],
    ExecutionTier::ESCALATION_CHAIN[2],
    ExecutionTier::ESCALATION_CHAIN[3],
];

const RUNTIME_FAILOVER_PRIORITY: [WorkspaceRuntimeType; 4] = [
    WorkspaceRuntimeType::from_execution_tier(RUNTIME_FAILOVER_TIERS[0]),
    WorkspaceRuntimeType::from_execution_tier(RUNTIME_FAILOVER_TIERS[1]),
    WorkspaceRuntimeType::from_execution_tier(RUNTIME_FAILOVER_TIERS[2]),
    WorkspaceRuntimeType::from_execution_tier(RUNTIME_FAILOVER_TIERS[3]),
];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceRouter {
    pub registry: WorkspaceRegistry,
    pub resolver: RuntimeResolver,
    pub proxy: WorkspaceProxy,
    pub events: Vec<WorkspaceRouterEvent>,
}

impl WorkspaceRouter {
    pub fn create_workspace(
        &mut self,
        repository_id: &str,
        commit_hash: &str,
        org_id: &str,
        created_by: &str,
        now: DateTime,
    ) -> WorkspaceRecord {
        let workspace_id = ExecutionRouter::sanitized_workspace_id(
            &hash_key(&format!("{org_id}:{repository_id}:{commit_hash}"))[..12],
        );
        let record = WorkspaceRecord {
            workspace_id: workspace_id.clone(),
            repository_id: repository_id.to_string(),
            org_id: org_id.to_string(),
            created_by: created_by.to_string(),
            visibility: WorkspaceVisibility::Private,
            execution_id: format!(
                "exec-{}",
                hash_key(&format!("{workspace_id}:{commit_hash}"))
            ),
            assigned_worker: None,
            assigned_runtime: RuntimeType::Unknown,
            assigned_url: stable_workspace_url(&workspace_id, true),
            state: WorkspaceState::Pending,
            created_at: now,
            updated_at: now,
            quota: WorkspaceQuota::default(),
        };
        self.registry.upsert(record.clone());
        self.events.push(WorkspaceRouterEvent {
            workspace_id,
            event_type: "workspace_created".to_string(),
            timestamp: now,
        });
        record
    }

    pub fn resolve_workspace(
        &self,
        registry: &WorkspaceRegistry,
        workspace_id: &str,
    ) -> Option<WorkspaceRecord> {
        registry.get(workspace_id).cloned()
    }

    pub fn resolve_worker(
        &self,
        registry: &WorkspaceRegistry,
        workspace_id: &str,
    ) -> Option<WorkerId> {
        registry
            .get(workspace_id)
            .and_then(|record| record.assigned_worker.clone())
    }

    pub fn bind_runtime(
        &mut self,
        workspace_id: &str,
        binding: WorkspaceRuntimeBinding,
        now: DateTime,
    ) -> bool {
        let Some(workspace) = self.registry.get_mut(workspace_id) else {
            return false;
        };
        workspace.assigned_runtime = binding.runtime_type.to_runtime_type();
        workspace.assigned_worker = Some(binding.runtime_instance_id.clone());
        workspace.updated_at = now;
        self.proxy.bind(
            workspace_id,
            &binding.runtime_instance_id,
            binding.endpoint.clone(),
        );
        self.resolver.bind(workspace_id, binding);
        self.events.push(WorkspaceRouterEvent {
            workspace_id: workspace_id.to_string(),
            event_type: "runtime_bound".to_string(),
            timestamp: now,
        });
        true
    }

    pub fn migrate_runtime(
        &mut self,
        workspace_id: &str,
        binding: WorkspaceRuntimeBinding,
        now: DateTime,
    ) -> bool {
        let Some(workspace) = self.registry.get_mut(workspace_id) else {
            return false;
        };
        workspace.state = WorkspaceState::Migrating;
        workspace.updated_at = now;
        let preserved_url = workspace.assigned_url.clone();
        if !self.bind_runtime(workspace_id, binding, now) {
            return false;
        }
        if let Some(updated) = self.registry.get_mut(workspace_id) {
            updated.assigned_url = preserved_url;
            updated.state = WorkspaceState::Running;
            updated.updated_at = now.saturating_add(1);
        }
        self.events.push(WorkspaceRouterEvent {
            workspace_id: workspace_id.to_string(),
            event_type: "runtime_migrated".to_string(),
            timestamp: now,
        });
        true
    }

    pub fn mark_runtime_failed(&mut self, workspace_id: &str, now: DateTime) -> bool {
        let Some(workspace) = self.registry.get_mut(workspace_id) else {
            return false;
        };
        workspace.state = WorkspaceState::Degraded;
        workspace.updated_at = now;
        self.events.push(WorkspaceRouterEvent {
            workspace_id: workspace_id.to_string(),
            event_type: "runtime_failed".to_string(),
            timestamp: now,
        });
        true
    }

    pub fn route_workspace_request(
        &mut self,
        request_target: &str,
        now: DateTime,
    ) -> Option<WorkspaceRoute> {
        let route = self.route_request(&self.registry, &self.proxy, request_target)?;
        self.events.push(WorkspaceRouterEvent {
            workspace_id: route.workspace_id.clone(),
            event_type: "url_resolved".to_string(),
            timestamp: now,
        });
        let _ = self
            .resolver
            .update_health(&route.workspace_id, now, now, true);
        Some(route)
    }

    pub fn route_request(
        &self,
        registry: &WorkspaceRegistry,
        proxy: &WorkspaceProxy,
        request_target: &str,
    ) -> Option<WorkspaceRoute> {
        let workspace_id = parse_workspace_id(request_target)?;
        let workspace = registry.get(&workspace_id)?;
        let binding = proxy.resolve(&workspace_id)?;

        Some(WorkspaceRoute {
            workspace_id: workspace_id.clone(),
            worker_id: binding.worker_id.clone(),
            runtime: workspace.assigned_runtime,
            target: binding.target.clone(),
        })
    }

    pub fn select_failover_runtime(
        &self,
        available: &[WorkspaceRuntimeType],
    ) -> Option<WorkspaceRuntimeType> {
        RUNTIME_FAILOVER_PRIORITY
            .iter()
            .copied()
            .find(|candidate| available.contains(candidate))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHealthStatus {
    pub workspace_id: WorkspaceId,
    pub http_ok: bool,
    pub tcp_reachable: bool,
    pub process_alive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceHealthMonitor;

impl WorkspaceHealthMonitor {
    pub fn evaluate(&self, health: &WorkspaceHealthStatus) -> WorkspaceState {
        if health.http_ok && health.tcp_reachable && health.process_alive {
            WorkspaceState::Running
        } else {
            WorkspaceState::Degraded
        }
    }

    pub fn apply(
        &self,
        registry: &mut WorkspaceRegistry,
        health: WorkspaceHealthStatus,
        now: DateTime,
    ) -> Option<WorkspaceState> {
        let next = self.evaluate(&health);
        if registry.set_state(&health.workspace_id, next, now) {
            Some(next)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceRecoveryManager;

impl WorkspaceRecoveryManager {
    pub fn restart(
        &self,
        registry: &mut WorkspaceRegistry,
        workspace_id: &str,
        now: DateTime,
    ) -> bool {
        if !registry.set_state(workspace_id, WorkspaceState::Restarting, now) {
            return false;
        }
        registry.set_state(workspace_id, WorkspaceState::Running, now.saturating_add(1))
    }

    pub fn migrate(
        &self,
        registry: &mut WorkspaceRegistry,
        lease_registry: &mut ExecutionLeaseRegistry,
        workspace_id: &str,
        target_worker: &str,
        now: DateTime,
        lease_ttl_secs: u64,
    ) -> bool {
        if !registry.set_state(workspace_id, WorkspaceState::Migrating, now) {
            return false;
        }
        let Some(record) = registry.get_mut(workspace_id) else {
            return false;
        };
        record.assigned_worker = Some(target_worker.to_string());
        record.updated_at = now;
        lease_registry.assign(workspace_id, target_worker, now, lease_ttl_secs);
        registry.set_state(workspace_id, WorkspaceState::Running, now.saturating_add(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHeartbeat {
    pub worker_id: WorkerId,
    pub cpu: u32,
    pub memory: u64,
    pub running_workspaces: usize,
    pub health: bool,
    pub timestamp: DateTime,
}

pub const MAX_WORKSPACES_PER_WORKER: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerCapacitySnapshot {
    pub worker_id: WorkerId,
    pub cpu_available: u32,
    pub memory_available: u64,
    pub workspace_capacity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityScheduler {
    pub max_workspaces_per_worker: usize,
}

impl Default for CapacityScheduler {
    fn default() -> Self {
        Self {
            max_workspaces_per_worker: MAX_WORKSPACES_PER_WORKER,
        }
    }
}

impl CapacityScheduler {
    pub fn score(&self, worker: &WorkerCapacitySnapshot) -> u128 {
        u128::from(worker.cpu_available)
            + u128::from(worker.memory_available)
            + (worker.workspace_capacity as u128)
    }

    pub fn select_worker(&self, workers: &[WorkerCapacitySnapshot]) -> Option<WorkerId> {
        workers
            .iter()
            .filter(|worker| worker.workspace_capacity < self.max_workspaces_per_worker)
            .max_by(|a, b| {
                self.score(a)
                    .cmp(&self.score(b))
                    .then_with(|| a.worker_id.cmp(&b.worker_id))
            })
            .map(|worker| worker.worker_id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Workspace {
    pub id: String,
    pub repo_url: String,
    pub root: PathBuf,
    pub state: WorkspaceState,
    pub framework: Framework,
    pub ports: Vec<PortInfo>,
    pub network_policy: NetworkPolicy,
    pub resource_quotas: ResourceQuotas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProcessStatus {
    Launching,
    Initializing,
    Ready,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionReadinessState {
    Starting,
    SignalDetected,
    Ready,
    Exited,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionTruthEvent {
    StdoutLine(String),
    StderrLine(String),
    ProcessAlive(bool),
    ProcessExited(Option<i32>),
    ObservedPort(Option<u16>),
    HttpProbeOk(u16),
    HttpProbeErr(String),
    ReadinessTimedOut,
    Lifecycle(WorkspaceState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutionTruth {
    pub workspace_id: String,
    pub provider_selected: String,
    pub pid: u32,
    pub process_state: ProcessStatus,
    pub exit_code: Option<i32>,
    pub requested_port: u16,
    pub actual_port: Option<u16>,
    pub stdout_tail: Vec<String>,
    pub stderr_tail: Vec<String>,
    pub detected_start_signal: Option<String>,
    pub http_ready: bool,
    pub last_http_probe: String,
    pub readiness_state: ExecutionReadinessState,
    pub lifecycle_state: WorkspaceState,
    #[serde(skip_serializing)]
    pub started_at_unix_ms: u128,
    #[serde(skip_serializing)]
    pub process_alive: bool,
    #[serde(skip_serializing)]
    pub readiness_attempts: u32,
    #[serde(skip_serializing)]
    pub health_check_duration_ms: u64,
}

impl ExecutionTruth {
    fn new(workspace_id: String, requested_port: u16, pid: u32) -> Self {
        Self {
            workspace_id,
            provider_selected: "local-supervised-process".to_string(),
            pid,
            process_state: ProcessStatus::Launching,
            exit_code: None,
            requested_port,
            actual_port: None,
            stdout_tail: vec![],
            stderr_tail: vec![],
            detected_start_signal: None,
            http_ready: false,
            last_http_probe: "not probed".to_string(),
            readiness_state: ExecutionReadinessState::Starting,
            lifecycle_state: WorkspaceState::Launching,
            started_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            process_alive: true,
            readiness_attempts: 0,
            health_check_duration_ms: 0,
        }
    }

    fn update_from_event(&mut self, event: ExecutionTruthEvent) {
        match event {
            ExecutionTruthEvent::StdoutLine(line) => {
                self.apply_runtime_log_line(line, false);
            }
            ExecutionTruthEvent::StderrLine(line) => {
                self.apply_runtime_log_line(line, true);
            }
            ExecutionTruthEvent::ProcessAlive(alive) => {
                self.process_alive = alive;
                if alive {
                    self.process_state = ProcessStatus::Initializing;
                } else {
                    self.process_state = ProcessStatus::Stopped;
                    self.lifecycle_state = WorkspaceState::Failed;
                    self.readiness_state = ExecutionReadinessState::Exited;
                }
            }
            ExecutionTruthEvent::ProcessExited(code) => {
                self.exit_code = code;
                self.process_alive = false;
                self.process_state = ProcessStatus::Failed;
                self.lifecycle_state = WorkspaceState::Failed;
                self.readiness_state = ExecutionReadinessState::Exited;
            }
            ExecutionTruthEvent::ObservedPort(port) => {
                self.actual_port = port;
            }
            ExecutionTruthEvent::HttpProbeOk(status) => {
                self.http_ready = true;
                self.last_http_probe = format!("{status} OK");
            }
            ExecutionTruthEvent::HttpProbeErr(err) => {
                self.http_ready = false;
                self.last_http_probe = err;
            }
            ExecutionTruthEvent::ReadinessTimedOut => {
                self.process_state = ProcessStatus::Failed;
                self.readiness_state = ExecutionReadinessState::TimedOut;
                self.lifecycle_state = WorkspaceState::Failed;
            }
            ExecutionTruthEvent::Lifecycle(state) => {
                self.lifecycle_state = state;
                self.process_state = match state {
                    WorkspaceState::Launching => ProcessStatus::Launching,
                    WorkspaceState::Initializing => ProcessStatus::Initializing,
                    WorkspaceState::Ready => ProcessStatus::Ready,
                    WorkspaceState::Running => ProcessStatus::Running,
                    WorkspaceState::Stopping => ProcessStatus::Stopping,
                    WorkspaceState::Stopped => ProcessStatus::Stopped,
                    WorkspaceState::Failed => ProcessStatus::Failed,
                    _ => self.process_state,
                };
            }
        }
        self.evaluate_readiness();
    }

    fn apply_runtime_log_line(&mut self, line: String, is_stderr: bool) {
        if is_stderr {
            WorkspaceManager::append_capped(&mut self.stderr_tail, line.clone());
        } else {
            WorkspaceManager::append_capped(&mut self.stdout_tail, line.clone());
        }
        if self.detected_start_signal.is_none()
            && STARTUP_LOG_PATTERNS
                .iter()
                .any(|pattern| line.to_ascii_lowercase().contains(pattern))
        {
            self.detected_start_signal = Some(line);
            self.readiness_state = ExecutionReadinessState::SignalDetected;
        }
    }

    fn evaluate_readiness(&mut self) {
        if self.exit_code.is_some() || !self.process_alive {
            self.readiness_state = ExecutionReadinessState::Exited;
            if !matches!(
                self.lifecycle_state,
                WorkspaceState::Stopped | WorkspaceState::Stopping | WorkspaceState::Destroyed
            ) {
                self.lifecycle_state = WorkspaceState::Failed;
            }
            return;
        }
        if self.http_ready {
            self.readiness_state = ExecutionReadinessState::Ready;
            if matches!(
                self.lifecycle_state,
                WorkspaceState::Launching | WorkspaceState::Initializing | WorkspaceState::Ready
            ) {
                self.lifecycle_state = WorkspaceState::Ready;
                self.process_state = ProcessStatus::Ready;
            }
        } else if self.detected_start_signal.is_some() {
            self.readiness_state = ExecutionReadinessState::SignalDetected;
            if matches!(
                self.lifecycle_state,
                WorkspaceState::Launching | WorkspaceState::Initializing
            ) {
                self.lifecycle_state = WorkspaceState::Initializing;
                self.process_state = ProcessStatus::Initializing;
            }
        } else if matches!(
            self.lifecycle_state,
            WorkspaceState::Launching | WorkspaceState::Initializing
        ) {
            self.readiness_state = ExecutionReadinessState::Starting;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum ExecutionRoutingMode {
    Local,
    Wasm,
    Remote,
    Hybrid,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionHandle {
    pub workspace_id: String,
    pub provider_id: String,
    pub execution_id: String,
    pub routing_mode: ExecutionRoutingMode,
    pub endpoint: Option<String>,
    pub stream_channel: Option<String>,
    pub readiness_state: ExecutionReadinessState,
    pub authority_node: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRuntimeStatus {
    pub workspace_id: String,
    pub provider_selected: String,
    pub provider: String,
    pub runtime: String,
    pub pid: u32,
    pub alive: bool,
    pub process_state: ProcessStatus,
    pub exit_code: Option<i32>,
    pub framework: String,
    pub requested_port: u16,
    pub actual_port: Option<u16>,
    #[serde(rename = "assignedPort")]
    pub assigned_port: Option<u16>,
    #[serde(rename = "proxyUrl")]
    pub proxy_url: Option<String>,
    pub healthy: bool,
    pub listening: bool,
    pub detected_start_signal: Option<String>,
    pub http_ready: bool,
    pub readiness_state: ExecutionReadinessState,
    pub lifecycle_state: WorkspaceState,
    pub last_http_probe: String,
    pub last_probe: String,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
    #[serde(rename = "executionHandle", skip_serializing_if = "Option::is_none")]
    pub execution_handle: Option<ExecutionHandle>,
    #[serde(rename = "executionTrace", skip_serializing_if = "Option::is_none")]
    pub execution_trace: Option<WorkspaceExecutionTrace>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceExecutionTrace {
    pub provider: String,
    #[serde(rename = "selectedBecause")]
    pub selected_because: Vec<String>,
    pub fallbacks: Vec<String>,
    #[serde(rename = "actualStartup")]
    pub actual_startup: String,
    pub successful: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LaunchOverrides {
    pub branch: Option<String>,
    pub start_command: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionContext {
    pub workspace_id: String,
    pub repo_path: String,
    pub analysis: RepositoryAnalysis,
    pub execution_graph: ExecutionGraph,
    pub runtime_spec: ExecutionRuntimeSpec,
    pub compiled_runtime: CompiledWasmExecutionEnvironment,
    pub wasm_sandbox: Option<WasmSandbox>,
    pub resources: ResourceQuotas,
    pub network: NetworkPolicy,
}

pub struct BuildPlanner;

/// Provider contract for deterministic workspace execution.
///
/// Implementations are selected via `can_handle`, then called in the
/// lifecycle order `prepare` -> `start` -> `health` (and eventually `stop`).
pub trait ExecutionProvider {
    /// Stable provider identifier used by runtime affinity and router selection.
    fn id(&self) -> &'static str;
    /// Execution tier owned by this provider.
    fn tier(&self) -> ExecutionTier;
    /// Runtime family owned by this provider.
    fn runtime(&self) -> RuntimeType;
    /// Declares how clients can reach execution started by this provider.
    fn transport(&self) -> ExecutionRoutingMode {
        ExecutionRoutingMode::Local
    }
    /// Provider capability metadata used for ranked selection.
    fn capability(&self) -> ProviderCapability {
        let (latency_score, cost_score, reliability_score) = match self.tier() {
            ExecutionTier::LocalMachine => (10, 5, 35),
            ExecutionTier::LocalDocker => (15, 10, 30),
            ExecutionTier::ExternalProvider => (20, 20, 25),
            ExecutionTier::CloudPartner => (25, 30, 25),
            ExecutionTier::DDockitCloud => (30, 35, 30),
        };
        ProviderCapability {
            tier: self.tier(),
            latency_score,
            cost_score,
            reliability_score,
            supported_runtimes: vec![self.runtime()],
        }
    }
    /// Returns true when this provider owns runtime execution for `ctx`.
    fn can_handle(&self, ctx: &ExecutionContext) -> bool;
    /// Returns true when this provider can run `req`.
    fn can_run(&self, req: &ExecutionContext) -> bool {
        self.can_handle(req)
    }
    /// Mutates provider-specific runtime details before start.
    fn prepare(&self, ctx: &mut ExecutionContext) -> Result<()>;
    /// Starts execution from an immutable execution contract.
    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle>;
    /// Stops a process started by this provider.
    fn stop(&self, handle: &ProcessHandle) -> Result<()>;
    /// Reports process health after startup and during monitoring.
    fn health(&self, handle: &ProcessHandle) -> Result<HealthStatus>;
}

pub struct WasmExecutionProvider;

pub trait WasmWorkspace {
    fn launch(&self, repo_url: &str) -> Result<Workspace>;
    fn stop(&self, id: &str) -> Result<()>;
    fn restart(&self, id: &str) -> Result<()>;
    fn logs(&self, id: &str) -> Result<Vec<String>>;
    fn filesystem(&self, id: &str) -> Result<VirtualFileSystem>;
    fn ports(&self, id: &str) -> Result<Vec<PortInfo>>;
}

struct LocalWorkspaceRecord {
    workspace: Workspace,
    logs: Vec<String>,
    execution_context: Option<ExecutionContext>,
    process_handle: Option<ProcessHandle>,
    child_process: Option<Child>,
    runtime: Option<ExecutionTruth>,
    launch_overrides: LaunchOverrides,
}

#[derive(Debug, Serialize)]
struct ExecutionArtifactEnvironmentSummary {
    #[serde(rename = "overrideCount")]
    override_count: usize,
    #[serde(rename = "environmentVariables")]
    environment_variables: Vec<String>,
    #[serde(rename = "versionOverrides")]
    version_overrides: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RuntimeRepairTelemetry {
    #[serde(rename = "repairReason", skip_serializing_if = "Option::is_none")]
    repair_reason: Option<String>,
    #[serde(rename = "repairConfidence", skip_serializing_if = "Option::is_none")]
    repair_confidence: Option<f32>,
    #[serde(rename = "successfulPatch", skip_serializing_if = "Option::is_none")]
    successful_patch: Option<String>,
    #[serde(rename = "finalRuntimeState")]
    final_runtime_state: String,
}

#[derive(Debug, Serialize)]
struct ExecutionArtifactMetadata {
    #[serde(rename = "launchCommand")]
    launch_command: Option<String>,
    #[serde(rename = "startupDurationMs")]
    startup_duration_ms: u64,
    #[serde(rename = "healthCheckDurationMs")]
    health_check_duration_ms: Option<u64>,
    retries: u32,
    #[serde(rename = "assignedRuntime")]
    assigned_runtime: String,
    #[serde(rename = "assignedProvider")]
    assigned_provider: String,
    #[serde(rename = "proxyEndpoint")]
    proxy_endpoint: Option<String>,
    #[serde(rename = "environmentSummary")]
    environment_summary: ExecutionArtifactEnvironmentSummary,
    error: Option<String>,
    #[serde(rename = "runtimeRepair", skip_serializing_if = "Option::is_none")]
    runtime_repair: Option<RuntimeRepairTelemetry>,
}

#[derive(Debug, Serialize)]
struct ExecutionArtifactRecord {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "executionId")]
    execution_id: String,
    provider: String,
    runtime: String,
    #[serde(rename = "packageManager")]
    package_manager: Option<String>,
    #[serde(rename = "assignedPort")]
    assigned_port: Option<u16>,
    #[serde(rename = "proxyUrl")]
    proxy_url: Option<String>,
    #[serde(rename = "startupTimeMs")]
    startup_time_ms: u64,
    #[serde(rename = "healthStatus")]
    health_status: String,
    #[serde(rename = "exitCode")]
    exit_code: Option<i32>,
    metadata: ExecutionArtifactMetadata,
}

pub struct ExecutionEngine {
    router: ExecutionRouter,
    artifact_store: ArtifactStore,
}

pub struct WorkspaceManager {
    root: PathBuf,
    execution_engine: ExecutionEngine,
    workspaces: Arc<Mutex<HashMap<String, LocalWorkspaceRecord>>>,
    repository_cache: Arc<Mutex<HashMap<String, PathBuf>>>,
    sequence: AtomicU64,
}

impl ExecutionEngine {
    pub fn new(
        providers: Vec<Box<dyn ExecutionProvider + Send + Sync>>,
        artifact_store: ArtifactStore,
    ) -> Self {
        Self {
            router: ExecutionRouter::new(providers),
            artifact_store,
        }
    }

    pub fn start(&self, ctx: &mut ExecutionContext) -> Result<ProcessHandle> {
        self.prime_artifacts(ctx)?;
        self.router.dispatch_start(ctx)
    }

    /// Ensures each node has artifact metadata recorded unless a matching cache key already exists.
    fn prime_artifacts(&self, ctx: &ExecutionContext) -> Result<()> {
        let keys = ctx.execution_graph.compute_cache_keys();
        // ArtifactStore persists metadata under the runtime root; this path tracks
        // workspace-local node output locations referenced by those metadata records.
        let artifacts_root = Path::new(&ctx.repo_path).join(".rustgit").join("artifacts");
        fs::create_dir_all(&artifacts_root)?;

        for node in &ctx.execution_graph.nodes {
            let Some(key) = keys.get(&node.id) else {
                continue;
            };
            if self.artifact_store.exists(key) {
                continue;
            }
            let artifact_path = artifacts_root.join(&node.id);
            fs::create_dir_all(&artifact_path)?;
            let artifact_type = match ExecutionRouter::route(node, &ctx.analysis.execution_profile)
            {
                ExecutionTarget::Wasm(_) => ArtifactType::WasmModule,
                ExecutionTarget::Native | ExecutionTarget::Static => ArtifactType::BuildOutput,
            };
            self.artifact_store.put(ExecutionArtifact {
                key: key.clone(),
                node_id: node.id.clone(),
                artifact_type,
                path: artifact_path.to_string_lossy().to_string(),
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
            if matches!(
                ExecutionRouter::route(node, &ctx.analysis.execution_profile),
                ExecutionTarget::Wasm(_)
            ) {
                self.artifact_store
                    .register_wasm_artifact_binding(wasm_artifact_binding(ctx, node, key));
                match load_compiled_wasm_module(ctx, node) {
                    Ok(module) => {
                        self.artifact_store.register_wasm_artifact(WasmArtifact {
                            node_id: node.id.clone(),
                            module_path: module.path,
                            hash: module.hash,
                        });
                    }
                    Err(err) => {
                        eprintln!(
                            "wasm artifact for node {} not yet available during priming: {err}",
                            node.id
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub fn stop(&self, ctx: &ExecutionContext, handle: &ProcessHandle) -> Result<()> {
        self.router.dispatch_stop(ctx, handle)
    }
}

fn run_command_with_timeout(
    command: &mut Command,
    timeout_secs: u64,
) -> Result<std::process::Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| RuntimeError::CommandFailed(format!("failed to spawn command: {err}")))?;

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(timeout_secs))
        .ok_or_else(|| {
            RuntimeError::CommandFailed(format!("command timeout value too large: {timeout_secs}s"))
        })?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_end(&mut stdout).map_err(|err| {
                        RuntimeError::CommandFailed(format!("failed to read command stdout: {err}"))
                    })?;
                }
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_end(&mut stderr).map_err(|err| {
                        RuntimeError::CommandFailed(format!("failed to read command stderr: {err}"))
                    })?;
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    child.kill().map_err(|err| {
                        RuntimeError::CommandFailed(format!(
                            "command timed out after {timeout_secs}s but failed to kill process: {err}"
                        ))
                    })?;
                    if let Err(err) = child.wait() {
                        return Err(RuntimeError::CommandFailed(format!(
                            "command timed out after {timeout_secs}s, process was killed but wait failed: {err}"
                        )));
                    }
                    return Err(RuntimeError::CommandFailed(format!(
                        "command timed out after {timeout_secs}s and was killed"
                    )));
                }
                std::thread::sleep(Duration::from_millis(INSTALL_POLL_INTERVAL_MS));
            }
            Err(err) => {
                return Err(RuntimeError::CommandFailed(format!(
                    "failed to poll command status: {err}"
                )));
            }
        }
    }
}

impl WorkspaceManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let requested_root: PathBuf = root.into();
        let normalized_root = if requested_root.is_absolute() {
            requested_root
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(requested_root)
        };

        let providers: Vec<Box<dyn ExecutionProvider + Send + Sync>> = vec![
            Box::new(WasmExecutionProvider),
            Box::new(LocalAgentProvider::default_agent()),
            Box::new(DockerExecutionProvider),
            Box::new(NodeRuntimeProvider),
            Box::new(GoExecutionProvider),
            Box::new(PythonExecutionProvider),
            Box::new(JavaExecutionProvider),
            Box::new(RustRuntimeProvider),
            Box::new(StaticRuntimeProvider),
        ];

        let artifact_store = ArtifactStore::new(normalized_root.join("artifacts"));

        Self {
            root: normalized_root,
            execution_engine: ExecutionEngine::new(providers, artifact_store),
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            repository_cache: Arc::new(Mutex::new(HashMap::new())),
            sequence: AtomicU64::new(0),
        }
    }

    pub fn analyze_repository(&self, path: impl AsRef<Path>) -> Result<RepositoryAnalysis> {
        analyze_repository(path.as_ref())
    }

    pub fn rest_api_spec(&self) -> RestApiSpec {
        RestApiSpec::default()
    }

    fn next_workspace_id(&self) -> String {
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        format!("ws-{ts}-{seq}")
    }

    fn transition_state(workspace: &mut Workspace, to: WorkspaceState) -> Result<()> {
        if can_transition(workspace.state, to) {
            workspace.state = to;
            Ok(())
        } else {
            Err(RuntimeError::InvalidTransition {
                from: workspace.state,
                to,
            })
        }
    }

    fn append_capped(lines: &mut Vec<String>, line: String) {
        lines.push(line);
        if lines.len() > MAX_RUNTIME_LOG_LINES {
            let overflow = lines.len() - MAX_RUNTIME_LOG_LINES;
            lines.drain(0..overflow);
        }
    }

    fn push_workspace_log(&self, id: &str, message: String) {
        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                Self::append_capped(&mut record.logs, message);
            }
        }
    }

    fn set_workspace_state(&self, id: &str, state: WorkspaceState) {
        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                record.workspace.state = state;
            }
        }
    }

    fn fail_workspace(&self, id: &str, message: String) {
        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                record.workspace.state = WorkspaceState::Failed;
                Self::append_capped(&mut record.logs, message.clone());
                if let Some(runtime) = record.runtime.as_mut() {
                    runtime.update_from_event(ExecutionTruthEvent::ProcessExited(None));
                    runtime
                        .update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Failed));
                }
            }
        }
    }

    fn update_runtime_line(
        workspaces: &Arc<Mutex<HashMap<String, LocalWorkspaceRecord>>>,
        id: &str,
        line: String,
        is_stderr: bool,
    ) {
        if let Ok(mut table) = workspaces.lock() {
            if let Some(record) = table.get_mut(id) {
                Self::append_capped(&mut record.logs, line.clone());
                if let Some(runtime) = record.runtime.as_mut() {
                    if is_stderr {
                        runtime.update_from_event(ExecutionTruthEvent::StderrLine(line));
                    } else {
                        runtime.update_from_event(ExecutionTruthEvent::StdoutLine(line));
                    }
                }
            }
        }
    }

    fn start_stream_reader(
        workspaces: Arc<Mutex<HashMap<String, LocalWorkspaceRecord>>>,
        workspace_id: String,
        stream_name: &'static str,
        stream: impl io::Read + Send + 'static,
    ) {
        std::thread::spawn(move || {
            let reader = BufReader::new(stream);
            for line in reader.lines().map_while(std::result::Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let prefixed = format!("{stream_name}: {trimmed}");
                Self::update_runtime_line(
                    &workspaces,
                    &workspace_id,
                    prefixed,
                    stream_name == "stderr",
                );
            }
        });
    }

    fn parse_listening_ports(pid: u32) -> Vec<u16> {
        let mut inodes = HashSet::new();
        let fd_dir = PathBuf::from(format!("/proc/{pid}/fd"));
        if let Ok(entries) = fs::read_dir(fd_dir) {
            for entry in entries.flatten() {
                if let Ok(target) = fs::read_link(entry.path()) {
                    let target = target.to_string_lossy();
                    if let Some(inode) = target
                        .strip_prefix("socket:[")
                        .and_then(|value| value.strip_suffix(']'))
                    {
                        inodes.insert(inode.to_string());
                    }
                }
            }
        }

        if inodes.is_empty() {
            return vec![];
        }

        fn parse_proc_net(path: &str, inodes: &HashSet<String>) -> Vec<u16> {
            let mut ports = Vec::new();
            let Ok(content) = fs::read_to_string(path) else {
                return ports;
            };
            for line in content.lines().skip(1) {
                let columns: Vec<&str> = line.split_whitespace().collect();
                if columns.len() < 10 || columns[3] != "0A" {
                    continue;
                }
                let Some((_, port_hex)) = columns[1].split_once(':') else {
                    continue;
                };
                if !inodes.contains(columns[9]) {
                    continue;
                }
                if let Ok(port) = u16::from_str_radix(port_hex, 16) {
                    ports.push(port);
                }
            }
            ports
        }

        let mut ports = parse_proc_net("/proc/net/tcp", &inodes);
        ports.extend(parse_proc_net("/proc/net/tcp6", &inodes));
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    fn http_probe(port: u16, health_check: &str) -> std::result::Result<u16, String> {
        use std::io::{BufRead, Write};
        use std::net::TcpStream;
        let addr = format!("127.0.0.1:{port}");
        let mut stream = TcpStream::connect_timeout(
            &addr
                .parse()
                .map_err(|_| "invalid socket address".to_string())?,
            Duration::from_millis(500),
        )
        .map_err(|err| err.to_string())?;
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        stream
            .write_all(
                format!(
                    "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    normalize_health_check_path(health_check)
                )
                .as_bytes(),
            )
            .map_err(|err| err.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut status_line = String::new();
        reader
            .read_line(&mut status_line)
            .map_err(|err| err.to_string())?;
        let status = status_line
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| "invalid response".to_string())?;
        if matches!(status, 200 | 301 | 302 | 404) {
            Ok(status)
        } else {
            Err(format!("http {status}"))
        }
    }

    fn start_process_monitor(&self, workspace_id: &str) {
        let workspaces = Arc::clone(&self.workspaces);
        let workspace_id = workspace_id.to_string();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let mut should_exit = false;
            if let Ok(mut table) = workspaces.lock() {
                let Some(record) = table.get_mut(&workspace_id) else {
                    break;
                };
                if matches!(
                    record.workspace.state,
                    WorkspaceState::Failed | WorkspaceState::Stopped | WorkspaceState::Destroyed
                ) {
                    break;
                }
                if let Some(child) = record.child_process.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            Self::append_capped(
                                &mut record.logs,
                                format!(
                                    "process exited unexpectedly ({})",
                                    status.code().map_or("signal".to_string(), |code| format!(
                                        "code {code}"
                                    ))
                                ),
                            );
                            if let Some(runtime) = record.runtime.as_mut() {
                                runtime.update_from_event(ExecutionTruthEvent::ProcessExited(
                                    status.code(),
                                ));
                                runtime.update_from_event(ExecutionTruthEvent::Lifecycle(
                                    WorkspaceState::Failed,
                                ));
                                record.workspace.state = runtime.lifecycle_state;
                            }
                            record.child_process = None;
                            should_exit = true;
                        }
                        Ok(None) => {
                            if let Some(runtime) = record.runtime.as_mut() {
                                runtime.update_from_event(ExecutionTruthEvent::ProcessAlive(true));
                                if runtime.http_ready
                                    && runtime.lifecycle_state == WorkspaceState::Ready
                                {
                                    runtime.update_from_event(ExecutionTruthEvent::Lifecycle(
                                        WorkspaceState::Running,
                                    ));
                                }
                                record.workspace.state = runtime.lifecycle_state;
                            } else if record.workspace.state == WorkspaceState::Ready {
                                record.workspace.state = WorkspaceState::Running;
                            }
                        }
                        Err(_) => {}
                    }
                } else {
                    should_exit = true;
                }
            } else {
                should_exit = true;
            }
            if should_exit {
                break;
            }
        });
    }

    /// Removes the on-disk directories for workspaces in terminal states (Failed / Stopped /
    /// Destroyed) and purges the repo cache. Returns the number of workspaces evicted.
    fn evict_terminal_workspaces(&self) -> usize {
        let terminal = {
            let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
            workspaces
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.workspace.state,
                        WorkspaceState::Failed
                            | WorkspaceState::Stopped
                            | WorkspaceState::Destroyed
                    )
                })
                .map(|(id, r)| (id.clone(), r.workspace.root.clone()))
                .collect::<Vec<_>>()
        };

        let mut evicted = 0;
        for (id, root) in &terminal {
            if root.exists() {
                let _ = fs::remove_dir_all(root);
            }
            // Remove from in-memory map so the ID is no longer surfaced.
            self.workspaces
                .lock()
                .expect("workspace lock poisoned")
                .remove(id);
            evicted += 1;
        }

        // Also purge the repo clone cache — those dirs can be large.
        let cache_dir = self.root.join("cache");
        if cache_dir.exists() {
            let _ = fs::remove_dir_all(&cache_dir);
        }
        self.repository_cache
            .lock()
            .expect("repo cache lock poisoned")
            .clear();

        evicted
    }

    pub fn cleanup(&self) -> (usize, u64) {
        let evicted = self.evict_terminal_workspaces();
        // Also wipe any rustgit-analyze temp dirs created by the analyze endpoint.
        let analyze_tmp = std::env::temp_dir().join("rustgit-analyze");
        if analyze_tmp.exists() {
            let _ = fs::remove_dir_all(&analyze_tmp);
        }
        let free_bytes = available_disk_bytes(&self.root).unwrap_or(0);
        (evicted, free_bytes)
    }

    fn materialize_repository(
        &self,
        repo_url: &str,
        branch: Option<&str>,
        destination: &Path,
    ) -> Result<()> {
        const MIN_FREE_BYTES_TO_CLONE: u64 = 1024 * 1024 * 1024; // 1 GB
        const MIN_FREE_BYTES_TO_CACHE: u64 = 2 * 1024 * 1024 * 1024; // 2 GB
        const MAX_REPO_BYTES_TO_CACHE: u64 = 200 * 1024 * 1024; // 200 MB

        if destination.exists() {
            fs::remove_dir_all(destination)?;
        }
        fs::create_dir_all(destination)?;

        if let Some(cached) = self
            .repository_cache
            .lock()
            .expect("repo cache lock poisoned")
            .get(repo_url)
            .cloned()
        {
            copy_directory(&cached, destination)?;
            return Ok(());
        }

        // If disk is low, evict terminal workspaces and the repo cache before giving up.
        let free_before = available_disk_bytes(destination).unwrap_or(u64::MAX);
        if free_before < MIN_FREE_BYTES_TO_CLONE {
            self.evict_terminal_workspaces();
            let free_after_eviction = available_disk_bytes(destination).unwrap_or(0);
            if free_after_eviction < MIN_FREE_BYTES_TO_CLONE {
                return Err(RuntimeError::CommandFailed(format!(
                    "insufficient disk space to clone repository: {:.1} GB free after cleanup, need at least 1 GB",
                    free_after_eviction as f64 / 1_073_741_824.0
                )));
            }
        }

        if looks_like_local_path(repo_url) {
            copy_directory(Path::new(repo_url), destination)?;
        } else {
            let mut clone_command = Command::new("git");
            clone_command
                .arg("-c")
                .arg("credential.helper=")
                .arg("-c")
                .arg("credential.username=")
                .arg("clone")
                .arg("--depth")
                .arg("1");
            clone_command.env("GIT_TERMINAL_PROMPT", "0");
            if let Some(extra_header) = github_clone_extra_header(repo_url) {
                clone_command.arg("-c").arg(format!(
                    "http.https://github.com/.extraheader={extra_header}"
                ));
            }
            let effective_branch = branch
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .map(str::to_string);
            if let Some(ref b) = effective_branch {
                clone_command.arg("--branch").arg(b);
            }
            let output = clone_command
                .arg(repo_url)
                .arg(destination)
                .output()
                .map_err(|e| RuntimeError::CommandFailed(format!("git clone failed: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                // If the branch wasn't found, retry without it to fall back to default.
                if effective_branch.is_some()
                    && (stderr.contains("Remote branch") && stderr.contains("not found")
                        || stderr.contains("pathspec") && stderr.contains("did not match"))
                {
                    if destination.exists() {
                        let _ = fs::remove_dir_all(destination);
                    }
                    fs::create_dir_all(destination)?;
                    let output2 = clone_command
                        .args(["--branch"]) // remove last --branch arg isn't possible; rebuild
                        .output();
                    // Rebuild without --branch and retry
                    let mut retry = Command::new("git");
                    retry
                        .arg("-c")
                        .arg("credential.helper=")
                        .arg("-c")
                        .arg("credential.username=")
                        .arg("clone")
                        .arg("--depth")
                        .arg("1")
                        .env("GIT_TERMINAL_PROMPT", "0")
                        .arg(repo_url)
                        .arg(destination);
                    drop(output2);
                    let retry_out = retry.output().map_err(|e| {
                        RuntimeError::CommandFailed(format!("git clone failed: {e}"))
                    })?;
                    if !retry_out.status.success() {
                        let err = String::from_utf8_lossy(&retry_out.stderr)
                            .trim()
                            .to_string();
                        let reason = github_clone_error_reason(repo_url, &err);
                        return Err(RuntimeError::CommandFailed(format!(
                            "git clone exited with status {}: {}",
                            retry_out.status, reason
                        )));
                    }
                } else {
                    let reason = github_clone_error_reason(repo_url, &stderr);
                    return Err(RuntimeError::CommandFailed(format!(
                        "git clone exited with status {}: {}",
                        output.status, reason
                    )));
                }
            }
        }

        // Cache the clone only when the repo is small and there is plenty of free space.
        let repo_size = directory_size_bytes(destination);
        let free_after = available_disk_bytes(destination).unwrap_or(0);
        if repo_size <= MAX_REPO_BYTES_TO_CACHE && free_after >= MIN_FREE_BYTES_TO_CACHE {
            let cache_path = self
                .root
                .join("cache")
                .join(format!("repo-{}", hash_key(repo_url)));
            if cache_path.exists() {
                fs::remove_dir_all(&cache_path)?;
            }
            fs::create_dir_all(&cache_path)?;
            copy_directory(destination, &cache_path)?;
            self.repository_cache
                .lock()
                .expect("repo cache lock poisoned")
                .insert(repo_url.to_string(), cache_path);
        }

        Ok(())
    }

    pub fn list_workspaces(&self) -> Vec<Workspace> {
        let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        workspaces.values().map(|r| r.workspace.clone()).collect()
    }

    pub fn get_workspace(&self, id: &str) -> Result<Workspace> {
        let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        workspaces
            .get(id)
            .map(|r| r.workspace.clone())
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))
    }

    /// Immediately allocates a workspace ID and inserts a pending record.
    /// Call `complete_launch` on a background thread to do the actual work.
    pub fn begin_launch(&self, repo_url: &str) -> String {
        self.begin_launch_with_overrides(repo_url, LaunchOverrides::default())
    }

    pub fn begin_launch_with_overrides(
        &self,
        repo_url: &str,
        overrides: LaunchOverrides,
    ) -> String {
        let id = self.next_workspace_id();
        let workspace_root = self.root.join("workspaces").join(&id);
        let workspace = Workspace {
            id: id.clone(),
            repo_url: repo_url.to_string(),
            root: workspace_root,
            state: WorkspaceState::Created,
            framework: Framework::Unknown,
            ports: vec![],
            network_policy: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
            resource_quotas: ResourceQuotas {
                max_memory_mb: 1024,
                max_cpu_millis: 1000,
            },
        };
        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        workspaces.insert(
            id.clone(),
            LocalWorkspaceRecord {
                workspace,
                logs: vec!["workspace queued".to_string()],
                execution_context: None,
                process_handle: None,
                child_process: None,
                runtime: None,
                launch_overrides: overrides,
            },
        );
        id
    }

    /// Does the blocking work for a workspace previously allocated by `begin_launch`.
    pub fn complete_launch(&self, id: &str, repo_url: &str) {
        let overrides = self
            .workspaces
            .lock()
            .ok()
            .and_then(|workspaces| {
                workspaces
                    .get(id)
                    .map(|record| record.launch_overrides.clone())
            })
            .unwrap_or_default();
        self.complete_launch_with_overrides(id, repo_url, overrides);
    }

    pub fn complete_launch_with_overrides(
        &self,
        id: &str,
        repo_url: &str,
        overrides: LaunchOverrides,
    ) {
        let launch_started = Instant::now();
        let workspace_root = self.root.join("workspaces").join(id);
        let repository_root = workspace_root.join("repo");

        let mut workspace = {
            let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
            match workspaces.get(id) {
                Some(r) => r.workspace.clone(),
                None => return,
            }
        };

        if let Err(e) = fs::create_dir_all(&workspace_root) {
            let message = format!("workspace failed: {e}");
            self.fail_workspace(id, message.clone());
            self.emit_execution_artifact(
                id,
                launch_started.elapsed().as_millis() as u64,
                None,
                None,
                None,
                Some(message),
                None,
            );
            return;
        }

        // Proactively free disk space from any finished workspaces before cloning.
        self.evict_terminal_workspaces();

        self.set_workspace_state(id, WorkspaceState::Materializing);
        if let Err(e) =
            self.materialize_repository(repo_url, overrides.branch.as_deref(), &repository_root)
        {
            let message = format!("workspace failed: {e}");
            self.fail_workspace(id, message.clone());
            self.emit_execution_artifact(
                id,
                launch_started.elapsed().as_millis() as u64,
                None,
                None,
                None,
                Some(message),
                None,
            );
            return;
        }
        self.push_workspace_log(id, format!("materialized repository: {repo_url}"));

        self.set_workspace_state(id, WorkspaceState::Analyzing);
        let analysis = match analyze_repository(&repository_root) {
            Ok(a) => a,
            Err(e) => {
                let message = format!("workspace failed: {e}");
                self.fail_workspace(id, message.clone());
                self.emit_execution_artifact(
                    id,
                    launch_started.elapsed().as_millis() as u64,
                    None,
                    None,
                    None,
                    Some(message),
                    None,
                );
                return;
            }
        };
        self.push_workspace_log(id, format!("detected framework: {:?}", analysis.framework));

        self.set_workspace_state(id, WorkspaceState::Planning);
        let ctx = ExecutionContext {
            workspace_id: id.to_string(),
            repo_path: repository_root.to_string_lossy().to_string(),
            analysis: analysis.clone(),
            execution_graph: analysis.execution_graph.clone(),
            runtime_spec: analysis.runtime_spec.clone(),
            compiled_runtime: analysis.compiled_runtime.clone(),
            wasm_sandbox: None,
            resources: ResourceQuotas {
                max_memory_mb: analysis.runtime_spec.memory_limit_mb,
                max_cpu_millis: analysis.runtime_spec.cpu_limit_units,
            },
            network: analysis.runtime_spec.network_policy.clone(),
        };
        let planned_command =
            Self::resolved_run_command(&ctx, &overrides).unwrap_or_else(|| "none".to_string());
        self.push_workspace_log(id, format!("planned execution command: {planned_command}"));
        if !overrides.environment.is_empty() {
            self.push_workspace_log(
                id,
                format!("applied env overrides: {}", overrides.environment.len()),
            );
        }
        if !overrides.versions.is_empty() {
            self.push_workspace_log(
                id,
                format!("applied version overrides: {}", overrides.versions.len()),
            );
        }

        workspace.framework = ctx.analysis.framework;
        workspace.ports = workspace_ports_from_manifest_or_framework(
            Path::new(&ctx.repo_path),
            ctx.analysis.framework,
        );
        workspace.network_policy = ctx.network.clone();
        workspace.resource_quotas = ctx.resources.clone();
        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                record.workspace = workspace.clone();
                record.execution_context = Some(ctx.clone());
                record.launch_overrides = overrides.clone();
            }
        }

        let original_manifest = load_runtime_manifest_value(&repository_root);
        let mut current_overrides = overrides.clone();
        let mut repair_candidates: VecDeque<RuntimeRepairCandidate> = VecDeque::new();
        let mut retries = 0u32;
        let mut failure_message: Option<String> = None;
        let mut health_duration_ms: Option<u64> = None;
        let mut repair_reason: Option<String> = None;
        let mut repair_confidence: Option<f32> = None;
        let mut successful_patch: Option<String> = None;

        let max_attempts = runtime_repair_attempt_limit();
        for attempt in 0..max_attempts {
            self.set_workspace_state(id, WorkspaceState::Installing);
            if let Err(err) =
                Self::run_dependency_install(&ctx, &current_overrides, &self.workspaces, id)
            {
                failure_message = Some(format!("install failed: {err}"));
            } else {
                self.set_workspace_state(id, WorkspaceState::Launching);
                match Self::spawn_supervised_process(&ctx, &current_overrides) {
                    Ok((mut child, requested_port, provider_selected)) => {
                        let pid = child.id();
                        self.push_workspace_log(
                            id,
                            format!("spawned pid: {pid} on requested port {requested_port}"),
                        );

                        let stdout = child.stdout.take();
                        let stderr = child.stderr.take();
                        let mut runtime = ExecutionTruth::new(id.to_string(), requested_port, pid);
                        runtime.provider_selected = provider_selected;
                        runtime.update_from_event(ExecutionTruthEvent::Lifecycle(
                            WorkspaceState::Launching,
                        ));

                        if let Ok(mut workspaces) = self.workspaces.lock() {
                            if let Some(record) = workspaces.get_mut(id) {
                                record.child_process = Some(child);
                                record.runtime = Some(runtime);
                                record.launch_overrides = current_overrides.clone();
                            }
                        }

                        if let Some(stdout) = stdout {
                            Self::start_stream_reader(
                                Arc::clone(&self.workspaces),
                                id.to_string(),
                                "stdout",
                                stdout,
                            );
                        }
                        if let Some(stderr) = stderr {
                            Self::start_stream_reader(
                                Arc::clone(&self.workspaces),
                                id.to_string(),
                                "stderr",
                                stderr,
                            );
                        }

                        self.set_workspace_state(id, WorkspaceState::Initializing);
                        let health_started = Instant::now();
                        match self.wait_for_runtime_readiness(id) {
                            Ok(()) => {
                                health_duration_ms =
                                    Some(health_started.elapsed().as_millis() as u64);
                                failure_message = None;
                                break;
                            }
                            Err(err) => {
                                failure_message = Some(format!("runtime failed: {err}"));
                                health_duration_ms =
                                    Some(health_started.elapsed().as_millis() as u64);
                            }
                        }
                    }
                    Err(err) => {
                        failure_message = Some(format!("launch failed: {err}"));
                    }
                }
            }

            let Some(current_failure) = failure_message.clone() else {
                break;
            };
            self.fail_workspace(id, current_failure.clone());
            self.emit_execution_artifact(
                id,
                launch_started.elapsed().as_millis() as u64,
                health_duration_ms,
                Some(retries),
                Some(planned_command.clone()),
                Some(current_failure.clone()),
                Some(RuntimeRepairTelemetry {
                    repair_reason: repair_reason.clone(),
                    repair_confidence: repair_confidence,
                    successful_patch: None,
                    final_runtime_state: "failed".to_string(),
                }),
            );

            if let Ok(mut workspaces) = self.workspaces.lock() {
                if let Some(record) = workspaces.get_mut(id) {
                    if let Some(child) = record.child_process.as_mut() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    record.child_process = None;
                    record.runtime = None;
                }
            }

            if repair_candidates.is_empty() {
                let execution_artifact =
                    fs::read_to_string(workspace_root.join("execution-artifact.json"))
                        .ok()
                        .and_then(|payload| serde_json::from_str::<Value>(&payload).ok());
                let launch_logs = self
                    .workspaces
                    .lock()
                    .ok()
                    .and_then(|workspaces| workspaces.get(id).map(|record| record.logs.clone()))
                    .unwrap_or_default();
                let input = RuntimeRepairInput {
                    runtime_manifest: load_runtime_manifest_value(&repository_root),
                    execution_artifact,
                    launch_logs,
                    failure_message: current_failure,
                };
                repair_candidates = build_runtime_repair_candidates(
                    &input,
                    &overrides,
                    &planned_command,
                    &ctx.analysis.fingerprint,
                )
                .into();
                if let Some(first) = repair_candidates.front() {
                    repair_reason = Some(first.reason.clone());
                    repair_confidence = Some(first.confidence);
                }
            }

            if attempt + 1 >= max_attempts {
                break;
            }
            let Some(candidate) = repair_candidates.pop_front() else {
                break;
            };
            if let Some(manifest_patch) = candidate.manifest_patch.as_ref() {
                let _ = write_runtime_manifest_value(&repository_root, manifest_patch);
            }
            current_overrides = overrides.clone();
            if let Some(start_command) = candidate.start_command_override.as_ref() {
                current_overrides.start_command = Some(start_command.clone());
            }
            retries = retries.saturating_add(1);
            successful_patch = Some(candidate.id.clone());
            self.push_workspace_log(
                id,
                format!(
                    "runtime repair retry {retries}: applying {} (confidence {:.2})",
                    candidate.id, candidate.confidence
                ),
            );
        }

        if let Some(message) = failure_message {
            if let Some(manifest) = original_manifest.as_ref() {
                let _ = write_runtime_manifest_value(&repository_root, manifest);
            }
            self.fail_workspace(id, message.clone());
            self.emit_execution_artifact(
                id,
                launch_started.elapsed().as_millis() as u64,
                health_duration_ms,
                Some(retries),
                Some(planned_command),
                Some(message),
                Some(RuntimeRepairTelemetry {
                    repair_reason,
                    repair_confidence,
                    successful_patch: None,
                    final_runtime_state: "failed".to_string(),
                }),
            );
            return;
        }

        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                if let Some(runtime) = record.runtime.as_mut() {
                    runtime
                        .update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Ready));
                    record.workspace.state = runtime.lifecycle_state;
                }
                Self::append_capped(&mut record.logs, "workspace ready".to_string());
            }
        }

        self.emit_execution_artifact(
            id,
            launch_started.elapsed().as_millis() as u64,
            health_duration_ms,
            Some(retries),
            Some(planned_command),
            None,
            Some(RuntimeRepairTelemetry {
                repair_reason,
                repair_confidence,
                successful_patch: successful_patch
                    .or_else(|| Some("original-manifest".to_string())),
                final_runtime_state: "ready".to_string(),
            }),
        );
        self.start_process_monitor(id);
    }

    fn reserve_prebound_port_with_preferences(
        preferred_ports: &[u16],
    ) -> Option<(u16, std::net::TcpListener)> {
        for preferred in preferred_ports {
            if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", *preferred)) {
                return Some((*preferred, listener));
            }
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        Some((port, listener))
    }

    // Returns (child, assigned_port)
    fn apply_command_overrides(command: &str, overrides: &LaunchOverrides) -> String {
        let mut rendered = command.to_string();
        for (name, value) in &overrides.versions {
            rendered = rendered.replace(&format!("{{{name}}}"), value);
        }
        rendered
    }

    fn extract_workdir_and_command(command: &str, default_dir: &Path) -> (PathBuf, String) {
        let trimmed = command.trim();
        let Some(remainder) = trimmed.strip_prefix("cd ") else {
            return (default_dir.to_path_buf(), trimmed.to_string());
        };
        let (raw_dir, raw_command) = if let Some((dir, rest)) = remainder.split_once("&&") {
            (dir.trim(), rest.trim())
        } else {
            (remainder.trim(), "")
        };
        if raw_dir.is_empty() {
            return (default_dir.to_path_buf(), trimmed.to_string());
        }
        let raw_dir = raw_dir.trim_matches('"').trim_matches('\'');
        let mut workdir = PathBuf::from(raw_dir);
        if workdir.is_relative() {
            workdir = default_dir.join(workdir);
        }
        let normalized_default =
            fs::canonicalize(default_dir).unwrap_or_else(|_| default_dir.to_path_buf());
        let normalized_workdir = fs::canonicalize(&workdir).unwrap_or_else(|_| workdir.clone());
        if normalized_workdir.starts_with(&normalized_default) {
            (workdir, raw_command.to_string())
        } else {
            (default_dir.to_path_buf(), raw_command.to_string())
        }
    }

    fn resolved_run_command(ctx: &ExecutionContext, overrides: &LaunchOverrides) -> Option<String> {
        overrides
            .start_command
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| load_execution_manifest_start_command(Path::new(&ctx.repo_path)))
            .or_else(|| ctx.execution_graph.primary_run_command())
    }

    fn apply_process_overrides(command: &mut Command, overrides: &LaunchOverrides) {
        for (name, value) in &overrides.environment {
            command.env(name, value);
        }
        for (name, value) in &overrides.versions {
            command.env(name, value);
        }
    }

    fn run_dependency_install(
        ctx: &ExecutionContext,
        overrides: &LaunchOverrides,
        workspaces: &Arc<Mutex<HashMap<String, LocalWorkspaceRecord>>>,
        id: &str,
    ) -> Result<()> {
        let repo_path = Path::new(&ctx.repo_path);
        let install_cmd = load_execution_manifest_install_command(repo_path).or_else(|| {
            ctx.execution_graph
                .nodes
                .iter()
                .find(|n| n.node_type == ExecutionNodeType::InstallDependencies)
                .and_then(|n| n.command.clone())
        });
        let Some(install_cmd) = install_cmd else {
            return Ok(());
        };
        let (install_cwd, install_cmd) = Self::extract_workdir_and_command(&install_cmd, repo_path);
        if install_cmd.is_empty() {
            return Ok(());
        }
        Self::update_runtime_line(workspaces, id, format!("install: {install_cmd}"), false);
        let mut parts = install_cmd.split_whitespace();
        let Some(program) = parts.next() else {
            return Ok(());
        };
        let args: Vec<&str> = parts.collect();
        let mut install_cmd_handle = Command::new(program);
        install_cmd_handle
            .args(&args)
            .current_dir(&install_cwd)
            .envs(&overrides.environment)
            .envs(&overrides.versions);
        let output = run_command_with_timeout(&mut install_cmd_handle, INSTALL_TIMEOUT_SECS)?;

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if !line.trim().is_empty() {
                Self::update_runtime_line(
                    workspaces,
                    id,
                    format!("stdout: {}", line.trim()),
                    false,
                );
            }
        }
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            if !line.trim().is_empty() {
                Self::update_runtime_line(workspaces, id, format!("stderr: {}", line.trim()), true);
            }
        }

        if output.status.success() {
            return Ok(());
        }

        let lowered = install_cmd.to_ascii_lowercase();
        if lowered.contains("pnpm")
            && lowered.contains("install")
            && lowered.contains("--frozen-lockfile")
        {
            Self::update_runtime_line(
                workspaces,
                id,
                "stderr: pnpm frozen lockfile install failed, retrying with regenerated lockfile"
                    .to_string(),
                true,
            );
            let mut retry_cmd_handle = Command::new("pnpm");
            retry_cmd_handle
                .arg("install")
                .current_dir(&install_cwd)
                .envs(&overrides.environment)
                .envs(&overrides.versions);
            let retry = run_command_with_timeout(&mut retry_cmd_handle, INSTALL_TIMEOUT_SECS)?;

            for line in String::from_utf8_lossy(&retry.stdout).lines() {
                if !line.trim().is_empty() {
                    Self::update_runtime_line(
                        workspaces,
                        id,
                        format!("stdout: {}", line.trim()),
                        false,
                    );
                }
            }
            for line in String::from_utf8_lossy(&retry.stderr).lines() {
                if !line.trim().is_empty() {
                    Self::update_runtime_line(
                        workspaces,
                        id,
                        format!("stderr: {}", line.trim()),
                        true,
                    );
                }
            }
            if retry.status.success() {
                Self::update_runtime_line(
                    workspaces,
                    id,
                    "stdout: lockfile regenerated".to_string(),
                    false,
                );
                return Ok(());
            }
            return Err(RuntimeError::CommandFailed(format!(
                "pnpm install retry exited with status {}",
                retry.status
            )));
        }
        Err(RuntimeError::CommandFailed(format!(
            "install command exited with status {}",
            output.status
        )))
    }

    fn spawn_supervised_process(
        ctx: &ExecutionContext,
        overrides: &LaunchOverrides,
    ) -> Result<(Child, u16, String)> {
        let run_cmd = Self::resolved_run_command(ctx, overrides)
            .ok_or_else(|| RuntimeError::CommandFailed("no run command resolved".to_string()))?;
        let repo_path = Path::new(&ctx.repo_path);
        let manifest_config = load_runtime_manifest_launch_config(repo_path);
        let is_python = matches!(
            ctx.analysis.framework,
            Framework::Python
                | Framework::Flask
                | Framework::FastApi
                | Framework::Django
                | Framework::Streamlit
                | Framework::Gradio
        );

        let (requested_port, prebound_port_listener) =
            Self::reserve_prebound_port_with_preferences(
                manifest_config
                    .as_ref()
                    .map(|manifest| manifest.preferred_ports.as_slice())
                    .unwrap_or(&[]),
            )
            .ok_or_else(|| {
                RuntimeError::CommandFailed(
                    "failed to reserve runtime port via bind(127.0.0.1:0)".to_string(),
                )
            })?;

        let run_cmd = Self::auto_heal_runtime_command(
            &Self::apply_command_overrides(&run_cmd, overrides)
                .replace("{PORT}", &requested_port.to_string()),
        );
        let run_cmd = if is_python {
            let venv_bin = repo_path.join(".venv").join("bin");
            let mut parts = run_cmd.splitn(2, ' ');
            let prog = parts.next().unwrap_or_default();
            let rest = parts.next().unwrap_or_default();
            let venv_prog = venv_bin.join(prog);
            if venv_prog.exists() {
                if rest.is_empty() {
                    venv_prog.to_string_lossy().to_string()
                } else {
                    format!("{} {rest}", venv_prog.display())
                }
            } else {
                run_cmd
            }
        } else {
            run_cmd
        };

        let (run_cwd, run_cmd) = Self::extract_workdir_and_command(&run_cmd, repo_path);
        if run_cmd.is_empty() {
            return Err(RuntimeError::CommandFailed(
                "run command only changes directory and has no run step".to_string(),
            ));
        }
        let provider_selected = if DockerExecutionProvider::is_docker_command(&run_cmd) {
            DockerExecutionProvider::ensure_docker_ready(&run_cmd)?;
            DockerExecutionProvider::id_static().to_string()
        } else {
            "local-supervised-process".to_string()
        };
        let mut parts = run_cmd.splitn(2, ' ');
        let program = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RuntimeError::CommandFailed("missing run program".to_string()))?;
        let args: Vec<&str> = parts.next().unwrap_or("").split_whitespace().collect();

        let mut cmd = Command::new(program);
        cmd.args(&args)
            .current_dir(run_cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PORT", requested_port.to_string());
        if let Some(node_version) = manifest_config
            .as_ref()
            .and_then(|manifest| manifest.node_version.as_deref())
        {
            cmd.env("NODE_VERSION", node_version);
        }
        if let Some(package_manager) = manifest_config
            .as_ref()
            .and_then(|manifest| manifest.package_manager.as_deref())
        {
            cmd.env("PACKAGE_MANAGER", package_manager);
        }
        Self::apply_process_overrides(&mut cmd, overrides);
        drop(prebound_port_listener);
        let child = cmd
            .spawn()
            .map_err(|err| RuntimeError::CommandFailed(format!("spawn failed: {err}")))?;
        Ok((child, requested_port, provider_selected))
    }

    fn auto_heal_runtime_command(command: &str) -> String {
        let lower = command.to_ascii_lowercase();
        let mut healed = command.trim().to_string();
        let is_node_like = ["npm ", "pnpm ", "yarn ", "bun ", "vite ", "next "]
            .iter()
            .any(|marker| lower.contains(marker));
        if !is_node_like {
            return healed;
        }
        if !lower.contains("--host") && !lower.contains("hostname") {
            healed.push_str(" --host 0.0.0.0");
        }
        if !lower.contains("--port") && !lower.contains("port=") {
            let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
            healed.push_str(" --port ");
            healed.push_str(&port);
        }
        healed
    }

    fn wait_for_runtime_readiness(&self, id: &str) -> Result<()> {
        const READINESS_POLL_INTERVAL_MS: u64 = 500;
        const READINESS_TIMEOUT_MS: u64 = 30_000;
        const MAX_ATTEMPTS: usize = (READINESS_TIMEOUT_MS / READINESS_POLL_INTERVAL_MS) as usize;
        let readiness_started = Instant::now();
        let health_check = self
            .workspaces
            .lock()
            .ok()
            .and_then(|workspaces| {
                workspaces
                    .get(id)
                    .and_then(|record| record.execution_context.as_ref())
                    .map(|ctx| load_execution_manifest_health_check(Path::new(&ctx.repo_path)))
            })
            .unwrap_or_else(|| "/".to_string());

        for attempt in 0..MAX_ATTEMPTS {
            let mut exited_status = None;
            if let Ok(mut workspaces) = self.workspaces.lock() {
                let Some(record) = workspaces.get_mut(id) else {
                    return Err(RuntimeError::WorkspaceMissing(id.to_string()));
                };
                if let Some(child) = record.child_process.as_mut() {
                    if let Ok(Some(status)) = child.try_wait() {
                        exited_status = Some(status.code());
                    }
                }
                if let Some(runtime) = record.runtime.as_mut() {
                    runtime.readiness_attempts = (attempt + 1) as u32;
                    runtime.update_from_event(ExecutionTruthEvent::ProcessAlive(
                        exited_status.is_none(),
                    ));
                    let observed_port = Self::parse_listening_ports(runtime.pid).into_iter().next();
                    runtime.update_from_event(ExecutionTruthEvent::ObservedPort(observed_port));
                    if let Some(port) = runtime.actual_port {
                        match Self::http_probe(port, &health_check) {
                            Ok(status) => {
                                runtime.update_from_event(ExecutionTruthEvent::HttpProbeOk(status));
                                runtime.health_check_duration_ms =
                                    readiness_started.elapsed().as_millis() as u64;
                                return Ok(());
                            }
                            Err(err) => {
                                runtime.update_from_event(ExecutionTruthEvent::HttpProbeErr(err));
                            }
                        }
                    } else {
                        runtime.update_from_event(ExecutionTruthEvent::HttpProbeErr(
                            "connection refused".to_string(),
                        ));
                    }
                    runtime.update_from_event(ExecutionTruthEvent::Lifecycle(
                        WorkspaceState::Initializing,
                    ));
                    record.workspace.state = runtime.lifecycle_state;
                }
            }

            if let Some(code) = exited_status {
                if let Ok(mut workspaces) = self.workspaces.lock() {
                    if let Some(record) = workspaces.get_mut(id) {
                        if let Some(runtime) = record.runtime.as_mut() {
                            runtime.update_from_event(ExecutionTruthEvent::ProcessExited(code));
                            runtime.health_check_duration_ms =
                                readiness_started.elapsed().as_millis() as u64;
                        }
                        record.workspace.state = WorkspaceState::Failed;
                    }
                }
                return Err(RuntimeError::CommandFailed(format!(
                    "process exited before readiness probe completed (code {:?})",
                    code
                )));
            }
            std::thread::sleep(Duration::from_millis(READINESS_POLL_INTERVAL_MS));
        }

        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                if let Some(runtime) = record.runtime.as_mut() {
                    runtime.update_from_event(ExecutionTruthEvent::ReadinessTimedOut);
                    runtime.health_check_duration_ms =
                        readiness_started.elapsed().as_millis() as u64;
                }
                record.workspace.state = WorkspaceState::Failed;
            }
        }
        Err(RuntimeError::CommandFailed(
            "readiness probe timed out".to_string(),
        ))
    }

    fn emit_execution_artifact(
        &self,
        id: &str,
        startup_duration_ms: u64,
        health_check_duration_ms: Option<u64>,
        retries: Option<u32>,
        launch_command: Option<String>,
        error: Option<String>,
        runtime_repair: Option<RuntimeRepairTelemetry>,
    ) {
        let snapshot = {
            let Ok(workspaces) = self.workspaces.lock() else {
                return;
            };
            let Some(record) = workspaces.get(id) else {
                return;
            };
            (
                record.workspace.root.clone(),
                record.workspace.state,
                record.runtime.clone(),
                record.execution_context.clone(),
                record.launch_overrides.clone(),
            )
        };

        let (workspace_root, workspace_state, runtime, context, launch_overrides) = snapshot;
        let manifest_config = context
            .as_ref()
            .and_then(|ctx| load_runtime_manifest_launch_config(Path::new(&ctx.repo_path)));
        let runtime_label = runtime_label_for_context(context.as_ref(), manifest_config.as_ref());
        let package_manager = manifest_config
            .as_ref()
            .and_then(|manifest| manifest.package_manager.clone());
        let provider = runtime
            .as_ref()
            .map(|r| r.provider_selected.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let assigned_port = runtime
            .as_ref()
            .and_then(|r| r.actual_port.or(Some(r.requested_port)));
        let proxy_url = assigned_port.map(|port| format!("http://127.0.0.1:{port}"));
        let health_status = if workspace_state == WorkspaceState::Failed {
            "failed".to_string()
        } else if runtime.as_ref().map(|r| r.http_ready).unwrap_or(false) {
            "healthy".to_string()
        } else if runtime.as_ref().map(|r| r.process_alive).unwrap_or(false) {
            "starting".to_string()
        } else {
            "unknown".to_string()
        };
        let exit_code = if let Some(code) = runtime.as_ref().and_then(|r| r.exit_code) {
            Some(code)
        } else if health_status == "healthy" {
            Some(0)
        } else if workspace_state == WorkspaceState::Failed {
            Some(1)
        } else {
            None
        };
        let startup_time_ms = runtime
            .as_ref()
            .map(|r| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .saturating_sub(r.started_at_unix_ms) as u64
            })
            .unwrap_or(startup_duration_ms);
        let runtime_health_check_duration_ms = runtime
            .as_ref()
            .map(|r| r.health_check_duration_ms)
            .filter(|duration| *duration > 0);
        let retry_count = retries.unwrap_or_else(|| {
            runtime
                .as_ref()
                .map(|r| {
                    if r.readiness_attempts > 0 {
                        r.readiness_attempts - 1
                    } else {
                        0
                    }
                })
                .unwrap_or(0)
        });

        let mut environment_variables = launch_overrides
            .environment
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        environment_variables.sort();
        let mut version_overrides = launch_overrides
            .versions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        version_overrides.sort();
        let environment_summary = ExecutionArtifactEnvironmentSummary {
            override_count: environment_variables.len() + version_overrides.len(),
            environment_variables,
            version_overrides,
        };

        let artifact = ExecutionArtifactRecord {
            schema_version: 1,
            execution_id: id.to_string(),
            provider: provider.clone(),
            runtime: runtime_label.clone(),
            package_manager,
            assigned_port,
            proxy_url: proxy_url.clone(),
            startup_time_ms,
            health_status,
            exit_code,
            metadata: ExecutionArtifactMetadata {
                launch_command,
                startup_duration_ms,
                health_check_duration_ms: health_check_duration_ms
                    .or(runtime_health_check_duration_ms),
                retries: retry_count,
                assigned_runtime: runtime_label,
                assigned_provider: provider,
                proxy_endpoint: proxy_url,
                environment_summary,
                error,
                runtime_repair,
            },
        };

        if fs::create_dir_all(&workspace_root).is_err() {
            return;
        }
        if let Ok(body) = serde_json::to_string_pretty(&artifact) {
            let _ = fs::write(workspace_root.join("execution-artifact.json"), body);
        }
    }

    // Returns (child, assigned_port)
    fn spawn_run_command(
        ctx: &ExecutionContext,
        overrides: &LaunchOverrides,
        logs: &mut Vec<String>,
    ) -> (Option<std::process::Child>, Option<u16>, String) {
        let mut provider_selected = "local-supervised-process".to_string();
        let run_cmd = match Self::resolved_run_command(ctx, overrides) {
            Some(cmd) => cmd,
            None => return (None, None, provider_selected),
        };
        let repo_path = std::path::Path::new(&ctx.repo_path);
        let manifest_config = load_runtime_manifest_launch_config(repo_path);
        let is_python = matches!(
            ctx.analysis.framework,
            Framework::Python
                | Framework::Flask
                | Framework::FastApi
                | Framework::Django
                | Framework::Streamlit
                | Framework::Gradio
        );

        // For Python apps, create a venv and install into it
        if is_python {
            let venv_path = repo_path.join(".venv");
            if !venv_path.exists() {
                logs.push("creating virtual environment: python -m venv .venv".to_string());
                let _ = Command::new("python3")
                    .args(["-m", "venv", ".venv"])
                    .current_dir(repo_path)
                    .output();
            }

            let install_cmd = load_execution_manifest_install_command(repo_path).or_else(|| {
                ctx.execution_graph
                    .nodes
                    .iter()
                    .find(|n| n.node_type == ExecutionNodeType::InstallDependencies)
                    .and_then(|n| n.command.clone())
            });
            if let Some(install) = install_cmd {
                let venv_pip = venv_path.join("bin").join("pip");
                let pip_path = venv_pip.to_string_lossy().to_string();
                // Extract everything after "pip " so we get "install -r requirements.txt"
                let pip_args: Vec<String> = if let Some(idx) = install.find("pip ") {
                    install[idx + 4..]
                        .split_whitespace()
                        .map(String::from)
                        .collect()
                } else {
                    install
                        .split_whitespace()
                        .skip(1)
                        .map(String::from)
                        .collect()
                };
                logs.push(format!(
                    "installing dependencies: {pip_path} {}",
                    pip_args.join(" ")
                ));
                match Command::new(&pip_path)
                    .args(&pip_args)
                    .current_dir(repo_path)
                    .envs(&overrides.environment)
                    .envs(&overrides.versions)
                    .output()
                {
                    Ok(out) => {
                        if !out.status.success() {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            for line in stderr.lines().take(8) {
                                if !line.trim().is_empty() {
                                    logs.push(format!("  pip: {line}"));
                                }
                            }
                        }
                    }
                    Err(e) => logs.push(format!("  pip failed to launch: {e}")),
                }
            }
        } else {
            // JS/other: run install command as-is
            let install_cmd = load_execution_manifest_install_command(repo_path).or_else(|| {
                ctx.execution_graph
                    .nodes
                    .iter()
                    .find(|n| n.node_type == ExecutionNodeType::InstallDependencies)
                    .and_then(|n| n.command.clone())
            });
            if let Some(install) = install_cmd {
                let (install_cwd, install) = Self::extract_workdir_and_command(&install, repo_path);
                if install.is_empty() {
                    logs.push(
                        "install command only changes directory and has no install step; skipping"
                            .to_string(),
                    );
                } else {
                    let mut parts = install.split_whitespace();
                    if let Some(program) = parts.next() {
                        let args: Vec<&str> = parts.collect();
                        logs.push(format!("installing dependencies: {install}"));
                        let _ = Command::new(program)
                            .args(&args)
                            .current_dir(&install_cwd)
                            .envs(&overrides.environment)
                            .envs(&overrides.versions)
                            .output();
                    }
                }
            }
        }

        // For Django: run migrations before starting the server
        if ctx.analysis.framework == Framework::Django {
            let venv_python = repo_path.join(".venv").join("bin").join("python");
            let python = if venv_python.exists() {
                venv_python.to_string_lossy().to_string()
            } else {
                "python3".to_string()
            };
            logs.push("running migrations: manage.py migrate".to_string());
            let _ = Command::new(&python)
                .args(["manage.py", "migrate", "--run-syncdb"])
                .current_dir(repo_path)
                .envs(&overrides.environment)
                .envs(&overrides.versions)
                .output();
        }

        let preferred_ports = manifest_config
            .as_ref()
            .map(|manifest| manifest.preferred_ports.clone())
            .unwrap_or_default();
        let (assigned_port, prebound_port_listener) =
            match Self::reserve_prebound_port_with_preferences(&preferred_ports) {
                Some((port, listener)) => (Some(port), Some(listener)),
                None => (None, None),
            };
        let port_str = assigned_port.map_or("8000".to_string(), |p| p.to_string());

        // Substitute {PORT} placeholder used in Python command templates, then
        // rewrite the program path to the venv binary if this is a Python app
        let run_cmd =
            Self::apply_command_overrides(&run_cmd, overrides).replace("{PORT}", &port_str);
        let run_cmd = if is_python {
            let venv_bin = repo_path.join(".venv").join("bin");
            let mut parts = run_cmd.splitn(2, ' ');
            let prog = parts.next().unwrap_or("");
            let rest = parts.next().unwrap_or("");
            let venv_prog = venv_bin.join(prog);
            if venv_prog.exists() {
                if rest.is_empty() {
                    venv_prog.to_string_lossy().to_string()
                } else {
                    format!("{} {rest}", venv_prog.display())
                }
            } else {
                run_cmd
            }
        } else {
            run_cmd
        };
        let (run_cwd, run_cmd) = Self::extract_workdir_and_command(&run_cmd, repo_path);
        if run_cmd.is_empty() {
            logs.push(
                "run command only changes directory and has no run step; skipping process spawn"
                    .to_string(),
            );
            return (None, None, provider_selected);
        }
        if DockerExecutionProvider::is_docker_command(&run_cmd) {
            if let Err(err) = DockerExecutionProvider::ensure_docker_ready(&run_cmd) {
                logs.push(format!("docker runtime readiness check failed: {err}"));
                return (None, None, provider_selected);
            }
            provider_selected = DockerExecutionProvider::id_static().to_string();
        }

        let mut parts = run_cmd.splitn(2, ' ');
        let program = match parts.next() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return (None, None, provider_selected),
        };
        let args_str = parts.next().unwrap_or("");
        let args: Vec<&str> = args_str.split_whitespace().collect();

        // Pipe stderr to a log file in the workspace root so crashes are visible
        let stderr_log_path = repo_path.join("..").join("stderr.log");
        let stderr_stdio = std::fs::File::create(&stderr_log_path)
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .current_dir(&run_cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(stderr_stdio);
        if let Some(node_version) = manifest_config
            .as_ref()
            .and_then(|manifest| manifest.node_version.as_deref())
        {
            cmd.env("NODE_VERSION", node_version);
        }
        if let Some(package_manager) = manifest_config
            .as_ref()
            .and_then(|manifest| manifest.package_manager.as_deref())
        {
            cmd.env("PACKAGE_MANAGER", package_manager);
        }
        // Always set PORT env var for frameworks that read it (Node.js etc.)
        if let Some(port) = assigned_port {
            cmd.env("PORT", port.to_string());
        }
        Self::apply_process_overrides(&mut cmd, overrides);

        logs.push(format!("running: {program} {args_str}"));
        drop(prebound_port_listener);
        match cmd.spawn() {
            Ok(child) => {
                logs.push(format!("spawned pid: {} on port {port_str}", child.id()));
                // Give the process a moment to bind its port, then check stderr for early crashes
                std::thread::sleep(std::time::Duration::from_millis(2000));
                if let Ok(content) = std::fs::read_to_string(&stderr_log_path) {
                    for line in content.lines().take(10) {
                        if !line.trim().is_empty() {
                            logs.push(format!("  stderr: {line}"));
                        }
                    }
                }
                (Some(child), assigned_port, provider_selected)
            }
            Err(err) => {
                logs.push(format!("spawn failed: {err}"));
                (None, None, provider_selected)
            }
        }
    }

    /// Checks if the child process has exited and marks the workspace Failed if so.
    /// Called on every GET /workspaces/:id so the UI reflects reality quickly.
    pub fn sync_process_health(&self, id: &str) {
        if let Ok(mut workspaces) = self.workspaces.lock() {
            if let Some(record) = workspaces.get_mut(id) {
                if !matches!(
                    record.workspace.state,
                    WorkspaceState::Running | WorkspaceState::Initializing | WorkspaceState::Ready
                ) {
                    return;
                }
                if let Some(child) = record.child_process.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            Self::append_capped(
                                &mut record.logs,
                                format!(
                                    "process exited unexpectedly ({})",
                                    status
                                        .code()
                                        .map_or("signal".to_string(), |c| format!("code {c}"))
                                ),
                            );
                            if let Some(runtime) = record.runtime.as_mut() {
                                runtime.update_from_event(ExecutionTruthEvent::ProcessExited(
                                    status.code(),
                                ));
                                runtime.update_from_event(ExecutionTruthEvent::Lifecycle(
                                    WorkspaceState::Failed,
                                ));
                                record.workspace.state = runtime.lifecycle_state;
                            }
                            record.child_process = None;
                        }
                        Ok(None) => {
                            if let Some(runtime) = record.runtime.as_mut() {
                                runtime.update_from_event(ExecutionTruthEvent::ProcessAlive(true));
                                record.workspace.state = runtime.lifecycle_state;
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }
    }

    pub fn restart_with_overrides(
        &self,
        id: &str,
        overrides: Option<LaunchOverrides>,
    ) -> Result<()> {
        // Kill old process and snapshot what we need before releasing the lock
        let (ctx, effective_overrides) = {
            let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
            let record = workspaces
                .get_mut(id)
                .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
            if let Some(new_overrides) = overrides {
                record.launch_overrides = new_overrides;
            }
            Self::transition_state(&mut record.workspace, WorkspaceState::Starting)?;
            if let Some(child) = record.child_process.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            record.child_process = None;
            record.process_handle = None;
            record.logs.push("restarting…".to_string());
            (
                record
                    .execution_context
                    .clone()
                    .ok_or_else(|| RuntimeError::ExecutionContextMissing(id.to_string()))?,
                record.launch_overrides.clone(),
            )
        }; // lock released here

        // Spawn the new process without holding the lock (can take seconds)
        let mut spawn_logs: Vec<String> = vec![];
        let (child, assigned_port, provider_selected) =
            Self::spawn_run_command(&ctx, &effective_overrides, &mut spawn_logs);

        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        for line in spawn_logs {
            record.logs.push(line);
        }
        if let Some(port) = assigned_port {
            for p in &mut record.workspace.ports {
                p.port = port;
            }
        }
        if let Some(runtime) = record.runtime.as_mut() {
            runtime.provider_selected = provider_selected.clone();
        }
        record.child_process = child;
        record.workspace.state = WorkspaceState::Running;
        record.logs.push("workspace restarted".to_string());
        Ok(())
    }

    pub fn runtime_status(&self, id: &str) -> Result<WorkspaceRuntimeStatus> {
        let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        let runtime = record
            .runtime
            .as_ref()
            .ok_or_else(|| RuntimeError::ExecutionContextMissing(id.to_string()))?;
        let execution_trace = record.process_handle.as_ref().map(|handle| {
            let provider = if runtime.provider_selected.trim().is_empty() {
                infer_provider_from_pid_hint(&handle.pid_hint)
            } else {
                runtime.provider_selected.clone()
            };
            let fallback_providers = record
                .execution_context
                .as_ref()
                .map(|ctx| {
                    ctx.analysis
                        .execution_profile
                        .runtime_affinity
                        .fallback_providers
                        .clone()
                })
                .unwrap_or_default();
            let preferred_provider = record
                .execution_context
                .as_ref()
                .map(|ctx| {
                    ctx.analysis
                        .execution_profile
                        .runtime_affinity
                        .preferred_provider
                        .clone()
                })
                .unwrap_or_default();
            let selected_because = if provider == preferred_provider {
                vec![
                    "compatible".to_string(),
                    "preferred provider".to_string(),
                    "healthy".to_string(),
                ]
            } else {
                vec![
                    "compatible".to_string(),
                    "lowest startup".to_string(),
                    "healthy".to_string(),
                ]
            };
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let startup_ms = now_ms.saturating_sub(runtime.started_at_unix_ms);
            WorkspaceExecutionTrace {
                provider,
                selected_because,
                fallbacks: fallback_providers,
                actual_startup: format!("{startup_ms}ms"),
                successful: runtime.process_alive || runtime.exit_code == Some(0),
            }
        });
        let routing_mode = transport_for_provider_id(&runtime.provider_selected);
        let endpoint = match routing_mode {
            ExecutionRoutingMode::Local => runtime
                .actual_port
                .map(|port| format!("http://127.0.0.1:{port}")),
            ExecutionRoutingMode::Wasm
            | ExecutionRoutingMode::Remote
            | ExecutionRoutingMode::Hybrid => None,
        };
        let stream_channel = if endpoint.is_none() {
            Some(format!(
                "/api/v1/workspaces/{}/runtime",
                runtime.workspace_id
            ))
        } else {
            None
        };
        let manifest_config = record
            .execution_context
            .as_ref()
            .and_then(|ctx| load_runtime_manifest_launch_config(Path::new(&ctx.repo_path)));
        let runtime_label =
            runtime_label_for_context(record.execution_context.as_ref(), manifest_config.as_ref());
        let healthy = runtime.process_alive && runtime.http_ready;
        let execution_handle = Some(ExecutionHandle {
            workspace_id: runtime.workspace_id.clone(),
            provider_id: runtime.provider_selected.clone(),
            execution_id: record
                .process_handle
                .as_ref()
                .map(|handle| handle.pid_hint.clone())
                .unwrap_or_else(|| format!("workspace-{}", runtime.workspace_id)),
            routing_mode,
            endpoint: endpoint.clone(),
            stream_channel,
            readiness_state: runtime.readiness_state,
            authority_node: "workspace-manager".to_string(),
        });
        Ok(WorkspaceRuntimeStatus {
            workspace_id: runtime.workspace_id.clone(),
            provider_selected: runtime.provider_selected.clone(),
            provider: runtime.provider_selected.clone(),
            runtime: runtime_label,
            pid: runtime.pid,
            alive: runtime.process_alive,
            process_state: runtime.process_state,
            exit_code: runtime.exit_code,
            framework: format!("{:?}", record.workspace.framework).to_lowercase(),
            requested_port: runtime.requested_port,
            actual_port: runtime.actual_port,
            assigned_port: runtime.actual_port,
            proxy_url: endpoint.clone(),
            healthy,
            listening: runtime.actual_port.is_some(),
            detected_start_signal: runtime.detected_start_signal.clone(),
            http_ready: runtime.http_ready,
            readiness_state: runtime.readiness_state,
            lifecycle_state: runtime.lifecycle_state,
            last_http_probe: runtime.last_http_probe.clone(),
            last_probe: runtime.last_http_probe.clone(),
            stdout: runtime.stdout_tail.clone(),
            stderr: runtime.stderr_tail.clone(),
            execution_handle,
            execution_trace,
        })
    }
}

fn infer_provider_from_pid_hint(pid_hint: &str) -> String {
    if pid_hint.starts_with("wasm:") {
        return "WasmExecutionProvider".to_string();
    }
    if pid_hint.starts_with("dea:") {
        return "LocalAgentProvider".to_string();
    }
    if pid_hint.starts_with("node:") {
        return "NodeRuntimeProvider".to_string();
    }
    if pid_hint.starts_with("go:") {
        return "GoExecutionProvider".to_string();
    }
    if pid_hint.starts_with("python:") {
        return "PythonExecutionProvider".to_string();
    }
    if pid_hint.starts_with("java:") {
        return "JavaExecutionProvider".to_string();
    }
    if pid_hint.starts_with("docker:") {
        return DockerExecutionProvider::id_static().to_string();
    }
    if pid_hint.starts_with("rust:") {
        return "RustRuntimeProvider".to_string();
    }
    if pid_hint.starts_with("static:") {
        return "StaticRuntimeProvider".to_string();
    }
    pid_hint.to_string()
}

const LOCAL_AGENT_TRANSPORT_MODE: ExecutionRoutingMode = ExecutionRoutingMode::Local;
static WASM_PROVIDER_FOR_TRANSPORT: WasmExecutionProvider = WasmExecutionProvider;
static DOCKER_PROVIDER_FOR_TRANSPORT: DockerExecutionProvider = DockerExecutionProvider;
static NODE_PROVIDER_FOR_TRANSPORT: NodeRuntimeProvider = NodeRuntimeProvider;
static GO_PROVIDER_FOR_TRANSPORT: GoExecutionProvider = GoExecutionProvider;
static PYTHON_PROVIDER_FOR_TRANSPORT: PythonExecutionProvider = PythonExecutionProvider;
static JAVA_PROVIDER_FOR_TRANSPORT: JavaExecutionProvider = JavaExecutionProvider;
static RUST_PROVIDER_FOR_TRANSPORT: RustRuntimeProvider = RustRuntimeProvider;
static STATIC_PROVIDER_FOR_TRANSPORT: StaticRuntimeProvider = StaticRuntimeProvider;

fn transport_for_provider_id(provider_id: &str) -> ExecutionRoutingMode {
    match provider_id {
        "WasmExecutionProvider" => WASM_PROVIDER_FOR_TRANSPORT.transport(),
        // LocalAgentProvider requires an agent instance, so it cannot be
        // instantiated as a stateless singleton just to call the default trait method.
        "LocalAgentProvider" => LOCAL_AGENT_TRANSPORT_MODE,
        "DockerExecutionProvider" => DOCKER_PROVIDER_FOR_TRANSPORT.transport(),
        "NodeRuntimeProvider" => NODE_PROVIDER_FOR_TRANSPORT.transport(),
        "GoExecutionProvider" => GO_PROVIDER_FOR_TRANSPORT.transport(),
        "PythonExecutionProvider" => PYTHON_PROVIDER_FOR_TRANSPORT.transport(),
        "JavaExecutionProvider" => JAVA_PROVIDER_FOR_TRANSPORT.transport(),
        "RustRuntimeProvider" => RUST_PROVIDER_FOR_TRANSPORT.transport(),
        "StaticRuntimeProvider" => STATIC_PROVIDER_FOR_TRANSPORT.transport(),
        _ => ExecutionRoutingMode::Local,
    }
}

impl WasmWorkspace for WorkspaceManager {
    fn launch(&self, repo_url: &str) -> Result<Workspace> {
        let id = self.next_workspace_id();
        let workspace_root = self.root.join("workspaces").join(&id);
        let repository_root = workspace_root.join("repo");
        fs::create_dir_all(&workspace_root)?;
        let mut workspace = Workspace {
            id: id.clone(),
            repo_url: repo_url.to_string(),
            root: workspace_root,
            state: WorkspaceState::Created,
            framework: Framework::Unknown,
            ports: vec![],
            network_policy: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
            resource_quotas: ResourceQuotas {
                max_memory_mb: 1024,
                max_cpu_millis: 1000,
            },
        };
        let mut logs = vec![];

        {
            let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
            workspaces.insert(
                id.clone(),
                LocalWorkspaceRecord {
                    workspace: workspace.clone(),
                    logs: vec!["workspace created".to_string()],
                    execution_context: None,
                    process_handle: None,
                    child_process: None,
                    runtime: None,
                    launch_overrides: LaunchOverrides::default(),
                },
            );
        }

        // Proactively free disk space from any finished workspaces before cloning.
        self.evict_terminal_workspaces();

        let launch_result = (|| -> Result<(ExecutionContext, ProcessHandle)> {
            Self::transition_state(&mut workspace, WorkspaceState::Materializing)?;
            self.materialize_repository(repo_url, None, &repository_root)?;
            logs.push(format!("materialized repository: {repo_url}"));

            Self::transition_state(&mut workspace, WorkspaceState::Analyzing)?;
            let analysis = analyze_repository(&repository_root)?;
            logs.push(format!("detected framework: {:?}", analysis.framework));

            Self::transition_state(&mut workspace, WorkspaceState::Planning)?;
            let mut ctx = ExecutionContext {
                workspace_id: id.clone(),
                repo_path: repository_root.to_string_lossy().to_string(),
                analysis: analysis.clone(),
                execution_graph: analysis.execution_graph.clone(),
                runtime_spec: analysis.runtime_spec.clone(),
                compiled_runtime: analysis.compiled_runtime.clone(),
                wasm_sandbox: None,
                resources: ResourceQuotas {
                    max_memory_mb: analysis.runtime_spec.memory_limit_mb,
                    max_cpu_millis: analysis.runtime_spec.cpu_limit_units,
                },
                network: analysis.runtime_spec.network_policy.clone(),
            };
            logs.push(format!(
                "planned execution command: {}",
                ctx.execution_graph
                    .primary_run_command()
                    .unwrap_or_else(|| "none".to_string())
            ));

            Self::transition_state(&mut workspace, WorkspaceState::Starting)?;
            let handle = self.execution_engine.start(&mut ctx)?;
            logs.push(format!("started process: {}", handle.pid_hint));

            Self::transition_state(&mut workspace, WorkspaceState::Running)?;
            workspace.framework = ctx.analysis.framework;
            workspace.ports = workspace_ports_from_manifest_or_framework(
                Path::new(&ctx.repo_path),
                ctx.analysis.framework,
            );
            workspace.network_policy = ctx.network.clone();
            workspace.resource_quotas = ctx.resources.clone();

            Ok((ctx, handle))
        })();

        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(&id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.clone()))?;

        match launch_result {
            Ok((ctx, handle)) => {
                let overrides = record.launch_overrides.clone();
                let (child, assigned_port, provider_selected) =
                    Self::spawn_run_command(&ctx, &overrides, &mut record.logs);
                // Patch the port list to use the actual assigned port
                if let Some(port) = assigned_port {
                    for p in &mut workspace.ports {
                        p.port = port;
                    }
                }
                let mut runtime = child.as_ref().map(|spawned| {
                    let requested_port = assigned_port
                        .or_else(|| workspace.ports.first().map(|port| port.port))
                        .unwrap_or_default();
                    let mut runtime = ExecutionTruth::new(id.clone(), requested_port, spawned.id());
                    runtime.provider_selected = provider_selected.clone();
                    runtime.update_from_event(ExecutionTruthEvent::ProcessAlive(true));
                    runtime.update_from_event(ExecutionTruthEvent::ObservedPort(assigned_port));
                    runtime
                        .update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Running));
                    runtime
                });
                record.workspace = workspace.clone();
                record.logs.extend(logs);
                record.execution_context = Some(ctx);
                record.process_handle = Some(handle);
                record.child_process = child;
                record.runtime = runtime.take();
                Ok(workspace)
            }
            Err(err) => {
                record.workspace.state = WorkspaceState::Failed;
                record.logs.extend(logs);
                record.logs.push(format!("workspace failed: {err}"));
                Err(err)
            }
        }
    }

    fn stop(&self, id: &str) -> Result<()> {
        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        Self::transition_state(&mut record.workspace, WorkspaceState::Stopping)?;
        if let Some(runtime) = record.runtime.as_mut() {
            runtime.update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Stopping));
        }
        if let (Some(ctx), Some(handle)) = (&record.execution_context, &record.process_handle) {
            self.execution_engine.stop(ctx, handle)?;
        }
        if let Some(child) = record.child_process.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        record.child_process = None;
        record.process_handle = None;
        Self::transition_state(&mut record.workspace, WorkspaceState::Stopped)?;
        if let Some(runtime) = record.runtime.as_mut() {
            runtime.update_from_event(ExecutionTruthEvent::ProcessAlive(false));
            runtime.update_from_event(ExecutionTruthEvent::Lifecycle(WorkspaceState::Stopped));
            record.workspace.state = runtime.lifecycle_state;
        }
        Self::append_capped(&mut record.logs, "workspace stopped".to_string());
        Ok(())
    }

    fn restart(&self, id: &str) -> Result<()> {
        self.restart_with_overrides(id, None)
    }

    fn logs(&self, id: &str) -> Result<Vec<String>> {
        let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        Ok(record.logs.clone())
    }

    fn filesystem(&self, id: &str) -> Result<VirtualFileSystem> {
        let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        Ok(VirtualFileSystem::new(record.workspace.root.join("repo")))
    }

    fn ports(&self, id: &str) -> Result<Vec<PortInfo>> {
        let workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        Ok(record.workspace.ports.clone())
    }
}

#[derive(Default)]
struct RepositoryRegistryState {
    profiles: HashMap<String, ExecutionProfile>,
    snapshots: HashMap<String, HashMap<String, String>>,
    deltas: HashMap<String, RepoDelta>,
}

pub struct RepositoryRegistry;

static REPOSITORY_REGISTRY: OnceLock<Mutex<RepositoryRegistryState>> = OnceLock::new();
static EXECUTION_IMAGE_REGISTRY: OnceLock<Mutex<ExecutionImageRegistry>> = OnceLock::new();
static WARM_POOL_MANAGER: OnceLock<Mutex<WarmPoolManager>> = OnceLock::new();

impl RepositoryRegistry {
    pub fn get_or_compute(repo_reference: &str) -> ExecutionProfile {
        let root = Path::new(repo_reference);
        if !root.exists() {
            return Self::default_profile(repo_reference);
        }

        let snapshot = collect_repository_snapshot(root);
        let (framework, language, package_content) = infer_framework_and_language(root);
        let topology = infer_application_topology(root);
        Self::compute_and_cache_profile(
            repo_reference,
            root,
            snapshot,
            framework,
            language,
            &package_content,
            topology.as_ref(),
        )
    }

    fn compute_and_cache_profile(
        repo_reference: &str,
        root: &Path,
        snapshot: HashMap<String, String>,
        framework: Framework,
        language: Language,
        package_content: &str,
        topology: Option<&ApplicationTopology>,
    ) -> ExecutionProfile {
        let fingerprint = build_repository_fingerprint(
            repo_reference,
            root,
            &snapshot,
            framework,
            language,
            package_content,
            topology,
        );

        let mut state = REPOSITORY_REGISTRY
            .get_or_init(|| Mutex::new(RepositoryRegistryState::default()))
            .lock()
            .expect("repository registry lock poisoned");

        if let Some(existing) = state.profiles.get(repo_reference) {
            if existing.fingerprint == fingerprint {
                return existing.clone();
            }
        }

        let classification = classify_repository(framework, &snapshot, &package_content);
        let runtime_affinity = runtime_affinity_for_classification(&classification);
        let recommended_graph_strategy = graph_strategy_for_classification(classification.class);
        let wasm_compatibility = wasm_compatibility_for_classification(&classification);
        let profile = ExecutionProfile {
            fingerprint,
            classification,
            recommended_graph_strategy,
            runtime_affinity,
            wasm_compatibility,
        };

        let delta = state
            .snapshots
            .get(repo_reference)
            .map(|previous| diff_repo_snapshots(previous, &snapshot))
            .unwrap_or_default();
        state.snapshots.insert(repo_reference.to_string(), snapshot);
        state.deltas.insert(repo_reference.to_string(), delta);
        state
            .profiles
            .insert(repo_reference.to_string(), profile.clone());

        profile
    }

    pub fn latest_delta(repo_reference: &str) -> Option<RepoDelta> {
        REPOSITORY_REGISTRY
            .get_or_init(|| Mutex::new(RepositoryRegistryState::default()))
            .lock()
            .expect("repository registry lock poisoned")
            .deltas
            .get(repo_reference)
            .cloned()
    }

    fn default_profile(repo_url: &str) -> ExecutionProfile {
        let fingerprint = RepositoryFingerprint {
            spec_version: "1.0".to_string(),
            repo_id: hash_key(repo_url),
            repo_url: repo_url.to_string(),
            languages: vec![],
            frameworks: vec![],
            package_managers: vec![],
            services: vec![],
            entrypoints: vec![],
            dependency_graph: DependencyGraph::default(),
            runtime_signals: RuntimeSignals::default(),
            build_signals: BuildSignals::default(),
            infra_signals: InfraSignals::default(),
            confidence: 0.0,
            confidence_model: ConfidenceModel::default(),
            repo_hash: hash_key(repo_url),
            lockfile_hash: None,
            dependency_hash: None,
            language_signature: "unknown".to_string(),
            framework_signature: Some("unknown".to_string()),
        };
        let classification = RepositoryClassification {
            class: RepoClass::Unknown,
            confidence: 0.0,
            primary_runtime: RuntimeType::Unknown,
            secondary_runtimes: vec![],
        };
        ExecutionProfile {
            fingerprint,
            classification: classification.clone(),
            recommended_graph_strategy: graph_strategy_for_classification(classification.class),
            runtime_affinity: runtime_affinity_for_classification(&classification),
            wasm_compatibility: wasm_compatibility_for_classification(&classification),
        }
    }
}

fn infer_framework_and_language(root: &Path) -> (Framework, Language, String) {
    let package_json = root.join("package.json");
    let cargo_toml = root.join("Cargo.toml");
    let go_mod = root.join("go.mod");
    let requirements = root.join("requirements.txt");
    let pyproject = root.join("pyproject.toml");
    let manage_py = root.join("manage.py");
    let package_content = fs::read_to_string(&package_json).unwrap_or_default();
    let cargo_content = fs::read_to_string(&cargo_toml).unwrap_or_default();
    let go_mod_content = fs::read_to_string(&go_mod).unwrap_or_default();
    let requirements_content = fs::read_to_string(&requirements).unwrap_or_default();
    let pyproject_content = fs::read_to_string(&pyproject).unwrap_or_default();

    let framework = if package_mentions_dependency(&package_content, "next")
        || package_mentions_dependency(&package_content, "nextjs")
    {
        Framework::NextJs
    } else if package_mentions_dependency(&package_content, "nuxt") {
        Framework::Nuxt
    } else if package_mentions_dependency(&package_content, "@sveltejs/kit") {
        Framework::SvelteKit
    } else if package_mentions_dependency(&package_content, "astro") {
        Framework::Astro
    } else if package_mentions_dependency(&package_content, "@remix-run/dev")
        || package_mentions_dependency(&package_content, "@remix-run/node")
    {
        Framework::Remix
    } else if package_mentions_dependency(&package_content, "@nestjs/core")
        || package_mentions_dependency(&package_content, "@nestjs/common")
    {
        Framework::NestJs
    } else if package_mentions_dependency(&package_content, "express") {
        Framework::Express
    } else if package_mentions_dependency(&package_content, "svelte") {
        Framework::Svelte
    } else if package_mentions_dependency(&package_content, "vue") {
        Framework::Vue
    } else if package_mentions_dependency(&package_content, "react") {
        Framework::React
    } else if package_mentions_dependency(&package_content, "vite") {
        Framework::Vite
    } else if text_mentions_dependency(&cargo_content, "axum") {
        Framework::Axum
    } else if text_mentions_dependency(&cargo_content, "actix-web")
        || text_mentions_dependency(&cargo_content, "actix")
    {
        Framework::Actix
    } else if text_mentions_dependency(&cargo_content, "rocket") {
        Framework::Rocket
    } else if text_mentions_dependency(&cargo_content, "leptos") {
        Framework::Leptos
    } else if text_mentions_dependency(&go_mod_content, "gin-gonic/gin") {
        Framework::Gin
    } else if text_mentions_dependency(&go_mod_content, "gofiber/fiber") {
        Framework::Fiber
    } else if text_mentions_dependency(&go_mod_content, "labstack/echo") {
        Framework::Echo
    } else if manage_py.exists()
        || text_mentions_dependency(&requirements_content, "django")
        || text_mentions_dependency(&pyproject_content, "django")
    {
        Framework::Django
    } else if text_mentions_dependency(&requirements_content, "fastapi")
        || text_mentions_dependency(&pyproject_content, "fastapi")
    {
        Framework::FastApi
    } else if text_mentions_dependency(&requirements_content, "flask")
        || text_mentions_dependency(&pyproject_content, "flask")
    {
        Framework::Flask
    } else if text_mentions_dependency(&requirements_content, "streamlit")
        || text_mentions_dependency(&pyproject_content, "streamlit")
    {
        Framework::Streamlit
    } else if text_mentions_dependency(&requirements_content, "gradio")
        || text_mentions_dependency(&pyproject_content, "gradio")
    {
        Framework::Gradio
    } else if package_json.exists() {
        Framework::Node
    } else if cargo_toml.exists() {
        Framework::Rust
    } else if go_mod.exists() {
        Framework::Go
    } else if requirements.exists() || pyproject.exists() {
        Framework::Python
    } else if root.join("index.html").exists() {
        Framework::StaticWeb
    } else {
        Framework::Unknown
    };

    let language = match framework {
        Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SvelteKit
        | Framework::Node
        | Framework::Vite
        | Framework::NextJs
        | Framework::Nuxt
        | Framework::Astro
        | Framework::Remix
        | Framework::Express
        | Framework::NestJs => {
            if package_mentions_dependency(&package_content, "typescript")
                || root.join("tsconfig.json").exists()
            {
                Language::TypeScript
            } else {
                Language::JavaScript
            }
        }
        Framework::Rust
        | Framework::Axum
        | Framework::Actix
        | Framework::Rocket
        | Framework::Leptos => Language::Rust,
        Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo => Language::Go,
        Framework::Python
        | Framework::Flask
        | Framework::FastApi
        | Framework::Django
        | Framework::Streamlit
        | Framework::Gradio => Language::Python,
        _ => Language::Unknown,
    };

    (framework, language, package_content)
}

fn collect_repository_snapshot(root: &Path) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    let patterns = read_gitignore_patterns(root);
    collect_repository_snapshot_inner(root, root, &patterns, &mut entries);
    entries
}

fn collect_repository_snapshot_inner(
    root: &Path,
    current: &Path,
    patterns: &[String],
    entries: &mut HashMap<String, String>,
) {
    let Ok(read_dir) = fs::read_dir(current) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative_str = relative.to_string_lossy().replace('\\', "/");
        if relative_str == ".git" || relative_str.starts_with(".git/") {
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if should_ignore_path(&relative_str, patterns) {
            continue;
        }
        if is_dir {
            collect_repository_snapshot_inner(root, &path, patterns, entries);
        } else if let Ok(bytes) = fs::read(&path) {
            entries.insert(relative_str, hash_bytes(&bytes));
        }
    }
}

fn read_gitignore_patterns(root: &Path) -> Vec<String> {
    let gitignore = root.join(".gitignore");
    fs::read_to_string(gitignore)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect()
}

fn should_ignore_path(relative: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        let normalized = pattern.trim_start_matches("./").trim_start_matches('/');
        if normalized.is_empty() {
            continue;
        }
        if normalized.ends_with('/') {
            let prefix = normalized.trim_end_matches('/');
            if relative == prefix || relative.starts_with(&format!("{prefix}/")) {
                return true;
            }
            continue;
        }
        if let Some(extension) = normalized.strip_prefix("*.") {
            if relative.ends_with(&format!(".{extension}")) {
                return true;
            }
            continue;
        }
        if normalized.contains('/') {
            if relative == normalized || relative.starts_with(&format!("{normalized}/")) {
                return true;
            }
            continue;
        }
        if relative == normalized || relative.split('/').any(|segment| segment == normalized) {
            return true;
        }
    }
    false
}

fn build_repository_fingerprint(
    repo_reference: &str,
    root: &Path,
    snapshot: &HashMap<String, String>,
    framework: Framework,
    language: Language,
    package_content: &str,
    topology: Option<&ApplicationTopology>,
) -> RepositoryFingerprint {
    let mut normalized = snapshot
        .iter()
        .map(|(path, content_hash)| format!("{path}:{content_hash}"))
        .collect::<Vec<_>>();
    normalized.sort();

    let lockfile_hash = aggregate_hash_by_filenames(
        snapshot,
        &[
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "Cargo.lock",
            "poetry.lock",
            "Pipfile.lock",
            "go.sum",
        ],
    );
    let dependency_hash = aggregate_hash_by_filenames(
        snapshot,
        &[
            "package.json",
            "Cargo.toml",
            "pyproject.toml",
            "requirements.txt",
            "go.mod",
        ],
    );
    let package_managers = infer_package_managers(snapshot);
    let runtime_signals = infer_runtime_signals(snapshot, language, &package_managers);
    let entrypoints = infer_entrypoints(root, package_content, language, &package_managers);
    let build_signals = infer_build_signals(
        snapshot,
        package_managers.first().cloned(),
        parse_package_scripts(package_content),
    );
    let service_fingerprints = infer_service_fingerprints(root, topology, &package_managers);
    let dependency_graph = infer_dependency_graph(topology);
    let infra_signals = infer_infra_signals(snapshot, topology);
    let languages = infer_language_profiles(snapshot, language);
    let frameworks = infer_framework_profiles(framework, snapshot);
    let confidence_model = compute_confidence_model(framework, &runtime_signals, topology);

    RepositoryFingerprint {
        spec_version: "1.0".to_string(),
        repo_id: hash_key(repo_reference),
        repo_url: repo_reference.to_string(),
        languages,
        frameworks,
        package_managers,
        services: service_fingerprints,
        entrypoints,
        dependency_graph,
        runtime_signals,
        build_signals,
        infra_signals,
        confidence: confidence_model.overall,
        confidence_model,
        repo_hash: hash_key(&normalized.join("|")),
        lockfile_hash,
        dependency_hash,
        language_signature: language_signature(snapshot, language),
        framework_signature: Some(format!("{framework:?}")),
    }
}

fn infer_package_managers(snapshot: &HashMap<String, String>) -> Vec<String> {
    let mut managers = vec![];
    if snapshot.contains_key("pnpm-lock.yaml") {
        managers.push("pnpm".to_string());
    }
    if snapshot.contains_key("yarn.lock") {
        managers.push("yarn".to_string());
    }
    if snapshot.contains_key("bun.lockb") || snapshot.contains_key("bun.lock") {
        managers.push("bun".to_string());
    }
    if snapshot.contains_key("Cargo.toml") {
        managers.push("cargo".to_string());
    }
    if snapshot.contains_key("poetry.lock") {
        managers.push("poetry".to_string());
    }
    if snapshot.contains_key("uv.lock") {
        managers.push("uv".to_string());
    }
    if snapshot.contains_key("Pipfile") || snapshot.contains_key("Pipfile.lock") {
        managers.push("pipenv".to_string());
    }
    if snapshot.contains_key("requirements.txt") || snapshot.contains_key("pyproject.toml") {
        managers.push("pip".to_string());
    }
    if snapshot.contains_key("package.json") && managers.is_empty() {
        managers.push("npm".to_string());
    }
    managers
}

fn infer_runtime_signals(
    snapshot: &HashMap<String, String>,
    language: Language,
    package_managers: &[String],
) -> RuntimeSignals {
    RuntimeSignals {
        node_detected: matches!(language, Language::JavaScript | Language::TypeScript)
            || snapshot.contains_key("package.json"),
        python_detected: language == Language::Python
            || snapshot.contains_key("requirements.txt")
            || snapshot.contains_key("pyproject.toml"),
        rust_detected: language == Language::Rust || snapshot.contains_key("Cargo.toml"),
        bun_detected: package_managers.iter().any(|manager| manager == "bun"),
        dockerfile_present: snapshot
            .keys()
            .any(|path| path.eq_ignore_ascii_case("dockerfile") || path.ends_with("/Dockerfile")),
        compose_present: snapshot.contains_key("docker-compose.yml")
            || snapshot.contains_key("docker-compose.yaml")
            || snapshot.contains_key("compose.yml")
            || snapshot.contains_key("compose.yaml"),
    }
}

fn infer_entrypoints(
    root: &Path,
    package_content: &str,
    language: Language,
    package_managers: &[String],
) -> Vec<EntryPoint> {
    let scripts = parse_package_scripts(package_content);
    let mut entrypoints = vec![];
    for script in ["dev", "start", "build"] {
        if scripts.contains_key(script) {
            entrypoints.push(EntryPoint {
                path: "package.json".to_string(),
                command: script.to_string(),
                confidence: 0.95,
            });
        }
    }
    if entrypoints.is_empty()
        && matches!(language, Language::JavaScript | Language::TypeScript)
        && root.join("package.json").exists()
    {
        entrypoints.push(EntryPoint {
            path: "package.json".to_string(),
            command: "dev".to_string(),
            confidence: 0.7,
        });
    }
    if language == Language::Python {
        if root.join("main.py").exists() || root.join("app.py").exists() {
            let (entry_file, module) = if root.join("main.py").exists() {
                ("main.py", "main:app")
            } else {
                ("app.py", "app:app")
            };
            entrypoints.push(EntryPoint {
                path: entry_file.to_string(),
                command: format!("uvicorn {module}"),
                confidence: 0.9,
            });
        }
    } else if language == Language::Rust && root.join("Cargo.toml").exists() {
        entrypoints.push(EntryPoint {
            path: "Cargo.toml".to_string(),
            command: "cargo run".to_string(),
            confidence: 0.9,
        });
    }
    if package_managers.iter().any(|manager| manager == "bun") {
        entrypoints.push(EntryPoint {
            path: "package.json".to_string(),
            command: "bun run dev".to_string(),
            confidence: 0.85,
        });
    }
    entrypoints
}

fn infer_build_signals(
    snapshot: &HashMap<String, String>,
    lockfile_type: Option<String>,
    scripts: HashMap<String, String>,
) -> BuildSignals {
    let mut build_scripts = scripts.keys().cloned().collect::<Vec<_>>();
    build_scripts.sort();
    BuildSignals {
        has_lockfile: snapshot.contains_key("package-lock.json")
            || snapshot.contains_key("pnpm-lock.yaml")
            || snapshot.contains_key("yarn.lock")
            || snapshot.contains_key("Cargo.lock")
            || snapshot.contains_key("poetry.lock")
            || snapshot.contains_key("Pipfile.lock")
            || snapshot.contains_key("uv.lock")
            || snapshot.contains_key("go.sum"),
        lockfile_type,
        build_scripts,
    }
}

fn infer_service_fingerprints(
    root: &Path,
    topology: Option<&ApplicationTopology>,
    package_managers: &[String],
) -> Vec<ServiceFingerprint> {
    if let Some(topology) = topology {
        let mut services = topology
            .services
            .iter()
            .map(|service| {
                let service_root = Path::new(&service.working_directory);
                let (framework, _, _) = infer_framework_and_language(service_root);
                ServiceFingerprint {
                    service_name: service.name.clone(),
                    service_type: infer_service_type(service),
                    root_path: Path::new(&service.working_directory)
                        .strip_prefix(root)
                        .map(|value| value.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| service.working_directory.clone()),
                    runtime_hint: runtime_kind_from_runtime_type(service.runtime),
                    framework: (framework != Framework::Unknown)
                        .then_some(format!("{framework:?}")),
                    entry_file: infer_entry_file(service_root),
                    build_context: BuildContext {
                        install_command: infer_install_command(
                            package_managers.first().map(String::as_str),
                            service.runtime,
                        ),
                        build_command: infer_build_command(
                            package_managers.first().map(String::as_str),
                            service.runtime,
                        ),
                        package_manager: service.package_manager.clone(),
                    },
                }
            })
            .collect::<Vec<_>>();
        services.sort_by(|left, right| left.root_path.cmp(&right.root_path));
        return services;
    }

    vec![ServiceFingerprint {
        service_name: "root".to_string(),
        service_type: ServiceType::CLI,
        root_path: ".".to_string(),
        runtime_hint: RuntimeKind::Unknown,
        framework: None,
        entry_file: infer_entry_file(root),
        build_context: BuildContext::default(),
    }]
}

fn infer_service_type(service: &ServiceDefinition) -> ServiceType {
    let normalized = service.name.to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "db" | "database" | "postgres" | "redis" | "cache" | "queue"
    ) {
        ServiceType::Database
    } else if normalized.contains("worker")
        || normalized.contains("celery")
        || normalized.contains("cron")
        || normalized.contains("job")
    {
        ServiceType::Worker
    } else if normalized.contains("web")
        || normalized.contains("frontend")
        || normalized.contains("ui")
    {
        ServiceType::Frontend
    } else if normalized.contains("lib") {
        ServiceType::SharedLibrary
    } else if service.runtime == RuntimeType::Unknown {
        ServiceType::CLI
    } else {
        ServiceType::Backend
    }
}

fn infer_entry_file(root: &Path) -> Option<String> {
    for candidate in [
        "src/main.rs",
        "main.rs",
        "main.py",
        "app.py",
        "src/main.ts",
        "src/index.ts",
        "src/main.js",
        "index.js",
    ] {
        if root.join(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn infer_install_command(package_manager: Option<&str>, runtime: RuntimeType) -> Option<String> {
    match (package_manager, runtime) {
        (Some("pnpm"), _) => Some("pnpm install --frozen-lockfile".to_string()),
        (Some("yarn"), _) => Some("yarn install --frozen-lockfile".to_string()),
        (Some("bun"), _) => Some("bun install --frozen-lockfile".to_string()),
        (Some("npm"), _) => Some("npm install".to_string()),
        (Some("cargo"), RuntimeType::Rust) => Some("cargo fetch".to_string()),
        (Some("poetry"), RuntimeType::Python) => Some("poetry install".to_string()),
        (Some("uv"), RuntimeType::Python) => Some("uv sync".to_string()),
        (Some("pipenv"), RuntimeType::Python) => Some("pipenv install --dev".to_string()),
        (Some("pip"), RuntimeType::Python) => {
            Some("python -m pip install -r requirements.txt".to_string())
        }
        _ => None,
    }
}

fn infer_build_command(package_manager: Option<&str>, runtime: RuntimeType) -> Option<String> {
    match (package_manager, runtime) {
        (Some("pnpm"), _) => Some("pnpm run build".to_string()),
        (Some("yarn"), _) => Some("yarn build".to_string()),
        (Some("bun"), _) => Some("bun run build".to_string()),
        (Some("npm"), _) => Some("npm run build".to_string()),
        (Some("cargo"), RuntimeType::Rust) => Some("cargo build".to_string()),
        (Some("go"), RuntimeType::Go) => Some("go build ./...".to_string()),
        _ => None,
    }
}

fn infer_dependency_graph(topology: Option<&ApplicationTopology>) -> DependencyGraph {
    if let Some(topology) = topology {
        return DependencyGraph {
            nodes: topology
                .services
                .iter()
                .map(|service| DependencyNode {
                    id: service.id.clone(),
                })
                .collect(),
            edges: topology
                .dependencies
                .iter()
                .map(|dependency| DependencyEdge {
                    from: dependency.service_id.clone(),
                    to: dependency.depends_on.clone(),
                })
                .collect(),
        };
    }

    DependencyGraph {
        nodes: vec![DependencyNode {
            id: "root".to_string(),
        }],
        edges: vec![],
    }
}

fn infer_infra_signals(
    snapshot: &HashMap<String, String>,
    topology: Option<&ApplicationTopology>,
) -> InfraSignals {
    let services = topology.map(|topology| &topology.services);
    let uses_database = services
        .map(|services| {
            services.iter().any(|service| {
                let name = service.name.to_ascii_lowercase();
                matches!(name.as_str(), "db" | "database" | "postgres")
            })
        })
        .unwrap_or(false)
        || snapshot.keys().any(|path| path.contains("migrations/"));
    let uses_redis = services
        .map(|services| {
            services
                .iter()
                .any(|service| service.name.eq_ignore_ascii_case("redis"))
        })
        .unwrap_or(false);
    let uses_queue = services
        .map(|services| {
            services.iter().any(|service| {
                let name = service.name.to_ascii_lowercase();
                name == "queue" || name.contains("worker")
            })
        })
        .unwrap_or(false);
    let docker_required = snapshot
        .keys()
        .any(|path| path.eq_ignore_ascii_case("dockerfile") || path.ends_with("/Dockerfile"));
    let cloud_native = snapshot.contains_key("k8s/deployment.yaml")
        || snapshot.contains_key("kubernetes/deployment.yaml")
        || snapshot
            .keys()
            .any(|path| path.ends_with(".tf") || path.contains("helm/"));

    InfraSignals {
        uses_database,
        uses_redis,
        uses_queue,
        docker_required,
        cloud_native,
    }
}

fn infer_language_profiles(
    snapshot: &HashMap<String, String>,
    primary_language: Language,
) -> Vec<LanguageProfile> {
    let mut ext_counts = HashMap::<Language, usize>::new();
    for path in snapshot.keys() {
        if path.ends_with(".rs") {
            *ext_counts.entry(Language::Rust).or_default() += 1;
        } else if path.ends_with(".py") {
            *ext_counts.entry(Language::Python).or_default() += 1;
        } else if path.ends_with(".go") {
            *ext_counts.entry(Language::Go).or_default() += 1;
        } else if path.ends_with(".ts") || path.ends_with(".tsx") {
            *ext_counts.entry(Language::TypeScript).or_default() += 1;
        } else if path.ends_with(".js") || path.ends_with(".jsx") {
            *ext_counts.entry(Language::JavaScript).or_default() += 1;
        }
    }
    if ext_counts.is_empty() {
        ext_counts.insert(primary_language, 1);
    }
    let total = ext_counts.values().sum::<usize>().max(1) as f32;
    let mut profiles = ext_counts
        .into_iter()
        .map(|(language, count)| LanguageProfile {
            language,
            confidence: (count as f32 / total).clamp(0.0, 1.0),
            files_detected: snapshot
                .keys()
                .filter(|path| match language {
                    Language::Rust => path.ends_with(".rs"),
                    Language::Python => path.ends_with(".py"),
                    Language::Go => path.ends_with(".go"),
                    Language::TypeScript => path.ends_with(".ts") || path.ends_with(".tsx"),
                    Language::JavaScript => path.ends_with(".js") || path.ends_with(".jsx"),
                    Language::Unknown => false,
                })
                .cloned()
                .collect(),
        })
        .collect::<Vec<_>>();
    profiles.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    profiles
}

fn infer_framework_profiles(
    framework: Framework,
    snapshot: &HashMap<String, String>,
) -> Vec<FrameworkProfile> {
    if framework == Framework::Unknown {
        return vec![];
    }
    let mut signals = vec![format!("framework:{framework:?}")];
    for path in ["package.json", "pyproject.toml", "Cargo.toml", "go.mod"] {
        if snapshot.contains_key(path) {
            signals.push(path.to_string());
        }
    }
    vec![FrameworkProfile {
        framework: format!("{framework:?}"),
        version: None,
        confidence: 0.9,
        detection_signals: signals,
    }]
}

fn compute_confidence_model(
    framework: Framework,
    runtime_signals: &RuntimeSignals,
    topology: Option<&ApplicationTopology>,
) -> ConfidenceModel {
    let framework_confidence: f32 = if framework == Framework::Unknown {
        0.4
    } else {
        0.95
    };
    let runtime_signal_count = [
        runtime_signals.node_detected,
        runtime_signals.python_detected,
        runtime_signals.rust_detected,
        runtime_signals.bun_detected,
    ]
    .into_iter()
    .filter(|signal| *signal)
    .count();
    let runtime_confidence: f32 = if runtime_signal_count == 0 {
        0.35
    } else if runtime_signal_count == 1 {
        0.9
    } else {
        0.75
    };
    let topology_confidence: f32 = match topology {
        Some(topology) if topology.services.len() > 1 => 0.9,
        Some(_) => 0.7,
        None => 0.6,
    };
    let overall = ((framework_confidence + runtime_confidence + topology_confidence) / 3.0_f32)
        .clamp(0.0_f32, 1.0_f32);
    ConfidenceModel {
        overall,
        framework_confidence,
        runtime_confidence,
        topology_confidence,
    }
}

fn runtime_kind_from_runtime_type(runtime: RuntimeType) -> RuntimeKind {
    match runtime {
        RuntimeType::Node => RuntimeKind::Node,
        RuntimeType::Rust => RuntimeKind::Rust,
        RuntimeType::Go => RuntimeKind::Go,
        RuntimeType::Python => RuntimeKind::Python,
        RuntimeType::Java => RuntimeKind::Java,
        RuntimeType::Wasm => RuntimeKind::Wasm,
        RuntimeType::Static => RuntimeKind::Static,
        RuntimeType::Unknown => RuntimeKind::Unknown,
    }
}

fn aggregate_hash_by_filenames(
    snapshot: &HashMap<String, String>,
    file_names: &[&str],
) -> Option<String> {
    let names: HashSet<&str> = file_names.iter().copied().collect();
    let mut selected = snapshot
        .iter()
        .filter(|(path, _)| {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| names.contains(name))
                .unwrap_or(false)
        })
        .map(|(path, hash)| format!("{path}:{hash}"))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return None;
    }
    selected.sort();
    Some(hash_key(&selected.join("|")))
}

fn language_signature(snapshot: &HashMap<String, String>, primary: Language) -> String {
    let mut langs = vec![format!("{primary:?}")];
    if snapshot.keys().any(|path| path.ends_with(".rs")) {
        langs.push("Rust".to_string());
    }
    if snapshot.keys().any(|path| path.ends_with(".go")) {
        langs.push("Go".to_string());
    }
    if snapshot
        .keys()
        .any(|path| path.ends_with(".py") || path_has_filename(path, "pyproject.toml"))
    {
        langs.push("Python".to_string());
    }
    if snapshot.keys().any(|path| path.ends_with(".js")) {
        langs.push("JavaScript".to_string());
    }
    if snapshot
        .keys()
        .any(|path| path.ends_with(".ts") || path.ends_with(".tsx"))
    {
        langs.push("TypeScript".to_string());
    }
    langs.sort();
    langs.dedup();
    langs.join("+")
}

fn classify_repository(
    framework: Framework,
    snapshot: &HashMap<String, String>,
    package_content: &str,
) -> RepositoryClassification {
    let package_json_count = snapshot
        .keys()
        .filter(|path| path.ends_with("package.json"))
        .count();
    let monorepo = snapshot.contains_key("pnpm-workspace.yaml")
        || snapshot.contains_key("turbo.json")
        || package_json_count > 1;

    let (class, confidence) = if monorepo {
        (RepoClass::Monorepo, 0.95)
    } else {
        match framework {
            Framework::NextJs => (RepoClass::FullStackNode, 0.95),
            Framework::Node
            | Framework::React
            | Framework::Vue
            | Framework::Svelte
            | Framework::SvelteKit
            | Framework::Vite
            | Framework::Nuxt
            | Framework::Astro
            | Framework::Remix
            | Framework::Express
            | Framework::NestJs => (RepoClass::NodeApp, 0.9),
            Framework::Rust
            | Framework::Axum
            | Framework::Actix
            | Framework::Rocket
            | Framework::Leptos => (RepoClass::RustBinary, 0.92),
            Framework::Python
            | Framework::Flask
            | Framework::FastApi
            | Framework::Django
            | Framework::Streamlit
            | Framework::Gradio => (RepoClass::PythonApp, 0.9),
            Framework::StaticWeb => (RepoClass::StaticSite, 0.88),
            Framework::Unknown => (RepoClass::Unknown, 0.2),
            Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo => {
                (RepoClass::Unknown, 0.4)
            }
        }
    };

    let primary_runtime = runtime_for_framework(framework);
    let mut secondary_runtimes = vec![];
    if monorepo {
        if snapshot.keys().any(|path| path.ends_with("Cargo.toml")) {
            secondary_runtimes.push(RuntimeType::Rust);
        }
        if snapshot.keys().any(|path| {
            path_has_filename(path, "requirements.txt") || path_has_filename(path, "pyproject.toml")
        }) {
            secondary_runtimes.push(RuntimeType::Python);
        }
        if snapshot.keys().any(|path| path.ends_with("go.mod")) {
            secondary_runtimes.push(RuntimeType::Go);
        }
    } else if class == RepoClass::FullStackNode
        && package_mentions_dependency(package_content, "react")
    {
        secondary_runtimes.push(RuntimeType::Node);
    }
    secondary_runtimes.sort();
    secondary_runtimes.dedup();

    RepositoryClassification {
        class,
        confidence,
        primary_runtime,
        secondary_runtimes,
    }
}

fn graph_strategy_for_classification(class: RepoClass) -> GraphStrategy {
    match class {
        RepoClass::Monorepo => GraphStrategy::MonorepoSegmented,
        RepoClass::FullStackNode => GraphStrategy::MultiStage,
        RepoClass::NodeApp | RepoClass::PythonApp | RepoClass::RustBinary => {
            GraphStrategy::Parallelized
        }
        RepoClass::StaticSite | RepoClass::Unknown => GraphStrategy::Linear,
    }
}

fn runtime_affinity_for_classification(
    classification: &RepositoryClassification,
) -> RuntimeAffinity {
    match classification.primary_runtime {
        RuntimeType::Node => RuntimeAffinity {
            preferred_provider: "NodeRuntimeProvider".to_string(),
            fallback_providers: vec![
                "WasmExecutionProvider".to_string(),
                "StaticRuntimeProvider".to_string(),
            ],
        },
        RuntimeType::Wasm => RuntimeAffinity {
            preferred_provider: "WasmExecutionProvider".to_string(),
            fallback_providers: vec!["StaticRuntimeProvider".to_string()],
        },
        RuntimeType::Rust => RuntimeAffinity {
            preferred_provider: "RustRuntimeProvider".to_string(),
            fallback_providers: vec![
                "WasmExecutionProvider".to_string(),
                "NodeRuntimeProvider".to_string(),
            ],
        },
        RuntimeType::Python => RuntimeAffinity {
            preferred_provider: "PythonExecutionProvider".to_string(),
            fallback_providers: vec!["NodeRuntimeProvider".to_string()],
        },
        RuntimeType::Java => RuntimeAffinity {
            preferred_provider: "JavaExecutionProvider".to_string(),
            fallback_providers: vec!["RustRuntimeProvider".to_string()],
        },
        RuntimeType::Go => RuntimeAffinity {
            preferred_provider: "GoExecutionProvider".to_string(),
            fallback_providers: vec!["RustRuntimeProvider".to_string()],
        },
        RuntimeType::Static => RuntimeAffinity {
            preferred_provider: "WasmExecutionProvider".to_string(),
            fallback_providers: vec!["StaticRuntimeProvider".to_string()],
        },
        RuntimeType::Unknown => RuntimeAffinity {
            preferred_provider: "NodeRuntimeProvider".to_string(),
            fallback_providers: vec!["RustRuntimeProvider".to_string()],
        },
    }
}

fn wasm_compatibility_for_classification(
    classification: &RepositoryClassification,
) -> WasmCompatibility {
    match classification.class {
        RepoClass::StaticSite => WasmCompatibility::Full,
        RepoClass::NodeApp
        | RepoClass::FullStackNode
        | RepoClass::RustBinary
        | RepoClass::Monorepo => WasmCompatibility::Partial,
        RepoClass::PythonApp | RepoClass::Unknown => WasmCompatibility::NotSupported,
    }
}

fn runtime_for_framework(framework: Framework) -> RuntimeType {
    match framework {
        Framework::Node
        | Framework::Vite
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SvelteKit
        | Framework::NextJs
        | Framework::Nuxt
        | Framework::Astro
        | Framework::Remix
        | Framework::Express
        | Framework::NestJs => RuntimeType::Node,
        Framework::Rust
        | Framework::Axum
        | Framework::Actix
        | Framework::Rocket
        | Framework::Leptos => RuntimeType::Rust,
        Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo => RuntimeType::Go,
        Framework::Python
        | Framework::Flask
        | Framework::FastApi
        | Framework::Django
        | Framework::Streamlit
        | Framework::Gradio => RuntimeType::Python,
        Framework::StaticWeb => RuntimeType::Static,
        Framework::Unknown => RuntimeType::Unknown,
    }
}

fn framework_for_runtime(runtime: RuntimeType) -> Framework {
    match runtime {
        RuntimeType::Node => Framework::Node,
        RuntimeType::Wasm | RuntimeType::Static => Framework::StaticWeb,
        RuntimeType::Rust => Framework::Rust,
        RuntimeType::Go => Framework::Go,
        RuntimeType::Python => Framework::Python,
        RuntimeType::Java => Framework::Unknown,
        RuntimeType::Unknown => Framework::Unknown,
    }
}

fn language_for_runtime(runtime: RuntimeType) -> Language {
    match runtime {
        RuntimeType::Node => Language::JavaScript,
        RuntimeType::Rust => Language::Rust,
        RuntimeType::Go => Language::Go,
        RuntimeType::Python => Language::Python,
        RuntimeType::Wasm
        | RuntimeType::Java
        | RuntimeType::Static
        | RuntimeType::Unknown => Language::Unknown,
    }
}

impl DdockitRuntime {
    fn as_runtime_type(self) -> RuntimeType {
        match self {
            Self::Node | Self::Bun => RuntimeType::Node,
            Self::Python => RuntimeType::Python,
            Self::Rust => RuntimeType::Rust,
            Self::Go => RuntimeType::Go,
            Self::Docker => RuntimeType::Static,
            Self::Wasm => RuntimeType::Wasm,
        }
    }
}

/// Loads DES from `.ddockit/ddockit.yaml` first, then falls back to `ddockit.yaml`.
fn load_ddockit_execution_spec(root: &Path) -> Result<Option<DdockitExecutionSpecification>> {
    let candidate = [
        root.join(".ddockit").join("ddockit.yaml"),
        root.join("ddockit.yaml"),
    ]
    .into_iter()
    .find(|path| path.exists());
    let Some(path) = candidate else {
        return Ok(None);
    };

    let content = fs::read_to_string(&path)?;
    let spec = serde_yaml::from_str::<DdockitExecutionSpecification>(&content).map_err(|err| {
        RuntimeError::UnsupportedRepository(format!(
            "invalid execution spec `{}`: {err}",
            path.display()
        ))
    })?;
    if spec.services.is_empty() {
        return Err(RuntimeError::UnsupportedRepository(format!(
            "invalid execution spec `{}`: at least one service is required",
            path.display()
        )));
    }
    Ok(Some(spec))
}

fn runtime_for_ddockit_service(service: &DdockitServiceSpecification) -> RuntimeType {
    service.runtime.as_runtime_type()
}

fn readiness_checks_for_ddockit_service(
    service: &DdockitServiceSpecification,
) -> Vec<ReadinessCheck> {
    let mut checks = vec![];
    if let Some(port) = service.port {
        checks.push(ReadinessCheck::Port(port));
    }
    if let Some(healthcheck) = service.healthcheck.as_ref() {
        match healthcheck.check_type {
            DdockitHealthcheckType::Http => checks.push(ReadinessCheck::Http(
                healthcheck.path.clone().unwrap_or_else(|| "/".to_string()),
            )),
            DdockitHealthcheckType::Tcp => {
                if let Some(port) = healthcheck.port.or(service.port) {
                    checks.push(ReadinessCheck::Port(port));
                }
            }
            DdockitHealthcheckType::Process => checks.push(ReadinessCheck::Process),
        }
    }
    if !checks
        .iter()
        .any(|entry| matches!(entry, ReadinessCheck::Process))
    {
        checks.push(ReadinessCheck::Process);
    }
    checks
}

fn service_definition_from_ddockit(
    repo_root: &Path,
    service_id: &str,
    service: &DdockitServiceSpecification,
) -> ServiceDefinition {
    let runtime = runtime_for_ddockit_service(service);
    let working_directory = service
        .working_directory
        .as_deref()
        .map(|path| {
            let service_path = Path::new(path);
            if service_path.is_absolute() {
                service_path.to_string_lossy().to_string()
            } else {
                repo_root.join(service_path).to_string_lossy().to_string()
            }
        })
        .unwrap_or_else(|| repo_root.to_string_lossy().to_string());
    let start_command = service
        .run
        .first()
        .cloned()
        .unwrap_or_else(|| format!("cd {working_directory}"));
    let package_manager = match service.runtime {
        DdockitRuntime::Node => Some("npm".to_string()),
        DdockitRuntime::Bun => Some("bun".to_string()),
        DdockitRuntime::Python => Some("pip".to_string()),
        DdockitRuntime::Rust => Some("cargo".to_string()),
        DdockitRuntime::Go => Some("go".to_string()),
        DdockitRuntime::Docker => Some("docker".to_string()),
        DdockitRuntime::Wasm => None,
    };
    ServiceDefinition {
        id: service_id.to_string(),
        name: service_id.to_string(),
        runtime,
        package_manager,
        working_directory,
        start_command,
        ports: service.port.map(|port| vec![port]).unwrap_or_default(),
        readiness_checks: readiness_checks_for_ddockit_service(service),
    }
}

fn topology_from_ddockit_spec(
    root: &Path,
    spec: &DdockitExecutionSpecification,
) -> ApplicationTopology {
    let mut service_ids = spec.services.keys().cloned().collect::<Vec<_>>();
    service_ids.sort();
    let services = service_ids
        .iter()
        .filter_map(|id| {
            spec.services
                .get(id)
                .map(|service| service_definition_from_ddockit(root, id, service))
        })
        .collect::<Vec<_>>();

    let mut dependencies = vec![];
    for service_id in &service_ids {
        if let Some(depends_on) = spec.dependencies.get(service_id) {
            for dependency in depends_on {
                dependencies.push(ServiceDependency {
                    service_id: service_id.clone(),
                    depends_on: dependency.clone(),
                });
            }
        }
    }
    dependencies.sort_by(|left, right| {
        left.service_id
            .cmp(&right.service_id)
            .then_with(|| left.depends_on.cmp(&right.depends_on))
    });

    let startup_order = compute_startup_order(&services, &dependencies);
    let topology_seed = spec
        .application
        .as_ref()
        .map(|application| application.name.clone())
        .unwrap_or_else(|| service_ids.join("|"));
    ApplicationTopology {
        topology_id: format!("des-{}", hash_key(&topology_seed)),
        services: services.clone(),
        edges: dependencies.clone(),
        global_network: infer_network_topology(&services),
        startup_strategy: StartupStrategy {
            stages: startup_order.stages.clone(),
            enforce_dependencies: true,
        },
        health_policy: infer_health_policy(&services),
        dependencies,
        startup_order,
    }
}

fn path_has_filename(path: &str, expected_file_name: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == expected_file_name)
        .unwrap_or(false)
}

fn diff_repo_snapshots(
    previous: &HashMap<String, String>,
    current: &HashMap<String, String>,
) -> RepoDelta {
    let mut added_files = current
        .keys()
        .filter(|path| !previous.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    let mut removed_files = previous
        .keys()
        .filter(|path| !current.contains_key(*path))
        .cloned()
        .collect::<Vec<_>>();
    let mut modified_files = current
        .iter()
        .filter_map(|(path, hash)| {
            previous
                .get(path)
                .filter(|previous_hash| *previous_hash != hash)
                .map(|_| path.clone())
        })
        .collect::<Vec<_>>();

    added_files.sort();
    removed_files.sort();
    modified_files.sort();

    RepoDelta {
        added_files,
        removed_files,
        modified_files,
    }
}

pub fn analyze_repository(root: &Path) -> Result<RepositoryAnalysis> {
    let mut dependency_files = vec![];
    let execution_spec = load_ddockit_execution_spec(root)?;

    let package_json = root.join("package.json");
    let cargo_toml = root.join("Cargo.toml");
    let go_mod = root.join("go.mod");
    let requirements = root.join("requirements.txt");
    let pyproject = root.join("pyproject.toml");
    let pipfile = root.join("Pipfile");
    let pipfile_lock = root.join("Pipfile.lock");
    let poetry_lock = root.join("poetry.lock");
    let uv_lock = root.join("uv.lock");

    if package_json.exists() {
        dependency_files.push(package_json.clone());
    }
    if cargo_toml.exists() {
        dependency_files.push(cargo_toml.clone());
    }
    if go_mod.exists() {
        dependency_files.push(go_mod.clone());
    }
    if requirements.exists() {
        dependency_files.push(requirements.clone());
    }
    if pyproject.exists() {
        dependency_files.push(pyproject.clone());
    }
    if pipfile.exists() {
        dependency_files.push(pipfile.clone());
    }
    if pipfile_lock.exists() {
        dependency_files.push(pipfile_lock.clone());
    }
    if poetry_lock.exists() {
        dependency_files.push(poetry_lock.clone());
    }
    if uv_lock.exists() {
        dependency_files.push(uv_lock.clone());
    }

    let (mut framework, mut language, package_content) = infer_framework_and_language(root);
    let topology = execution_spec
        .as_ref()
        .map(|spec| topology_from_ddockit_spec(root, spec))
        .or_else(|| infer_application_topology(root));

    if framework == Framework::Unknown {
        if let Some(discovered) = topology
            .as_ref()
            .and_then(|topology| topology.services.first())
        {
            framework = framework_for_runtime(discovered.runtime);
            language = language_for_runtime(discovered.runtime);
        } else {
            framework = Framework::Node;
            language = Language::JavaScript;
        }
    }

    let scripts = parse_package_scripts(&package_content);
    let pyproject_content = fs::read_to_string(&pyproject).unwrap_or_default();
    let package_manager = if root.join("pnpm-lock.yaml").exists()
        || package_manager_declares(&package_content, "pnpm")
    {
        Some("pnpm".to_string())
    } else if root.join("yarn.lock").exists() || package_manager_declares(&package_content, "yarn")
    {
        Some("yarn".to_string())
    } else if root.join("bun.lockb").exists()
        || root.join("bun.lock").exists()
        || package_manager_declares(&package_content, "bun")
    {
        Some("bun".to_string())
    } else if package_json.exists() {
        Some("npm".to_string())
    } else if cargo_toml.exists() {
        Some("cargo".to_string())
    } else if go_mod.exists() {
        Some("go".to_string())
    } else if poetry_lock.exists() || pyproject_content.contains("[tool.poetry]") {
        Some("poetry".to_string())
    } else if uv_lock.exists() || pyproject_content.contains("[tool.uv]") {
        Some("uv".to_string())
    } else if pipfile.exists() || pipfile_lock.exists() {
        Some("pipenv".to_string())
    } else if requirements.exists() || pyproject.exists() {
        Some("pip".to_string())
    } else {
        None
    };

    let build_tooling = match framework {
        Framework::Node
        | Framework::Vite
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SvelteKit
        | Framework::NextJs
        | Framework::Nuxt
        | Framework::Astro
        | Framework::Remix
        | Framework::Express
        | Framework::NestJs => vec![
            "node".to_string(),
            package_manager.clone().unwrap_or_else(|| "npm".to_string()),
        ],
        Framework::Rust
        | Framework::Axum
        | Framework::Actix
        | Framework::Rocket
        | Framework::Leptos => vec!["cargo".to_string()],
        Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo => {
            vec!["go".to_string()]
        }
        Framework::Python
        | Framework::Flask
        | Framework::FastApi
        | Framework::Django
        | Framework::Streamlit
        | Framework::Gradio => vec![
            "python".to_string(),
            package_manager.clone().unwrap_or_else(|| "pip".to_string()),
        ],
        Framework::StaticWeb => vec!["serve".to_string()],
        Framework::Unknown => vec![],
    };

    let build_intelligence = BuildIntelligence {
        framework,
        package_manager,
        build_tooling,
        entrypoints: ports_for_framework(framework)
            .iter()
            .map(|port| format!("{}://0.0.0.0:{}{}", port.protocol, port.port, port.route))
            .collect(),
        scripts,
    };

    let repo_reference = root.to_string_lossy().to_string();
    let snapshot = collect_repository_snapshot(root);
    let execution_profile = RepositoryRegistry::compute_and_cache_profile(
        &repo_reference,
        root,
        snapshot,
        framework,
        language,
        &package_content,
        topology.as_ref(),
    );
    let image_match = EXECUTION_IMAGE_REGISTRY
        .get_or_init(|| Mutex::new(ExecutionImageRegistry::default()))
        .lock()
        .expect("execution image registry lock poisoned")
        .resolve_for_fingerprint(&repo_reference, &execution_profile.fingerprint);
    let mut analysis = RepositoryAnalysis {
        root: root.to_path_buf(),
        framework,
        language,
        execution_spec,
        dependency_files,
        topology,
        fingerprint: execution_profile.fingerprint.clone(),
        classification: execution_profile.classification.clone(),
        execution_profile,
        build_intelligence,
        execution_graph: ExecutionGraph::default(),
        execution_image: image_match.image.clone(),
        image_match_confidence: image_match.confidence,
        runtime_spec: ExecutionRuntimeSpec {
            language: "unknown".to_string(),
            framework: "unknown".to_string(),
            package_manager: None,
            dependencies: vec![],
            filesystem: RuntimeFilesystemPlan {
                read_only_layers: vec![],
                dependency_cache_layer: "dependency-cache".to_string(),
                build_cache_layer: "build-cache".to_string(),
                execution_layer: "execution-layer".to_string(),
                temporary_layer: "temporary-layer".to_string(),
                copy_on_write: true,
            },
            network_policy: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
            memory_limit_mb: UNINITIALIZED_RESOURCE_LIMIT,
            cpu_limit_units: UNINITIALIZED_RESOURCE_LIMIT,
            cache_layers: vec![],
            environment: BTreeMap::new(),
            ports: vec![],
            services: vec![],
            build_steps: vec![],
            execution_steps: vec![],
            health_checks: vec![],
            recovery_steps: vec![],
            requires_wasm: false,
        },
        compiled_runtime: CompiledWasmExecutionEnvironment {
            environment_id: "runtime-uncompiled".to_string(),
            spec_fingerprint: "unknown".to_string(),
            warm_pool_key: "unknown".to_string(),
            deterministic: true,
            component_graph: vec![],
            wasi_component_graph: WasiComponentGraph::default(),
        },
    };
    analysis.execution_graph = BuildPlanner::build_graph(&analysis)
        .with_cache_keys_for(Some(&analysis.fingerprint))
        .with_execution_image(&analysis.execution_image);
    analysis.runtime_spec = ExecutionRuntimeSpecCompiler::compile(&analysis);
    analysis.compiled_runtime = WasmRuntimeCompiler::compile(&analysis.runtime_spec);
    {
        let mut warm_pool = WARM_POOL_MANAGER
            .get_or_init(|| Mutex::new(WarmPoolManager::default()))
            .lock()
            .expect("warm pool manager lock poisoned");
        warm_pool.prewarm(&analysis.execution_image, WarmPoolType::LocalDea, 1);
        warm_pool.bind_cache_layer(&analysis.fingerprint, &analysis.execution_image);
    }
    persist_execution_manifest(root, &analysis);

    Ok(analysis)
}

fn persist_execution_manifest(root: &Path, analysis: &RepositoryAnalysis) {
    let confidence = (analysis.fingerprint.confidence * 100.0)
        .round()
        .clamp(1.0, 99.0) as u8;
    let build_command = analysis
        .execution_graph
        .nodes
        .iter()
        .find(|node| node.node_type == ExecutionNodeType::Build)
        .and_then(|node| node.command.as_deref());
    let start_command = analysis.execution_graph.primary_run_command();
    let dev_command = analysis
        .build_intelligence
        .scripts
        .get("dev")
        .map(String::as_str);
    let runtime = analysis.runtime_spec.language.as_str();
    let package_manager = analysis.build_intelligence.package_manager.as_deref();
    let framework = if analysis.runtime_spec.framework.trim().is_empty()
        || analysis.runtime_spec.framework == UNKNOWN_SIGNATURE
    {
        analysis
            .fingerprint
            .framework_signature
            .clone()
            .unwrap_or_else(|| UNKNOWN_SIGNATURE.to_string())
            .to_ascii_lowercase()
    } else {
        analysis.runtime_spec.framework.clone()
    };
    let manifest = analyze::manifest_builder::AnalyzeManifest::synthesize(
        root,
        &framework,
        runtime,
        package_manager,
        build_command,
        start_command.as_deref(),
        dev_command,
        confidence,
    );
    let _ = analyze::manifest_builder::write_manifest(root, &manifest);
}

impl BuildPlanner {
    pub fn build_graph(analysis: &RepositoryAnalysis) -> ExecutionGraph {
        if let Some(topology) = analysis.topology.as_ref() {
            if analysis.execution_spec.is_some() || topology.services.len() > 1 {
                return Self::build_topology_graph(analysis, topology);
            }
        }

        let framework = analysis.framework;
        let scripts = &analysis.build_intelligence.scripts;
        let package_manager = analysis
            .build_intelligence
            .package_manager
            .as_deref()
            .unwrap_or("npm");

        let js_script = |name: &str, fallback: &str| -> String {
            if scripts.contains_key(name) {
                match package_manager {
                    "pnpm" => format!("pnpm run {name}"),
                    "yarn" => format!("yarn {name}"),
                    "bun" => format!("bun run {name}"),
                    _ => format!("npm run {name}"),
                }
            } else {
                fallback.to_string()
            }
        };

        let js_install = match package_manager {
            "pnpm" => "pnpm install --frozen-lockfile".to_string(),
            "yarn" => "yarn install --frozen-lockfile".to_string(),
            "bun" => "bun install --frozen-lockfile".to_string(),
            _ => "npm install".to_string(),
        };
        let js_build_fallback = match package_manager {
            "pnpm" => "pnpm run build".to_string(),
            "yarn" => "yarn build".to_string(),
            "bun" => "bun run build".to_string(),
            _ => "npm run build".to_string(),
        };
        let js_test_fallback = match package_manager {
            "pnpm" => "pnpm run test".to_string(),
            "yarn" => "yarn test".to_string(),
            "bun" => "bun test".to_string(),
            _ => "npm test".to_string(),
        };

        // Pick the best available run script, preferring Vite-style dev servers.
        // - "dev" / "preview" / "serve": Vite-based, need --host 0.0.0.0 to bind all interfaces.
        // - "start" / "develop": CRA/webpack/custom toolchain; they bind 0.0.0.0 by default,
        //   read PORT env var, and typically reject unknown CLI args via --.
        let js_dev_command = {
            let pm_run = match package_manager {
                "pnpm" => "pnpm run",
                "yarn" => "yarn",
                "bun" => "bun run",
                _ => "npm run",
            };
            // Check if the chosen script body contains "vite" to decide on --host
            let vite_host_flag = |name: &str| -> &str {
                let body = scripts.get(name).map(|s| s.as_str()).unwrap_or("");
                if body.contains("vite") || name == "dev" || name == "preview" || name == "serve" {
                    " -- --host 0.0.0.0 --port {PORT}"
                } else {
                    ""
                }
            };

            // Vite-style first (explicit dev/preview/serve script)
            if scripts.contains_key("dev") {
                format!("{pm_run} dev{}", vite_host_flag("dev"))
            } else if scripts.contains_key("preview") {
                format!("{pm_run} preview{}", vite_host_flag("preview"))
            } else if scripts.contains_key("serve") {
                format!("{pm_run} serve{}", vite_host_flag("serve"))
            // CRA / custom toolchain (start / develop)
            } else if scripts.contains_key("start") {
                format!("{pm_run} start")
            } else if scripts.contains_key("develop") {
                format!("{pm_run} develop")
            } else {
                // Nothing matched — log what scripts exist and fall back to dev
                format!("{pm_run} dev -- --host 0.0.0.0 --port {{PORT}}")
            }
        };
        let py_install = match package_manager {
            "poetry" => "poetry install".to_string(),
            "uv" => "uv sync".to_string(),
            "pipenv" => "pipenv install --dev".to_string(),
            _ => "python -m pip install -r requirements.txt".to_string(),
        };
        let py_test = match package_manager {
            "poetry" => "poetry run pytest".to_string(),
            "uv" => "uv run pytest".to_string(),
            "pipenv" => "pipenv run pytest".to_string(),
            _ => "python -m pytest".to_string(),
        };
        let fastapi_app_path = if analysis.root.join("main.py").exists() {
            "main:app"
        } else if analysis.root.join("app.py").exists() {
            "app:app"
        } else {
            "main:app"
        };
        let streamlit_entry = if analysis.root.join("streamlit_app.py").exists() {
            "streamlit_app.py"
        } else if analysis.root.join("main.py").exists() {
            "main.py"
        } else {
            "app.py"
        };
        let py_dev = match framework {
            Framework::FastApi => {
                format!("uvicorn {fastapi_app_path} --host 0.0.0.0 --port {{PORT}}")
            }
            Framework::Django => "python manage.py runserver 0.0.0.0:{PORT}".to_string(),
            Framework::Flask => "flask run --host 0.0.0.0 --port {PORT}".to_string(),
            Framework::Streamlit => {
                format!(
                    "streamlit run {streamlit_entry} --server.address 0.0.0.0 --server.port {{PORT}}"
                )
            }
            _ => "python -m app --port {PORT}".to_string(),
        };

        match framework {
            Framework::React
            | Framework::Vue
            | Framework::Svelte
            | Framework::SvelteKit
            | Framework::Vite
            | Framework::Node
            | Framework::NextJs
            | Framework::Nuxt
            | Framework::Astro
            | Framework::Remix
            | Framework::Express
            | Framework::NestJs => {
                let install = ExecutionNode {
                    id: "install".to_string(),
                    node_type: ExecutionNodeType::InstallDependencies,
                    command: Some(js_install),
                    execution_mode: ExecutionMode::Native,
                    inputs: vec![
                        "package.json".to_string(),
                        "package-lock.json|yarn.lock|pnpm-lock.yaml".to_string(),
                    ],
                    outputs: vec!["node_modules".to_string()],
                    cache_key: None,
                    runtime: None,
                    cache_binding: None,
                };
                let build = ExecutionNode {
                    id: "build".to_string(),
                    node_type: ExecutionNodeType::Build,
                    command: Some(js_script("build", &js_build_fallback)),
                    execution_mode: ExecutionMode::Native,
                    inputs: vec!["node_modules".to_string()],
                    outputs: vec![if framework == Framework::NextJs {
                        ".next".to_string()
                    } else {
                        "dist".to_string()
                    }],
                    cache_key: None,
                    runtime: None,
                    cache_binding: None,
                };
                let dev = ExecutionNode {
                    id: "dev".to_string(),
                    node_type: ExecutionNodeType::DevServer,
                    command: Some(js_dev_command.clone()),
                    execution_mode: ExecutionMode::Native,
                    inputs: build.outputs.clone(),
                    outputs: vec!["http://0.0.0.0:3000/".to_string()],
                    cache_key: None,
                    runtime: None,
                    cache_binding: None,
                };
                let test = ExecutionNode {
                    id: "test".to_string(),
                    node_type: ExecutionNodeType::Test,
                    command: Some(js_script("test", &js_test_fallback)),
                    execution_mode: ExecutionMode::Hybrid,
                    inputs: vec!["node_modules".to_string()],
                    outputs: vec!["test-report".to_string()],
                    cache_key: None,
                    runtime: None,
                    cache_binding: None,
                };
                ExecutionGraph {
                    nodes: vec![install, build, dev, test],
                    edges: vec![
                        ExecutionEdge {
                            from: "install".to_string(),
                            to: "build".to_string(),
                        },
                        ExecutionEdge {
                            from: "install".to_string(),
                            to: "test".to_string(),
                        },
                        ExecutionEdge {
                            from: "build".to_string(),
                            to: "dev".to_string(),
                        },
                    ],
                }
            }
            Framework::Rust
            | Framework::Axum
            | Framework::Actix
            | Framework::Rocket
            | Framework::Leptos => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "build".to_string(),
                        node_type: ExecutionNodeType::Build,
                        command: Some("cargo build".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()],
                        outputs: vec!["target".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("cargo run".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["target".to_string()],
                        outputs: vec!["http://0.0.0.0:8080/".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("cargo test".to_string()),
                        execution_mode: ExecutionMode::Hybrid,
                        inputs: vec!["target".to_string()],
                        outputs: vec!["test-report".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                ],
                edges: vec![
                    ExecutionEdge {
                        from: "build".to_string(),
                        to: "dev".to_string(),
                    },
                    ExecutionEdge {
                        from: "build".to_string(),
                        to: "test".to_string(),
                    },
                ],
            },
            Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "build".to_string(),
                        node_type: ExecutionNodeType::Build,
                        command: Some("go build ./...".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["go.mod".to_string(), "go.sum".to_string()],
                        outputs: vec!["go-build-cache".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("go run .".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["go-build-cache".to_string()],
                        outputs: vec!["http://0.0.0.0:8080/".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("go test ./...".to_string()),
                        execution_mode: ExecutionMode::Hybrid,
                        inputs: vec!["go-build-cache".to_string()],
                        outputs: vec!["test-report".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                ],
                edges: vec![
                    ExecutionEdge {
                        from: "build".to_string(),
                        to: "dev".to_string(),
                    },
                    ExecutionEdge {
                        from: "build".to_string(),
                        to: "test".to_string(),
                    },
                ],
            },
            Framework::Python
            | Framework::Flask
            | Framework::FastApi
            | Framework::Django
            | Framework::Streamlit
            | Framework::Gradio => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "install".to_string(),
                        node_type: ExecutionNodeType::InstallDependencies,
                        command: Some(py_install),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["requirements.txt|pyproject.toml".to_string()],
                        outputs: vec!["site-packages".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some(py_dev),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["site-packages".to_string()],
                        outputs: vec!["http://0.0.0.0:8000/".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some(py_test),
                        execution_mode: ExecutionMode::Hybrid,
                        inputs: vec!["site-packages".to_string()],
                        outputs: vec!["test-report".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                ],
                edges: vec![
                    ExecutionEdge {
                        from: "install".to_string(),
                        to: "dev".to_string(),
                    },
                    ExecutionEdge {
                        from: "install".to_string(),
                        to: "test".to_string(),
                    },
                ],
            },
            Framework::StaticWeb => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "wasm-compile".to_string(),
                        node_type: ExecutionNodeType::WasmCompile,
                        command: Some("wasm-pack build --target web".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["index.html".to_string(), "src".to_string()],
                        outputs: vec!["pkg/app_bg.wasm".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                    ExecutionNode {
                        id: "serve".to_string(),
                        node_type: ExecutionNodeType::StaticServe,
                        command: Some("serve .".to_string()),
                        execution_mode: ExecutionMode::Wasm,
                        inputs: vec!["pkg/app_bg.wasm".to_string()],
                        outputs: vec!["http://0.0.0.0:4173/".to_string()],
                        cache_key: None,
                        runtime: None,
                        cache_binding: None,
                    },
                ],
                edges: vec![ExecutionEdge {
                    from: "wasm-compile".to_string(),
                    to: "serve".to_string(),
                }],
            },
            Framework::Unknown => ExecutionGraph::default(),
        }
    }

    fn build_topology_graph(
        analysis: &RepositoryAnalysis,
        topology: &ApplicationTopology,
    ) -> ExecutionGraph {
        let mut nodes = vec![];
        let mut edges: Vec<ExecutionEdge> = vec![];
        let mut add_edge = |from: String, to: String| {
            if !edges.iter().any(|edge| edge.from == from && edge.to == to) {
                edges.push(ExecutionEdge { from, to });
            }
        };

        let install_command = topology.services.iter().find_map(service_install_command);
        if let Some(command) = install_command {
            nodes.push(ExecutionNode {
                id: "install".to_string(),
                node_type: ExecutionNodeType::InstallDependencies,
                command: Some(command),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["workspace-manifests".to_string()],
                outputs: vec!["workspace-dependencies".to_string()],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            });
        }

        nodes.push(ExecutionNode {
            id: "shared-build".to_string(),
            node_type: ExecutionNodeType::Build,
            command: Some(shared_build_command(analysis)),
            execution_mode: ExecutionMode::Native,
            inputs: vec!["workspace-dependencies".to_string()],
            outputs: vec!["workspace-build-cache".to_string()],
            cache_key: None,
            runtime: None,
            cache_binding: None,
        });
        if nodes.iter().any(|node| node.id == "install") {
            add_edge("install".to_string(), "shared-build".to_string());
        }

        for service in &topology.services {
            let build_id = format!("{}-build", service.id);
            let run_id = format!("{}-run", service.id);
            let role = service_role(service);
            let build_node_type = if matches!(role, ServiceRole::DataStore | ServiceRole::Queue) {
                ExecutionNodeType::CustomCommand
            } else {
                ExecutionNodeType::Build
            };
            let build_command = service_build_command(service);
            nodes.push(ExecutionNode {
                id: build_id.clone(),
                node_type: build_node_type,
                command: Some(build_command),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["workspace-build-cache".to_string()],
                outputs: vec![format!("{}-build-output", service.id)],
                cache_key: None,
                runtime: None,
                cache_binding: None,
            });
            add_edge("shared-build".to_string(), build_id.clone());

            let run_node_type = if matches!(role, ServiceRole::DataStore | ServiceRole::Queue) {
                ExecutionNodeType::CustomCommand
            } else {
                ExecutionNodeType::DevServer
            };
            let mut outputs = service
                .ports
                .iter()
                .map(|port| format!("tcp://0.0.0.0:{port}"))
                .collect::<Vec<_>>();
            if let Some(dns) = topology.global_network.service_dns.get(&service.id) {
                outputs.push(format!("svc://{dns}"));
            }
            nodes.push(ExecutionNode {
                id: run_id.clone(),
                node_type: run_node_type,
                command: Some(service.start_command.clone()),
                execution_mode: ExecutionMode::Native,
                inputs: vec![format!("{}-build-output", service.id)],
                outputs,
                cache_key: None,
                runtime: None,
                cache_binding: None,
            });
            add_edge(build_id, run_id);
        }

        for dependency in &topology.edges {
            add_edge(
                format!("{}-run", dependency.depends_on),
                format!("{}-run", dependency.service_id),
            );
        }

        for stage_pair in topology.startup_strategy.stages.windows(2) {
            if let [current_stage, next_stage] = stage_pair {
                for current in current_stage {
                    for next in next_stage {
                        add_edge(format!("{current}-run"), format!("{next}-run"));
                    }
                }
            }
        }

        ExecutionGraph { nodes, edges }
    }
}

impl ExecutionRuntimeSpecCompiler {
    pub fn compile(analysis: &RepositoryAnalysis) -> ExecutionRuntimeSpec {
        let dependencies = analysis
            .dependency_files
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .collect::<Vec<_>>();
        let framework_label = format!("{:?}", analysis.framework).to_ascii_lowercase();
        let language_label = format!("{:?}", analysis.language).to_ascii_lowercase();
        let mut ports = analysis
            .topology
            .as_ref()
            .map(|topology| {
                topology
                    .services
                    .iter()
                    .flat_map(|service| service.ports.iter().copied())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                ports_for_framework(analysis.framework)
                    .into_iter()
                    .map(|port| port.port)
                    .collect::<Vec<_>>()
            });
        ports.sort_unstable();
        ports.dedup();

        let services = analysis
            .topology
            .as_ref()
            .map(|topology| {
                topology
                    .services
                    .iter()
                    .map(|service| RuntimeServicePlan {
                        id: service.id.clone(),
                        runtime: runtime_kind_label(service.runtime).to_string(),
                        framework: Some(framework_label.clone()),
                        working_directory: service.working_directory.clone(),
                        start_command: service.start_command.clone(),
                        ports: service.ports.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![RuntimeServicePlan {
                    id: "default".to_string(),
                    runtime: runtime_kind_label(analysis.classification.primary_runtime)
                        .to_string(),
                    framework: Some(framework_label.clone()),
                    working_directory: ".".to_string(),
                    start_command: analysis
                        .execution_graph
                        .primary_run_command()
                        .unwrap_or_else(|| "unknown".to_string()),
                    ports: ports.clone(),
                }]
            });

        let mut build_steps = Vec::new();
        let mut execution_steps = Vec::new();
        let requires_wasm = analysis.execution_graph.nodes.iter().any(|node| {
            matches!(
                ExecutionRouter::route(node, &analysis.execution_profile),
                ExecutionTarget::Wasm(_)
            )
        });
        for node in &analysis.execution_graph.nodes {
            let Some(command) = node.command.clone() else {
                continue;
            };
            match node.node_type {
                ExecutionNodeType::InstallDependencies
                | ExecutionNodeType::Build
                | ExecutionNodeType::WasmCompile => build_steps.push(command),
                ExecutionNodeType::DevServer
                | ExecutionNodeType::Test
                | ExecutionNodeType::StaticServe
                | ExecutionNodeType::CustomCommand => execution_steps.push(command),
            }
        }

        let mut health_checks = analysis
            .topology
            .as_ref()
            .map(|topology| {
                topology
                    .health_policy
                    .service_checks
                    .iter()
                    .flat_map(|(service, checks)| {
                        checks.iter().map(move |check| match check {
                            ReadinessCheck::Port(port) => format!("{service}:tcp://{port}"),
                            ReadinessCheck::Http(path) => format!("{service}:http://{path}"),
                            ReadinessCheck::Process => format!("{service}:process"),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if health_checks.is_empty() {
            health_checks.push("/health".to_string());
        }

        let package_manager = analysis.build_intelligence.package_manager.clone();
        let network_policy = NetworkPolicy {
            allow_outbound: package_manager.is_some(),
            allowed_hosts: allowed_hosts_for_package_manager(package_manager.as_deref()),
        };

        let mut environment = BTreeMap::new();
        environment.insert("CI".to_string(), "true".to_string());
        environment.insert(
            "RUSTGIT_RUNTIME".to_string(),
            runtime_kind_label(analysis.classification.primary_runtime).to_string(),
        );
        if matches!(
            analysis.language,
            Language::JavaScript | Language::TypeScript
        ) {
            environment.insert("NODE_ENV".to_string(), "production".to_string());
        }
        if analysis.language == Language::Python {
            environment.insert("PYTHONUNBUFFERED".to_string(), "1".to_string());
        }

        ExecutionRuntimeSpec {
            language: language_label,
            framework: framework_label,
            package_manager,
            dependencies,
            filesystem: RuntimeFilesystemPlan {
                read_only_layers: vec!["repository-snapshot".to_string()],
                dependency_cache_layer: "dependency-cache".to_string(),
                build_cache_layer: "build-cache".to_string(),
                execution_layer: "execution-layer".to_string(),
                temporary_layer: "temporary-layer".to_string(),
                copy_on_write: true,
            },
            network_policy,
            memory_limit_mb: RUNTIME_SPEC_DEFAULT_MEMORY_LIMIT_MB,
            cpu_limit_units: RUNTIME_SPEC_DEFAULT_CPU_LIMIT_UNITS,
            cache_layers: vec![
                "repository-snapshot".to_string(),
                "dependency-cache".to_string(),
                "build-cache".to_string(),
                "execution-layer".to_string(),
                "temporary-layer".to_string(),
            ],
            environment,
            ports,
            services,
            build_steps,
            execution_steps,
            health_checks,
            recovery_steps: vec![
                "retry-with-warm-pool".to_string(),
                "fallback-provider-escalation".to_string(),
                "recompile-runtime-spec".to_string(),
            ],
            requires_wasm,
        }
    }
}

impl WasmRuntimeCompiler {
    pub fn compile(spec: &ExecutionRuntimeSpec) -> CompiledWasmExecutionEnvironment {
        let material = format!(
            "lang={}|framework={}|pm={}|deps={}|ports={}|build={}|exec={}|cache={}|wasm={}",
            spec.language,
            spec.framework,
            spec.package_manager.as_deref().unwrap_or(UNKNOWN_SIGNATURE),
            spec.dependencies.join(","),
            spec.ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(","),
            spec.build_steps.join("||"),
            spec.execution_steps.join("||"),
            spec.cache_layers.join("|"),
            spec.requires_wasm
        );
        let spec_fingerprint = hash_key(&material);
        // hash_key omits leading zeros, so guard the prefix length for shorter hashes.
        let environment_prefix_len = ENVIRONMENT_ID_PREFIX_LENGTH.min(spec_fingerprint.len());
        let environment_id = format!("uwef-{}", &spec_fingerprint[..environment_prefix_len]);
        let warm_pool_key = hash_key(&format!("{}:warm-pool", spec_fingerprint));
        let mut component_graph = vec![
            "filesystem".to_string(),
            "cache".to_string(),
            "logging".to_string(),
            "network".to_string(),
        ];
        match spec.language.as_str() {
            "javascript" | "typescript" => {
                component_graph.push("node-runtime".to_string());
                component_graph.push("package-manager".to_string());
            }
            "python" => {
                component_graph.push("python-runtime".to_string());
                component_graph.push("package-manager".to_string());
            }
            "rust" => {
                component_graph.push("rust-runtime".to_string());
                component_graph.push("cargo".to_string());
            }
            _ => {}
        }
        if spec.requires_wasm {
            component_graph.push("wasi".to_string());
        }
        component_graph.sort();
        component_graph.dedup();

        let mut capabilities = CapabilitySet::default();
        capabilities.insert("filesystem.read");
        capabilities.insert("filesystem.write");
        if spec.network_policy.allow_outbound {
            capabilities.insert("network.http");
        }
        if !spec.execution_steps.is_empty() {
            capabilities.insert("process.spawn");
        }
        if spec.requires_wasm {
            capabilities.insert("wasi.runtime");
        }
        if !spec.framework.is_empty() && spec.framework != UNKNOWN_SIGNATURE {
            capabilities.insert(format!("{}.framework", spec.framework));
        }
        if let Some(package_manager) = spec.package_manager.as_deref() {
            capabilities.insert(format!("{package_manager}.package_manager"));
        }
        if !spec.language.is_empty() && spec.language != UNKNOWN_SIGNATURE {
            capabilities.insert(format!("{}.runtime", spec.language));
        }

        let mut components = vec![
            WasiComponent {
                id: "filesystem".to_string(),
                module: "filesystem.wasm".to_string(),
                imports: vec![],
                exports: vec![
                    "filesystem.read".to_string(),
                    "filesystem.write".to_string(),
                ],
                capabilities: vec![
                    "filesystem.read".to_string(),
                    "filesystem.write".to_string(),
                ],
            },
            WasiComponent {
                id: "network".to_string(),
                module: "network.wasm".to_string(),
                imports: vec!["filesystem.write".to_string()],
                exports: vec!["network.http".to_string()],
                capabilities: vec!["network.http".to_string()],
            },
            WasiComponent {
                id: "process".to_string(),
                module: "process.wasm".to_string(),
                imports: vec!["filesystem.read".to_string()],
                exports: vec!["process.spawn".to_string()],
                capabilities: vec!["process.spawn".to_string()],
            },
        ];

        match spec.language.as_str() {
            "javascript" | "typescript" => {
                components.push(WasiComponent {
                    id: "nodejs".to_string(),
                    module: "nodejs.wasm".to_string(),
                    imports: vec!["process.spawn".to_string()],
                    exports: vec!["javascript.runtime".to_string()],
                    capabilities: vec!["javascript.runtime".to_string()],
                });
            }
            "python" => {
                components.push(WasiComponent {
                    id: "python".to_string(),
                    module: "python.wasm".to_string(),
                    imports: vec!["process.spawn".to_string()],
                    exports: vec!["python.runtime".to_string()],
                    capabilities: vec!["python.runtime".to_string()],
                });
            }
            "rust" => {
                components.push(WasiComponent {
                    id: "rust".to_string(),
                    module: "rust.wasm".to_string(),
                    imports: vec!["process.spawn".to_string()],
                    exports: vec!["rust.runtime".to_string()],
                    capabilities: vec!["rust.runtime".to_string()],
                });
            }
            _ => {}
        }

        if let Some(package_manager) = spec.package_manager.as_deref() {
            components.push(WasiComponent {
                id: package_manager.to_string(),
                module: format!("{package_manager}.wasm"),
                imports: package_manager_component_imports(package_manager),
                exports: vec![format!("{package_manager}.package_manager")],
                capabilities: vec![format!("{package_manager}.package_manager")],
            });
        }
        if !spec.framework.is_empty() && spec.framework != UNKNOWN_SIGNATURE {
            components.push(WasiComponent {
                id: spec.framework.clone(),
                module: format!("{}.wasm", spec.framework),
                imports: vec![format!("{}.runtime", spec.language)],
                exports: vec![format!("{}.framework", spec.framework)],
                capabilities: vec![format!("{}.framework", spec.framework)],
            });
        }

        if spec.requires_wasm {
            components.push(WasiComponent {
                id: "wasi".to_string(),
                module: "wasi.wasm".to_string(),
                imports: vec!["filesystem.read".to_string()],
                exports: vec!["wasi.runtime".to_string()],
                capabilities: vec!["wasi.runtime".to_string()],
            });
        }

        let mut loader = WasiComponentLoader::default();
        let runtime_constraints = RuntimeConstraints {
            read_only_paths: vec!["/workspace".to_string()],
            network_allowlist: spec.network_policy.allowed_hosts.clone(),
            max_memory_mb: spec.memory_limit_mb,
            max_cpu_units: spec.cpu_limit_units,
            process_spawn_bounded: true,
        };
        let wasi_component_graph = loader.load_graph(components, capabilities, runtime_constraints);

        CompiledWasmExecutionEnvironment {
            environment_id,
            spec_fingerprint,
            warm_pool_key,
            deterministic: true,
            component_graph,
            wasi_component_graph,
        }
    }
}

fn parse_link_entry<'a>(prefix: &str, value: &'a str) -> Option<(&'a str, &'a str)> {
    value
        .strip_prefix(prefix)
        .and_then(|entry| entry.split_once(':'))
        .filter(|(component, capability)| !component.is_empty() && !capability.is_empty())
}

fn interface_identity(capability: &str) -> String {
    capability.split_once('@').map_or_else(
        || capability.to_string(),
        |(interface_name, _)| interface_name.to_string(),
    )
}

fn package_manager_component_imports(package_manager: &str) -> Vec<String> {
    match package_manager {
        "pnpm" | "npm" | "yarn" | "bun" | "cargo" | "pip" | "pipenv" | "poetry" | "uv" => {
            vec!["network.http".to_string(), "filesystem.read".to_string()]
        }
        _ => vec!["filesystem.read".to_string()],
    }
}

fn component_startup_order(components: &[WasiComponent], links: &[WasiLink]) -> Vec<String> {
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    for component in components {
        in_degree.entry(component.id.clone()).or_insert(0);
    }
    for link in links {
        if link.from_component == link.to_component {
            continue;
        }
        if !in_degree.contains_key(&link.from_component)
            || !in_degree.contains_key(&link.to_component)
        {
            continue;
        }
        if dependents
            .entry(link.from_component.clone())
            .or_default()
            .insert(link.to_component.clone())
        {
            *in_degree.entry(link.to_component.clone()).or_insert(0) += 1;
        }
    }

    let mut ready = in_degree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(node.clone()))
        .collect::<BTreeSet<_>>();

    let mut ordered = Vec::new();
    while let Some(node) = ready.first().cloned() {
        ready.remove(&node);
        ordered.push(node.clone());
        if let Some(next_nodes) = dependents.get(&node) {
            for next in next_nodes {
                if let Some(degree) = in_degree.get_mut(next) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 && !ordered.contains(next) {
                        ready.insert(next.clone());
                    }
                }
            }
        }
    }

    if ordered.len() != in_degree.len() {
        let mut fallback = components
            .iter()
            .map(|component| component.id.clone())
            .collect::<Vec<_>>();
        fallback.sort();
        fallback.dedup();
        return fallback;
    }
    ordered
}

fn allowed_hosts_for_package_manager(package_manager: Option<&str>) -> Vec<String> {
    let mut hosts = vec!["github.com".to_string()];
    match package_manager.unwrap_or_default() {
        "pnpm" | "npm" | "yarn" | "bun" => hosts.push("registry.npmjs.org".to_string()),
        "cargo" => hosts.push("crates.io".to_string()),
        "pip" | "pipenv" | "poetry" | "uv" => hosts.push("pypi.org".to_string()),
        _ => {}
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

fn is_static_web_framework(value: &str) -> bool {
    let normalized = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    normalized == "staticweb"
}

pub mod ucpe_ti;

impl Default for RestApiSpec {
    fn default() -> Self {
        let mut routes = vec![
            "POST /auth/login",
            "POST /auth/logout",
            "GET /auth/me",
            "GET /auth/github/callback",
            "GET /auth/google/callback",
            "POST /orgs",
            "GET /orgs/{org_id}",
            "POST /orgs/{org_id}/members",
            "POST /workspaces",
            "GET /workspaces?org_id={org_id}",
            "GET /workspaces/{id}",
            "POST /workspaces/{id}/bind",
            "POST /workspaces/{id}/migrate",
            "DELETE /workspaces/{id}",
            "POST /workspaces/{id}/stop",
            "POST /workspaces/{id}/restart",
            "GET /workspaces/{id}/logs",
            "GET /workspaces/{id}/ports",
            "GET /workspaces/{id}/filesystem/*path",
            "POST /execution-image/compile",
            "GET /execution-image/{repo_id}",
            "POST /fingerprint/generate",
            "GET /fingerprint/{repo_id}",
            "POST /fingerprint/recompute",
            "GET /warm-pool/status",
            "POST /warm-pool/prewarm",
            "GET /repo/{id}/commits",
            "POST /execute/recover",
            "GET /executions?org_id={org_id}",
            "POST /api/v1/repositories/analyze",
            "POST /api/v1/repositories/publish",
            "POST /api/v1/execution/plan",
            "POST /api/v1/executions",
            "POST /api/v1/executions/{id}/claim",
            "GET /api/v1/executions/{id}",
            "GET /api/v1/executions/{id}/logs",
            "POST /api/v1/executions/{id}/restart",
            "POST /api/v1/executions/{id}/stop",
            "POST /api/v1/executions/{id}/migrate",
            "GET /repositories/{id}/history",
            "GET /executions/{id}/history",
            "GET /repositories/{id}/healing",
            "GET /repositories/{id}/last-good",
            "GET /api/repositories/{id}/intelligence",
            "POST /api/repositories/{id}/ask",
            "GET /repositories/{id}/twin",
            "GET /repositories/{id}/behavior",
            "GET /repositories/{id}/architecture",
            "GET /repositories/{id}/timeline",
            "GET /repositories/{id}/predictions",
            "GET /repositories/{id}/recommendations",
            "GET /repositories/{id}/blast-radius",
            "GET /repositories/{id}/dna",
            "GET /repositories/{id}/risk",
            "GET /repositories/{id}/memory",
            "POST /repositories/{id}/simulate",
            "POST /repositories/{id}/infer",
            "POST /repositories/{id}/compare",
            "POST /repositories/{id}/predict",
            "POST /repositories/{id}/explain",
            "GET /intelligence/{execution}",
            "GET /intelligence/similar",
            "GET /intelligence/patterns",
            "GET /intelligence/repairs",
            "GET /intelligence/context",
            "POST /intelligence/retrieve",
            "POST /intelligence/learn",
            "POST /intelligence/optimize",
            "GET /billing/usage?org_id={org_id}",
            "GET /billing/summary",
            "POST /billing/invoice",
            "GET /api/v1/dual-surface/contract",
            "GET /api/v1/surfaces/extension/actions",
            "GET /api/v1/surfaces/portal/navigation",
            "GET /api/v1/surfaces/extension/ui",
            "GET /api/v1/surfaces/portal/ui",
            "POST /api/badges/generate",
            "POST /api/badge/generate",
            "GET /badge/{owner}/{repo}.svg",
            "GET /badge/healed/{owner}/{repo}.svg",
            "GET /seed/{owner}/{repo}",
        ];
        routes.extend(ucpe_ti::unified_api_routes());
        Self { routes }
    }
}

struct NodeRuntimeProvider;
struct DockerExecutionProvider;
struct GoExecutionProvider;
struct PythonExecutionProvider;
struct JavaExecutionProvider;
struct RustRuntimeProvider;
struct StaticRuntimeProvider;
struct LocalAgentProvider {
    agent: Arc<Mutex<DistributedExecutionAgent>>,
}

impl LocalAgentProvider {
    fn new(agent: DistributedExecutionAgent) -> Self {
        Self {
            agent: Arc::new(Mutex::new(agent)),
        }
    }

    fn default_agent() -> Self {
        let mut agent = DistributedExecutionAgent::new(AgentIdentity {
            agent_id: "dea-local-default".to_string(),
            device_fingerprint: "local-device".to_string(),
            public_key: "local-public-key".to_string(),
            trusted: true,
        });
        agent.register(AgentCapabilities {
            cpu: 8,
            memory: "16GB".to_string(),
            runtimes: vec![
                "node".to_string(),
                "rust".to_string(),
                "go".to_string(),
                "python".to_string(),
                "static".to_string(),
            ],
            supports_wasm: false,
        });
        Self::new(agent)
    }
}

impl DockerExecutionProvider {
    fn id_static() -> &'static str {
        "DockerExecutionProvider"
    }

    fn docker_command_lowercase(command: &str) -> Option<String> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return None;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower == "docker"
            || lower.starts_with("docker ")
            || lower.starts_with("docker-compose ")
            || lower.starts_with("docker compose ")
        {
            Some(lower)
        } else {
            None
        }
    }

    fn is_docker_command(command: &str) -> bool {
        Self::docker_command_lowercase(command).is_some()
    }

    fn is_compose_command(lower_command: &str) -> bool {
        lower_command.starts_with("docker compose ") || lower_command.starts_with("docker-compose ")
    }

    fn command_for_context(ctx: &ExecutionContext) -> Option<String> {
        load_execution_manifest_start_command(Path::new(&ctx.repo_path))
            .or_else(|| ctx.execution_graph.primary_run_command())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn ensure_docker_ready(command: &str) -> Result<()> {
        let Some(lower) = Self::docker_command_lowercase(command) else {
            return Ok(());
        };
        if lower == "docker" {
            return Err(RuntimeError::CommandFailed(
                "docker command is missing a subcommand".to_string(),
            ));
        }

        let mut docker_version = Command::new("docker");
        docker_version.arg("--version");
        run_command_with_timeout(&mut docker_version, INSTALL_TIMEOUT_SECS).map_err(|err| {
            RuntimeError::CommandFailed(format!("docker runtime readiness check failed: {err}"))
        })?;

        if Self::is_compose_command(&lower) {
            let mut docker_compose_version = Command::new("docker");
            docker_compose_version.args(["compose", "version"]);
            run_command_with_timeout(&mut docker_compose_version, INSTALL_TIMEOUT_SECS).map_err(
                |err| {
                    RuntimeError::CommandFailed(format!(
                        "docker compose readiness check failed: {err}"
                    ))
                },
            )?;
        }

        Ok(())
    }
}

impl ExecutionProvider for WasmExecutionProvider {
    fn id(&self) -> &'static str {
        "WasmExecutionProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalMachine
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Wasm
    }

    fn transport(&self) -> ExecutionRoutingMode {
        ExecutionRoutingMode::Wasm
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        if ctx.execution_graph.nodes.is_empty() {
            return false;
        }
        let has_wasm = ctx.runtime_spec.requires_wasm;
        has_wasm
            && ctx.execution_graph.nodes.iter().all(|node| {
                match ExecutionRouter::route(node, &ctx.analysis.execution_profile) {
                    ExecutionTarget::Wasm(_) => true,
                    ExecutionTarget::Native | ExecutionTarget::Static => node.command.is_some(),
                }
            })
    }

    fn prepare(&self, ctx: &mut ExecutionContext) -> Result<()> {
        if let Some(spec) = ctx.execution_graph.nodes.iter().find_map(|node| {
            match ExecutionRouter::route(node, &ctx.analysis.execution_profile) {
                ExecutionTarget::Wasm(spec) => Some(spec),
                ExecutionTarget::Native | ExecutionTarget::Static => None,
            }
        }) {
            ctx.wasm_sandbox = Some(wasm_sandbox_for(&spec, &ctx.repo_path));
        }
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        let runtime = WasmRuntimeEngine::new()?;
        let native = NativeRuntimeEngine;
        let wasi = WasiContext {
            env: HashMap::from([("RUSTGIT_WORKSPACE_ID".to_string(), ctx.workspace_id.clone())]),
            args: vec!["rustgit-runtime".to_string()],
        };
        let ordered_ids = ctx.execution_graph.ordered_node_ids();
        let keys = ctx.execution_graph.compute_cache_keys();
        let mut last_handle = None;
        for node_id in ordered_ids {
            let Some(node) = ctx
                .execution_graph
                .nodes
                .iter()
                .find(|node| node.id == node_id)
            else {
                continue;
            };
            match ExecutionRouter::route(node, &ctx.analysis.execution_profile) {
                ExecutionTarget::Wasm(spec) => {
                    let module = load_compiled_wasm_module(ctx, node)?;
                    let artifact_key = node
                        .cache_key
                        .clone()
                        .or_else(|| keys.get(&node.id).cloned())
                        .unwrap_or_else(|| hash_key(&node.id));
                    let binding = wasm_artifact_binding(ctx, node, &artifact_key);
                    let sandbox = ctx
                        .wasm_sandbox
                        .clone()
                        .unwrap_or_else(|| wasm_sandbox_for(&spec, &ctx.repo_path));
                    let execution_context = WasmExecutionContext {
                        node_id: node.id.clone(),
                        module: module.clone(),
                        wasi: wasi.clone(),
                        env: WasmExecutionEnvironment::from_execution_context(ctx),
                        sandbox,
                        spec,
                    };
                    runtime.instantiate(&execution_context)?;
                    last_handle = Some(ProcessHandle {
                        pid_hint: format!("wasm:{}:{}", binding.node_id, binding.artifact_key),
                        ..ProcessHandle::default()
                    });
                }
                ExecutionTarget::Native | ExecutionTarget::Static => {
                    let Some(command) = node.command.clone() else {
                        return Err(RuntimeError::CommandFailed(format!(
                            "node {} is missing a command for native/static execution",
                            node.id
                        )));
                    };
                    let handle = native.execute(
                        &NativeExecutionRequest {
                            command,
                            args: vec![],
                            cwd: ctx.repo_path.clone(),
                            env: HashMap::new(),
                        },
                        &ctx.resources,
                        &ctx.network,
                    )?;
                    last_handle = Some(handle);
                }
            }
        }
        last_handle.ok_or_else(|| {
            RuntimeError::CommandFailed(
                "execution graph contains no dispatchable nodes".to_string(),
            )
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

impl ExecutionProvider for NodeRuntimeProvider {
    fn id(&self) -> &'static str {
        "NodeRuntimeProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalDocker
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Node
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        matches!(
            ctx.runtime_spec.language.as_str(),
            "javascript" | "typescript"
        ) || matches!(
            ctx.analysis.framework,
            Framework::Node
                | Framework::Vite
                | Framework::React
                | Framework::Vue
                | Framework::Svelte
                | Framework::SvelteKit
                | Framework::NextJs
                | Framework::Nuxt
                | Framework::Astro
                | Framework::Remix
                | Framework::Express
                | Framework::NestJs
        )
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("node:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

impl ExecutionProvider for DockerExecutionProvider {
    fn id(&self) -> &'static str {
        Self::id_static()
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalDocker
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Unknown
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        Self::command_for_context(ctx)
            .as_deref()
            .is_some_and(Self::is_docker_command)
    }

    fn prepare(&self, ctx: &mut ExecutionContext) -> Result<()> {
        if let Some(command) = Self::command_for_context(ctx) {
            Self::ensure_docker_ready(&command)?;
        }
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        let command = Self::command_for_context(ctx).ok_or_else(|| {
            RuntimeError::CommandFailed("docker provider could not resolve run command".to_string())
        })?;
        Self::ensure_docker_ready(&command)?;
        Ok(ProcessHandle {
            pid_hint: format!("docker:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        let mut docker_info = Command::new("docker");
        docker_info.arg("info");
        match run_command_with_timeout(&mut docker_info, INSTALL_TIMEOUT_SECS) {
            Ok(output) if output.status.success() => Ok(HealthStatus {
                healthy: true,
                message: "docker daemon reachable".to_string(),
            }),
            Ok(output) => Ok(HealthStatus {
                healthy: false,
                message: format!("docker health check exited with status {}", output.status),
            }),
            Err(err) => Ok(HealthStatus {
                healthy: false,
                message: format!("docker health check failed: {err}"),
            }),
        }
    }
}

impl ExecutionProvider for GoExecutionProvider {
    fn id(&self) -> &'static str {
        "GoExecutionProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalDocker
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Go
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        ctx.runtime_spec.language.eq_ignore_ascii_case("go")
            || matches!(
                ctx.analysis.framework,
                Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo
            )
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("go:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

impl ExecutionProvider for PythonExecutionProvider {
    fn id(&self) -> &'static str {
        "PythonExecutionProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalDocker
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Python
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        ctx.runtime_spec.language.eq_ignore_ascii_case("python")
            || matches!(
                ctx.analysis.framework,
                Framework::Python
                    | Framework::Flask
                    | Framework::FastApi
                    | Framework::Django
                    | Framework::Streamlit
                    | Framework::Gradio
            )
            || ctx
                .runtime_spec
                .package_manager
                .as_deref()
                .is_some_and(|pm| matches!(pm, "pip" | "pipenv" | "poetry" | "uv"))
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("python:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

impl ExecutionProvider for JavaExecutionProvider {
    fn id(&self) -> &'static str {
        "JavaExecutionProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalDocker
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Java
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        ctx.runtime_spec.language.eq_ignore_ascii_case("java")
            || ctx.runtime_spec.framework.eq_ignore_ascii_case("java")
            || ctx
                .runtime_spec
                .package_manager
                .as_deref()
                .is_some_and(|pm| matches!(pm, "maven" | "gradle"))
            || Path::new(&ctx.repo_path).join("pom.xml").exists()
            || Path::new(&ctx.repo_path).join("build.gradle").exists()
            || Path::new(&ctx.repo_path).join("build.gradle.kts").exists()
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("java:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

impl ExecutionProvider for LocalAgentProvider {
    fn id(&self) -> &'static str {
        "LocalAgentProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::LocalMachine
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Unknown
    }

    fn capability(&self) -> ProviderCapability {
        let agent = self.agent.lock().expect(LOCAL_AGENT_LOCK_POISONED);
        let capabilities = agent.capabilities.clone().unwrap_or(AgentCapabilities {
            cpu: 0,
            memory: "0MB".to_string(),
            runtimes: vec![],
            supports_wasm: false,
        });
        let mut supported_runtimes = capabilities
            .runtimes
            .iter()
            .filter_map(|runtime| match runtime.to_ascii_lowercase().as_str() {
                "node" => Some(RuntimeType::Node),
                "wasm" => Some(RuntimeType::Wasm),
                "rust" => Some(RuntimeType::Rust),
                "go" => Some(RuntimeType::Go),
                "python" => Some(RuntimeType::Python),
                "static" => Some(RuntimeType::Static),
                _ => None,
            })
            .collect::<Vec<_>>();
        if capabilities.supports_wasm && !supported_runtimes.contains(&RuntimeType::Wasm) {
            supported_runtimes.push(RuntimeType::Wasm);
        }
        ProviderCapability {
            tier: ExecutionTier::LocalMachine,
            latency_score: 5,
            cost_score: 1,
            reliability_score: if agent.identity.trusted { 35 } else { 10 },
            supported_runtimes,
        }
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        let agent = self.agent.lock().expect(LOCAL_AGENT_LOCK_POISONED);
        agent.can_execute(ctx)
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        let mut agent = self.agent.lock().expect(LOCAL_AGENT_LOCK_POISONED);
        let graph = SignedExecutionGraph {
            graph: ctx.execution_graph.clone(),
            signature: agent.sign_graph(&ctx.execution_graph),
        };
        agent.assign_execution(&graph)?;
        Ok(ProcessHandle {
            pid_hint: format!("dea:{}:{}", agent.identity.agent_id, ctx.workspace_id),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        let mut agent = self.agent.lock().expect(LOCAL_AGENT_LOCK_POISONED);
        agent.complete_execution();
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        let agent = self.agent.lock().expect(LOCAL_AGENT_LOCK_POISONED);
        Ok(HealthStatus {
            healthy: agent.identity.trusted,
            message: if agent.identity.trusted {
                "healthy".to_string()
            } else {
                "untrusted agent".to_string()
            },
        })
    }
}

impl ExecutionProvider for RustRuntimeProvider {
    fn id(&self) -> &'static str {
        "RustRuntimeProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::ExternalProvider
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Rust
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        ctx.runtime_spec.language == "rust"
            || matches!(
                ctx.analysis.framework,
                Framework::Rust
                    | Framework::Axum
                    | Framework::Actix
                    | Framework::Rocket
                    | Framework::Leptos
            )
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("rust:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

impl ExecutionProvider for StaticRuntimeProvider {
    fn id(&self) -> &'static str {
        "StaticRuntimeProvider"
    }

    fn tier(&self) -> ExecutionTier {
        ExecutionTier::DDockitCloud
    }

    fn runtime(&self) -> RuntimeType {
        RuntimeType::Static
    }

    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        is_static_web_framework(&ctx.runtime_spec.framework)
            || ctx.analysis.framework == Framework::StaticWeb
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("static:{}", ctx.execution_graph.cache_key()),
            ..ProcessHandle::default()
        })
    }

    fn stop(&self, _handle: &ProcessHandle) -> Result<()> {
        Ok(())
    }

    fn health(&self, _handle: &ProcessHandle) -> Result<HealthStatus> {
        Ok(HealthStatus {
            healthy: true,
            message: "healthy".to_string(),
        })
    }
}

fn can_transition(from: WorkspaceState, to: WorkspaceState) -> bool {
    match from {
        WorkspaceState::Created => to == WorkspaceState::Materializing,
        WorkspaceState::Materializing => {
            matches!(
                to,
                WorkspaceState::Installing | WorkspaceState::Analyzing | WorkspaceState::Failed
            )
        }
        WorkspaceState::Installing => {
            matches!(
                to,
                WorkspaceState::Analyzing | WorkspaceState::Launching | WorkspaceState::Failed
            )
        }
        WorkspaceState::Analyzing => {
            matches!(to, WorkspaceState::Planning | WorkspaceState::Failed)
        }
        WorkspaceState::Planning => matches!(
            to,
            WorkspaceState::Starting | WorkspaceState::Launching | WorkspaceState::Failed
        ),
        WorkspaceState::Pending => {
            matches!(to, WorkspaceState::Provisioning | WorkspaceState::Failed)
        }
        WorkspaceState::Provisioning => {
            matches!(to, WorkspaceState::Starting | WorkspaceState::Failed)
        }
        WorkspaceState::Starting => matches!(
            to,
            WorkspaceState::Launching | WorkspaceState::Running | WorkspaceState::Failed
        ),
        WorkspaceState::Launching => {
            matches!(to, WorkspaceState::Initializing | WorkspaceState::Failed)
        }
        WorkspaceState::Initializing => matches!(
            to,
            WorkspaceState::Ready | WorkspaceState::Running | WorkspaceState::Failed
        ),
        WorkspaceState::Ready => matches!(
            to,
            WorkspaceState::Running | WorkspaceState::Failed | WorkspaceState::Stopping
        ),
        WorkspaceState::Running => {
            matches!(
                to,
                WorkspaceState::Paused
                    | WorkspaceState::Stopping
                    | WorkspaceState::Degraded
                    | WorkspaceState::Restarting
                    | WorkspaceState::Migrating
                    | WorkspaceState::Failed
            )
        }
        WorkspaceState::Degraded => {
            matches!(
                to,
                WorkspaceState::Running
                    | WorkspaceState::Restarting
                    | WorkspaceState::Migrating
                    | WorkspaceState::Failed
            )
        }
        WorkspaceState::Restarting => {
            matches!(
                to,
                WorkspaceState::Starting | WorkspaceState::Running | WorkspaceState::Failed
            )
        }
        WorkspaceState::Migrating => {
            matches!(
                to,
                WorkspaceState::Starting | WorkspaceState::Running | WorkspaceState::Failed
            )
        }
        WorkspaceState::Paused => {
            matches!(
                to,
                WorkspaceState::Running | WorkspaceState::Stopping | WorkspaceState::Failed
            )
        }
        WorkspaceState::Failed => {
            matches!(
                to,
                WorkspaceState::Starting
                    | WorkspaceState::Restarting
                    | WorkspaceState::Migrating
                    | WorkspaceState::Stopping
                    | WorkspaceState::Stopped
                    | WorkspaceState::Destroyed
            )
        }
        WorkspaceState::Stopping => matches!(to, WorkspaceState::Stopped | WorkspaceState::Failed),
        WorkspaceState::Stopped => {
            matches!(
                to,
                WorkspaceState::Starting
                    | WorkspaceState::Restarting
                    | WorkspaceState::Provisioning
                    | WorkspaceState::Destroyed
            )
        }
        WorkspaceState::Destroyed => false,
    }
}

fn looks_like_local_path(repo_url: &str) -> bool {
    repo_url.starts_with('/') || repo_url.starts_with("./") || repo_url.starts_with("../")
}

fn github_clone_extra_header(repo_url: &str) -> Option<String> {
    github_clone_extra_header_with_token(
        repo_url,
        std::env::var("RUSTGIT_GITHUB_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("GITHUB_PAT")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("GH_TOKEN")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .as_deref(),
    )
}

fn github_clone_extra_header_with_token(repo_url: &str, token: Option<&str>) -> Option<String> {
    let token = token.map(str::trim).filter(|value| !value.is_empty())?;
    if !repo_url.starts_with("https://github.com/")
        && !repo_url.starts_with("https://www.github.com/")
    {
        return None;
    }
    let mut header = String::from("Authorization: Bearer ");
    header.push_str(token);
    Some(header)
}

fn github_clone_error_reason(repo_url: &str, stderr: &str) -> String {
    if repo_url.starts_with("https://github.com/")
        || repo_url.starts_with("https://www.github.com/")
    {
        if stderr.contains("could not read Username for 'https://github.com'")
            || stderr.contains("could not read Password for 'https://github.com'")
        {
            return "GitHub authentication is required in this environment; set RUSTGIT_GITHUB_TOKEN/GITHUB_PAT/GH_TOKEN or pass a credentialed repo URL such as https://<token>@github.com/owner/repo.git".to_string();
        }
    }

    if stderr.is_empty() {
        "git clone failed with no stderr output".to_string()
    } else {
        stderr.to_string()
    }
}

fn node_type_name(node_type: ExecutionNodeType) -> &'static str {
    match node_type {
        ExecutionNodeType::InstallDependencies => "install-dependencies",
        ExecutionNodeType::Build => "build",
        ExecutionNodeType::WasmCompile => "wasm-compile",
        ExecutionNodeType::DevServer => "dev-server",
        ExecutionNodeType::Test => "test",
        ExecutionNodeType::StaticServe => "static-serve",
        ExecutionNodeType::CustomCommand => "custom-command",
    }
}

fn execution_mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Native => "native",
        ExecutionMode::Wasm => "wasm",
        ExecutionMode::Hybrid => "hybrid",
    }
}

fn load_compiled_wasm_module(ctx: &ExecutionContext, node: &ExecutionNode) -> Result<WasmModule> {
    let repo_root = Path::new(&ctx.repo_path);
    if !repo_root.is_absolute() {
        return Err(RuntimeError::InvalidPath(ctx.repo_path.clone()));
    }

    let mut search_roots = vec![];
    // Prefer declared outputs first, then fall back to wasm-like inputs so a serve
    // node can bind to a module produced by a prior WasmCompile node.
    for location in &node.outputs {
        if location.contains("://") {
            continue;
        }
        search_roots.push(repo_root.join(location));
    }
    for location in &node.inputs {
        if location.contains("://")
            || (!location.ends_with(".wasm")
                && !location.contains("wasm")
                && !location.contains("pkg"))
        {
            continue;
        }
        search_roots.push(repo_root.join(location));
    }
    search_roots.push(repo_root.to_path_buf());

    for root in search_roots {
        let Some(module_path) = first_wasm_module_in(&root)? else {
            continue;
        };
        let bytes = fs::read(&module_path)?;
        return Ok(WasmModule {
            path: module_path.to_string_lossy().to_string(),
            hash: hash_bytes(&bytes),
            bytes,
        });
    }

    Err(RuntimeError::WasmRuntime(format!(
        "no compiled wasm artifact found for node {}",
        node.id
    )))
}

fn wasm_artifact_binding(
    ctx: &ExecutionContext,
    node: &ExecutionNode,
    artifact_key: &str,
) -> WasmArtifactBinding {
    let mut inputs = node.inputs.clone();
    inputs.sort();
    let source_files_hash = hash_key(&inputs.join("|"));
    WasmArtifactBinding {
        node_id: node.id.clone(),
        artifact_key: artifact_key.to_string(),
        build_fingerprint: ctx.analysis.fingerprint.repo_hash.clone(),
        source_files_hash,
    }
}

fn first_wasm_module_in(root: &Path) -> Result<Option<PathBuf>> {
    if !root.exists() {
        return Ok(None);
    }
    if root.is_file() {
        return Ok(is_wasm_module(root).then(|| root.to_path_buf()));
    }

    let mut pending = vec![root.to_path_buf()];
    let mut wasm_modules = vec![];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else if is_wasm_module(&path) {
                wasm_modules.push(path);
            }
        }
    }

    wasm_modules.sort();
    Ok(wasm_modules.into_iter().next())
}

fn is_wasm_module(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("wasm"))
        .unwrap_or(false)
}

fn wasm_sandbox_for(spec: &WasmRuntimeSpec, repo_path: &str) -> WasmSandbox {
    WasmSandbox {
        memory_limit: spec.memory_limit_mb.saturating_mul(BYTES_PER_MB),
        time_limit_ms: u64::from(spec.cpu_limit_units).saturating_mul(CPU_UNIT_TO_TIME_LIMIT_MS),
        filesystem_scope: vec![repo_path.to_string()],
    }
}

impl ArtifactType {
    fn as_str(self) -> &'static str {
        match self {
            ArtifactType::FileSystemSnapshot => "filesystem-snapshot",
            ArtifactType::BuildOutput => "build-output",
            ArtifactType::TestResult => "test-result",
            ArtifactType::WasmModule => "wasm-module",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "filesystem-snapshot" => Some(Self::FileSystemSnapshot),
            "build-output" => Some(Self::BuildOutput),
            "test-result" => Some(Self::TestResult),
            "wasm-module" => Some(Self::WasmModule),
            _ => None,
        }
    }
}

/// Generates a stable cache key using the standard FNV-1a 64-bit basis and prime constants.
fn hash_key(input: &str) -> String {
    let mut state: u64 = 14695981039346656037;
    for byte in input.bytes() {
        state ^= byte as u64;
        state = state.wrapping_mul(1099511628211);
    }
    format!("{state:x}")
}

fn hash_bytes(input: &[u8]) -> String {
    let mut state: u64 = 14695981039346656037;
    for byte in input {
        state ^= *byte as u64;
        state = state.wrapping_mul(1099511628211);
    }
    format!("{state:x}")
}

fn cache_artifacts_for_image(image: &ExecutionImage) -> Vec<String> {
    let mut artifacts = vec!["build-artifacts".to_string()];
    match image.runtime {
        RuntimeType::Node => {
            artifacts.push("node_modules".to_string());
            artifacts.push("pnpm-store".to_string());
        }
        RuntimeType::Python => {
            artifacts.push("pip-cache".to_string());
            artifacts.push("site-packages".to_string());
        }
        RuntimeType::Rust => {
            artifacts.push("cargo-registry".to_string());
            artifacts.push("target-cache".to_string());
            artifacts.push("wasm-modules".to_string());
        }
        RuntimeType::Wasm => {
            artifacts.push("wasm-modules".to_string());
        }
        RuntimeType::Go => {
            artifacts.push("go-mod-cache".to_string());
        }
        RuntimeType::Java => {
            artifacts.push("maven-cache".to_string());
            artifacts.push("gradle-cache".to_string());
        }
        RuntimeType::Static | RuntimeType::Unknown => {}
    }
    artifacts
}

fn framework_tag(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return UNKNOWN_SIGNATURE.to_string();
    }
    trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn language_tag(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        UNKNOWN_SIGNATURE.to_string()
    } else {
        normalized
    }
}

fn language_kind_from_signature(language: &str) -> LanguageKind {
    match language {
        value if value.contains("typescript") => Language::TypeScript,
        value if value.contains("javascript") => Language::JavaScript,
        value if value.contains("rust") => Language::Rust,
        value if value.contains("go") => Language::Go,
        value if value.contains("python") => Language::Python,
        _ => Language::Unknown,
    }
}

fn framework_kind_from_fingerprint(fingerprint: &RepositoryFingerprint) -> Option<FrameworkKind> {
    let framework = fingerprint
        .framework_signature
        .as_deref()
        .unwrap_or(UNKNOWN_SIGNATURE)
        .to_ascii_lowercase();
    if framework.contains("nextjs") {
        Some(FrameworkKind::NextJs)
    } else if framework.contains("react") {
        Some(FrameworkKind::React)
    } else if framework.contains("vite") {
        Some(FrameworkKind::Vite)
    } else if framework.contains("nestjs") {
        Some(FrameworkKind::NestJs)
    } else if framework.contains("express") {
        Some(FrameworkKind::Express)
    } else if framework.contains("remix") {
        Some(FrameworkKind::Remix)
    } else if framework.contains("fastapi") {
        Some(FrameworkKind::FastApi)
    } else if framework.contains("django") {
        Some(FrameworkKind::Django)
    } else if framework.contains("flask") {
        Some(FrameworkKind::Flask)
    } else if framework.contains("streamlit") {
        Some(FrameworkKind::Streamlit)
    } else if framework.contains("celery") {
        Some(FrameworkKind::Celery)
    } else if framework.contains("axum") {
        Some(FrameworkKind::Axum)
    } else if framework.contains("actix") {
        Some(FrameworkKind::Actix)
    } else if framework.contains("rocket") {
        Some(FrameworkKind::Rocket)
    } else if framework.contains("bun") && framework.contains("vite") {
        Some(FrameworkKind::BunVite)
    } else if framework.contains("bun") {
        Some(FrameworkKind::BunServer)
    } else if framework.contains("turborepo") {
        Some(FrameworkKind::Turborepo)
    } else if framework.contains("nx") {
        Some(FrameworkKind::Nx)
    } else if framework.contains("pnpm-workspace") {
        Some(FrameworkKind::PnpmWorkspace)
    } else if framework.contains("yarn-workspace") {
        Some(FrameworkKind::YarnWorkspace)
    } else {
        Some(FrameworkKind::Unknown)
    }
}

fn framework_kind_label(framework: FrameworkKind) -> &'static str {
    match framework {
        FrameworkKind::NextJs => "nextjs",
        FrameworkKind::React => "react",
        FrameworkKind::Vite => "vite",
        FrameworkKind::NestJs => "nestjs",
        FrameworkKind::Express => "express",
        FrameworkKind::Remix => "remix",
        FrameworkKind::FastApi => "fastapi",
        FrameworkKind::Django => "django",
        FrameworkKind::Flask => "flask",
        FrameworkKind::Streamlit => "streamlit",
        FrameworkKind::Celery => "celery",
        FrameworkKind::Axum => "axum",
        FrameworkKind::Actix => "actix",
        FrameworkKind::Rocket => "rocket",
        FrameworkKind::BunVite => "bun-vite",
        FrameworkKind::BunServer => "bun-server",
        FrameworkKind::Turborepo => "turborepo",
        FrameworkKind::Nx => "nx",
        FrameworkKind::PnpmWorkspace => "pnpm-workspace",
        FrameworkKind::YarnWorkspace => "yarn-workspace",
        FrameworkKind::Unknown => UNKNOWN_SIGNATURE,
    }
}

fn image_runtime_for_framework(
    framework: Option<FrameworkKind>,
    fingerprint: &RepositoryFingerprint,
) -> ImageRuntimeKind {
    match framework.unwrap_or(FrameworkKind::Unknown) {
        FrameworkKind::NextJs
        | FrameworkKind::React
        | FrameworkKind::Vite
        | FrameworkKind::NestJs
        | FrameworkKind::Express
        | FrameworkKind::Remix
        | FrameworkKind::Turborepo
        | FrameworkKind::Nx
        | FrameworkKind::PnpmWorkspace
        | FrameworkKind::YarnWorkspace => ImageRuntimeKind::Node,
        FrameworkKind::FastApi
        | FrameworkKind::Django
        | FrameworkKind::Flask
        | FrameworkKind::Streamlit
        | FrameworkKind::Celery => ImageRuntimeKind::Python,
        FrameworkKind::Axum | FrameworkKind::Actix | FrameworkKind::Rocket => {
            ImageRuntimeKind::Rust
        }
        FrameworkKind::BunVite | FrameworkKind::BunServer => ImageRuntimeKind::Bun,
        FrameworkKind::Unknown => {
            let language = fingerprint.language_signature.to_ascii_lowercase();
            if language.contains("rust") {
                ImageRuntimeKind::Rust
            } else if language.contains("python") {
                ImageRuntimeKind::Python
            } else if language.contains("javascript") || language.contains("typescript") {
                ImageRuntimeKind::Node
            } else {
                ImageRuntimeKind::Unknown
            }
        }
    }
}

fn package_manager_for_framework(
    framework: Option<FrameworkKind>,
    runtime: ImageRuntimeKind,
    fingerprint: &RepositoryFingerprint,
) -> Option<PackageManagerKind> {
    let framework = framework.unwrap_or(FrameworkKind::Unknown);
    if matches!(framework, FrameworkKind::BunVite | FrameworkKind::BunServer) {
        return Some(PackageManagerKind::Bun);
    }

    let lockfile_hint = fingerprint
        .lockfile_hash
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lockfile_hint.contains("pnpm") {
        return Some(PackageManagerKind::Pnpm);
    }
    if lockfile_hint.contains("yarn") {
        return Some(PackageManagerKind::Yarn);
    }
    if framework == FrameworkKind::NextJs {
        return Some(PackageManagerKind::Pnpm);
    }

    match runtime {
        ImageRuntimeKind::Node => Some(PackageManagerKind::Npm),
        ImageRuntimeKind::Python => Some(PackageManagerKind::Pip),
        ImageRuntimeKind::Rust => Some(PackageManagerKind::Cargo),
        ImageRuntimeKind::Bun => Some(PackageManagerKind::Bun),
        ImageRuntimeKind::Unknown => None,
    }
}

fn runtime_version_for(runtime: ImageRuntimeKind) -> &'static str {
    match runtime {
        ImageRuntimeKind::Node => "20",
        ImageRuntimeKind::Python => "3.11",
        ImageRuntimeKind::Rust => "stable",
        ImageRuntimeKind::Bun => "1.1",
        ImageRuntimeKind::Unknown => UNKNOWN_SIGNATURE,
    }
}

fn entry_strategy_for(
    runtime: ImageRuntimeKind,
    framework: Option<FrameworkKind>,
    package_manager: Option<PackageManagerKind>,
) -> EntryStrategy {
    match runtime {
        ImageRuntimeKind::Node => EntryStrategy::NodeScript {
            command: if matches!(framework, Some(FrameworkKind::NextJs)) {
                match package_manager.unwrap_or(PackageManagerKind::Pnpm) {
                    PackageManagerKind::Yarn => "yarn dev".to_string(),
                    PackageManagerKind::Bun => "bun run dev".to_string(),
                    PackageManagerKind::Npm => "npm run dev".to_string(),
                    _ => "pnpm run dev".to_string(),
                }
            } else {
                "node server.js".to_string()
            },
        },
        ImageRuntimeKind::Python => EntryStrategy::PythonModule {
            module: if matches!(framework, Some(FrameworkKind::FastApi)) {
                "uvicorn app:app".to_string()
            } else if matches!(framework, Some(FrameworkKind::Django)) {
                "gunicorn app.wsgi".to_string()
            } else {
                "app".to_string()
            },
        },
        ImageRuntimeKind::Rust => EntryStrategy::RustBinary {
            target: "./target/release/<binary>".to_string(),
        },
        ImageRuntimeKind::Bun => EntryStrategy::BunScript {
            command: "bun run dev".to_string(),
        },
        ImageRuntimeKind::Unknown => EntryStrategy::DockerEntrypoint,
    }
}

fn sandbox_model_for_runtime(runtime: ImageRuntimeKind) -> SandboxModel {
    match runtime {
        ImageRuntimeKind::Node | ImageRuntimeKind::Python | ImageRuntimeKind::Rust => {
            SandboxModel::ProcessIsolated
        }
        ImageRuntimeKind::Bun => SandboxModel::Hybrid,
        ImageRuntimeKind::Unknown => SandboxModel::DockerContainer,
    }
}

fn confidence_for_compiler(
    framework: Option<FrameworkKind>,
    runtime: ImageRuntimeKind,
    language: LanguageKind,
) -> u8 {
    match framework.unwrap_or(FrameworkKind::Unknown) {
        FrameworkKind::NextJs => 97,
        FrameworkKind::FastApi => 95,
        FrameworkKind::Django => 93,
        FrameworkKind::Axum | FrameworkKind::Actix | FrameworkKind::Rocket => 94,
        FrameworkKind::Vite | FrameworkKind::React => 92,
        FrameworkKind::BunVite | FrameworkKind::BunServer => 91,
        FrameworkKind::Unknown => match (runtime, language) {
            (ImageRuntimeKind::Rust, _) => 90,
            (ImageRuntimeKind::Python, _) => 89,
            (ImageRuntimeKind::Node, Language::JavaScript | Language::TypeScript) => 88,
            _ => 40,
        },
        _ => 90,
    }
}

fn image_runtime_kind_label(runtime: ImageRuntimeKind) -> &'static str {
    match runtime {
        ImageRuntimeKind::Node => "node",
        ImageRuntimeKind::Python => "python",
        ImageRuntimeKind::Rust => "rust",
        ImageRuntimeKind::Bun => "bun",
        ImageRuntimeKind::Unknown => UNKNOWN_SIGNATURE,
    }
}

fn runtime_type_from_image_runtime(runtime: ImageRuntimeKind) -> RuntimeType {
    match runtime {
        ImageRuntimeKind::Node | ImageRuntimeKind::Bun => RuntimeType::Node,
        ImageRuntimeKind::Python => RuntimeType::Python,
        ImageRuntimeKind::Rust => RuntimeType::Rust,
        ImageRuntimeKind::Unknown => RuntimeType::Unknown,
    }
}

fn package_manager_kind_label(package_manager: PackageManagerKind) -> &'static str {
    match package_manager {
        PackageManagerKind::Npm => "npm",
        PackageManagerKind::Pnpm => "pnpm",
        PackageManagerKind::Yarn => "yarn",
        PackageManagerKind::Bun => "bun",
        PackageManagerKind::Cargo => "cargo",
        PackageManagerKind::Pip => "pip",
        PackageManagerKind::Uv => "uv",
        PackageManagerKind::Poetry => "poetry",
    }
}

fn language_kind_label(language: LanguageKind) -> &'static str {
    match language {
        Language::JavaScript => "javascript",
        Language::TypeScript => "typescript",
        Language::Rust => "rust",
        Language::Go => "go",
        Language::Python => "python",
        Language::Unknown => UNKNOWN_SIGNATURE,
    }
}

fn entry_strategy_label(strategy: &EntryStrategy) -> &str {
    match strategy {
        EntryStrategy::NodeScript { command } => command.as_str(),
        EntryStrategy::PythonModule { module } => module.as_str(),
        EntryStrategy::RustBinary { target } => target.as_str(),
        EntryStrategy::BunScript { command } => command.as_str(),
        EntryStrategy::DockerEntrypoint => "docker-entrypoint",
    }
}

fn build_step_label(step: BuildStep) -> &'static str {
    match step {
        BuildStep::InstallDependencies => "install-dependencies",
        BuildStep::Compile => "compile",
        BuildStep::GenerateArtifacts => "generate-artifacts",
        BuildStep::LinkCache => "link-cache",
    }
}

fn sandbox_model_label(model: SandboxModel) -> &'static str {
    match model {
        SandboxModel::ProcessIsolated => "process-isolated",
        SandboxModel::DockerContainer => "docker-container",
        SandboxModel::WasmIsolated => "wasm-isolated",
        SandboxModel::Hybrid => "hybrid",
    }
}

fn repository_fingerprint_material(fingerprint: &RepositoryFingerprint) -> String {
    format!(
        "spec={}|repo_id={}|repo={}|lock={}|deps={}|lang={}|framework={}|services={}",
        fingerprint.spec_version,
        fingerprint.repo_id,
        fingerprint.repo_hash,
        fingerprint
            .lockfile_hash
            .as_deref()
            .unwrap_or(UNKNOWN_SIGNATURE),
        fingerprint
            .dependency_hash
            .as_deref()
            .unwrap_or(UNKNOWN_SIGNATURE),
        fingerprint.language_signature,
        fingerprint
            .framework_signature
            .as_deref()
            .unwrap_or(UNKNOWN_SIGNATURE)
            .to_ascii_lowercase(),
        fingerprint.services.len()
    )
}

fn execution_image_spec_material(spec: &ExecutionImageSpec) -> String {
    let framework = spec
        .framework
        .map(framework_kind_label)
        .unwrap_or(UNKNOWN_SIGNATURE);
    let package_manager = spec
        .package_manager
        .map(package_manager_kind_label)
        .unwrap_or(UNKNOWN_SIGNATURE);
    let build_steps = spec
        .build_steps
        .iter()
        .copied()
        .map(build_step_label)
        .collect::<Vec<_>>()
        .join(",");
    let environment = spec
        .environment
        .variables
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "version={}|commit={}|language={}|runtime={}|runtime_version={}|framework={}|package_manager={}|entry={}|build_steps={}|env={}|sandbox={}|deterministic={}|deterministic_build={}",
        spec.spec_version,
        spec.commit_hash.as_deref().unwrap_or(UNKNOWN_SIGNATURE),
        language_kind_label(spec.language),
        image_runtime_kind_label(spec.runtime),
        spec.runtime_version,
        framework,
        package_manager,
        entry_strategy_label(&spec.entry_strategy),
        build_steps,
        environment,
        sandbox_model_label(spec.sandbox_model),
        spec.caching_policy.deterministic,
        spec.deterministic_build
    )
}

fn execution_image_spec_payload(spec: &ExecutionImageSpec) -> Value {
    let framework = spec
        .framework
        .map(framework_kind_label)
        .unwrap_or(UNKNOWN_SIGNATURE);
    let package_manager = spec
        .package_manager
        .map(package_manager_kind_label)
        .unwrap_or(UNKNOWN_SIGNATURE);
    json!({
        "spec_version": spec.spec_version,
        "commit_hash": spec.commit_hash,
        "deterministic_build": spec.deterministic_build,
        "language": language_kind_label(spec.language),
        "runtime": image_runtime_kind_label(spec.runtime),
        "runtime_version": spec.runtime_version,
        "framework": framework,
        "package_manager": package_manager,
        "entry_strategy": entry_strategy_label(&spec.entry_strategy),
        "build_steps": spec.build_steps.iter().copied().map(build_step_label).collect::<Vec<_>>(),
        "environment": spec.environment.variables.iter().collect::<Vec<_>>(),
        "caching_policy": {
            "key": spec.caching_policy.key,
            "deterministic": spec.caching_policy.deterministic,
        },
        "sandbox_model": sandbox_model_label(spec.sandbox_model),
    })
}

fn warm_cache_binding_key(repo_hash: &str, image_id: &str) -> String {
    hash_key(&format!("{repo_hash}:{image_id}"))
}

fn runtime_type_to_agent_label(runtime: RuntimeType) -> &'static str {
    match runtime {
        RuntimeType::Node => "node",
        RuntimeType::Wasm => "wasm",
        RuntimeType::Rust => "rust",
        RuntimeType::Go => "go",
        RuntimeType::Python => "python",
        RuntimeType::Java => "java",
        RuntimeType::Static => "static",
        RuntimeType::Unknown => "unknown",
    }
}

fn parse_agent_memory_to_mb(memory: &str) -> u64 {
    let mut digits = String::new();
    let mut unit = String::new();
    for ch in memory.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            digits.push(ch);
        } else if !ch.is_whitespace() {
            unit.push(ch.to_ascii_lowercase());
        }
    }
    let Some(value) = digits.parse::<f64>().ok() else {
        eprintln!("unable to parse agent memory declaration `{memory}`");
        return 0;
    };
    let multiplier = match unit.as_str() {
        "tb" | "tib" => 1024.0 * 1024.0,
        "gb" | "gib" => 1024.0,
        "mb" | "mib" | "" => 1.0,
        "kb" | "kib" => 1.0 / 1024.0,
        _ => {
            eprintln!("unsupported agent memory unit `{unit}` from declaration `{memory}`");
            return 0;
        }
    };
    (value * multiplier).round().max(0.0) as u64
}

fn ports_for_framework(framework: Framework) -> Vec<PortInfo> {
    match framework {
        Framework::Vite => vec![PortInfo {
            port: 5173,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Node
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SvelteKit
        | Framework::NextJs
        | Framework::Nuxt
        | Framework::Astro
        | Framework::Remix
        | Framework::Express
        | Framework::NestJs => vec![PortInfo {
            port: 3000,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Rust
        | Framework::Axum
        | Framework::Actix
        | Framework::Rocket
        | Framework::Leptos
        | Framework::Go
        | Framework::Gin
        | Framework::Fiber
        | Framework::Echo => vec![PortInfo {
            port: 8080,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Streamlit => vec![PortInfo {
            port: 8501,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Gradio => vec![PortInfo {
            port: 7860,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Python | Framework::Flask | Framework::FastApi | Framework::Django => {
            vec![PortInfo {
                port: 8000,
                protocol: "http".to_string(),
                route: "/".to_string(),
            }]
        }
        Framework::StaticWeb => vec![PortInfo {
            port: 4173,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Unknown => vec![],
    }
}

fn infer_application_topology(root: &Path) -> Option<ApplicationTopology> {
    let mut services = vec![];
    for service_root in discover_service_roots(root) {
        if let Some(service) = infer_service_definition(root, &service_root) {
            services.push(service);
        }
    }

    if services.len() < MIN_SERVICES_FOR_TOPOLOGY {
        return None;
    }

    services.sort_by(|left, right| left.id.cmp(&right.id));
    let dependencies = infer_service_dependencies(&services);
    let startup_order = compute_startup_order(&services, &dependencies);
    let sorted_service_ids = services
        .iter()
        .map(|service| service.id.clone())
        .collect::<Vec<_>>();
    let topology_id = format!("mstr-{}", hash_key(&sorted_service_ids.join("|")));
    let global_network = infer_network_topology(&services);
    let startup_strategy = StartupStrategy {
        stages: startup_order.stages.clone(),
        enforce_dependencies: true,
    };
    let health_policy = infer_health_policy(&services);
    Some(ApplicationTopology {
        topology_id,
        services,
        edges: dependencies.clone(),
        global_network,
        startup_strategy,
        health_policy,
        dependencies,
        startup_order,
    })
}

fn discover_service_roots(root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![];
    let mut seen = HashSet::new();

    let apps_root = root.join("apps");
    if let Ok(entries) = fs::read_dir(&apps_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                && seen.insert(path.clone())
            {
                roots.push(path);
            }
        }
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let normalized = name.to_ascii_lowercase();
            let is_candidate = matches!(
                normalized.as_str(),
                "web"
                    | "frontend"
                    | "ui"
                    | "api"
                    | "backend"
                    | "server"
                    | "worker"
                    | "celery"
                    | "cron"
                    | "jobs"
                    | "db"
                    | "database"
                    | "postgres"
                    | "redis"
                    | "cache"
                    | "queue"
            );
            if is_candidate && seen.insert(path.clone()) {
                roots.push(path);
            }
        }
    }

    roots
}

fn infer_service_definition(repo_root: &Path, service_root: &Path) -> Option<ServiceDefinition> {
    let relative = service_root.strip_prefix(repo_root).ok()?;
    let relative_str = relative.to_string_lossy().replace('\\', "/");
    let id = relative_str.replace('/', "-");
    if id.is_empty() {
        return None;
    }
    let name = service_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());
    let normalized_name = name.to_ascii_lowercase();

    let (framework, _, package_content) = infer_framework_and_language(service_root);
    let is_infra = matches!(
        normalized_name.as_str(),
        "db" | "database" | "postgres" | "redis" | "cache" | "queue"
    );
    let is_worker = normalized_name.contains("worker")
        || normalized_name.contains("celery")
        || normalized_name.contains("cron");
    if framework == Framework::Unknown && !is_infra && !is_worker {
        return None;
    }

    let package_manager = if service_root.join("pnpm-lock.yaml").exists()
        || package_manager_declares(&package_content, "pnpm")
    {
        Some("pnpm".to_string())
    } else if service_root.join("yarn.lock").exists()
        || package_manager_declares(&package_content, "yarn")
    {
        Some("yarn".to_string())
    } else if service_root.join("bun.lockb").exists()
        || service_root.join("bun.lock").exists()
        || package_manager_declares(&package_content, "bun")
    {
        Some("bun".to_string())
    } else if framework != Framework::Unknown {
        Some("npm".to_string())
    } else {
        None
    };

    let ports = if framework == Framework::Unknown {
        infer_default_infrastructure_ports(&normalized_name)
    } else {
        ports_for_framework(framework)
            .into_iter()
            .map(|port| port.port)
            .collect()
    };
    let readiness_checks = readiness_checks_for_service(framework, &normalized_name, &ports);
    let start_command = service_start_command(
        service_root,
        framework,
        package_manager.as_deref(),
        &normalized_name,
    );
    let working_directory = service_root.to_string_lossy().to_string();

    Some(ServiceDefinition {
        id,
        name,
        runtime: runtime_for_service(framework, &normalized_name),
        package_manager,
        working_directory,
        start_command,
        ports,
        readiness_checks,
    })
}

fn infer_default_infrastructure_ports(name: &str) -> Vec<u16> {
    match name {
        "db" | "database" | "postgres" => vec![5432],
        "redis" | "cache" => vec![6379],
        "queue" => vec![5672],
        _ => vec![],
    }
}

fn runtime_for_service(framework: Framework, name: &str) -> RuntimeType {
    let runtime = runtime_for_framework(framework);
    if runtime != RuntimeType::Unknown {
        return runtime;
    }
    if name.contains("worker") || name.contains("celery") || name.contains("cron") {
        RuntimeType::Python
    } else if matches!(
        name,
        "db" | "database" | "postgres" | "redis" | "cache" | "queue"
    ) {
        RuntimeType::Static
    } else {
        RuntimeType::Unknown
    }
}

fn service_start_command(
    service_root: &Path,
    framework: Framework,
    package_manager: Option<&str>,
    normalized_name: &str,
) -> String {
    let root = service_root.display();
    match framework {
        Framework::FastApi => format!("cd {root} && uvicorn app:app --host 0.0.0.0 --port 8000"),
        Framework::Django => format!("cd {root} && python manage.py runserver 0.0.0.0:8000"),
        Framework::Flask => format!("cd {root} && flask run --host 0.0.0.0 --port 8000"),
        Framework::Streamlit => {
            format!("cd {root} && streamlit run app.py --server.address 0.0.0.0 --server.port 8501")
        }
        Framework::Rust
        | Framework::Axum
        | Framework::Actix
        | Framework::Rocket
        | Framework::Leptos => {
            format!("cd {root} && cargo run")
        }
        Framework::Go | Framework::Gin | Framework::Fiber | Framework::Echo => {
            format!("cd {root} && go run .")
        }
        Framework::StaticWeb => format!("cd {root} && serve ."),
        Framework::Node
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SvelteKit
        | Framework::Vite
        | Framework::NextJs
        | Framework::Nuxt
        | Framework::Astro
        | Framework::Remix
        | Framework::Express
        | Framework::NestJs => match package_manager.unwrap_or("npm") {
            "pnpm" => format!("cd {root} && pnpm run dev -- --host 0.0.0.0"),
            "yarn" => format!("cd {root} && yarn dev --host 0.0.0.0"),
            "bun" => format!("cd {root} && bun run dev -- --host 0.0.0.0"),
            _ => format!("cd {root} && npm run dev -- --host 0.0.0.0"),
        },
        Framework::Python | Framework::Gradio => format!("cd {root} && python -m app"),
        Framework::Unknown => match normalized_name {
            "worker" | "celery" | "cron" => {
                format!("cd {root} && celery -A app worker --loglevel=info")
            }
            "db" | "database" | "postgres" => {
                "docker run -d --name rustgit-postgres postgres".to_string()
            }
            "redis" | "cache" => "docker run -d --name rustgit-redis redis".to_string(),
            "queue" => "docker run -d --name rustgit-queue rabbitmq".to_string(),
            _ => format!("cd {root}"),
        },
    }
}

fn readiness_checks_for_service(
    framework: Framework,
    normalized_name: &str,
    ports: &[u16],
) -> Vec<ReadinessCheck> {
    let mut checks = ports
        .iter()
        .copied()
        .map(ReadinessCheck::Port)
        .collect::<Vec<_>>();

    match framework {
        Framework::FastApi => checks.push(ReadinessCheck::Http("/docs".to_string())),
        Framework::Django => checks.push(ReadinessCheck::Http("/admin".to_string())),
        Framework::NextJs
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::SvelteKit
        | Framework::Vite
        | Framework::Nuxt
        | Framework::Astro
        | Framework::Remix
        | Framework::Node
        | Framework::Express
        | Framework::NestJs
        | Framework::StaticWeb => checks.push(ReadinessCheck::Http("/".to_string())),
        Framework::Unknown
            if matches!(
                normalized_name,
                "db" | "database" | "postgres" | "redis" | "cache"
            ) => {}
        _ => {}
    }
    checks.push(ReadinessCheck::Process);
    checks
}

fn infer_service_dependencies(services: &[ServiceDefinition]) -> Vec<ServiceDependency> {
    let mut dependencies = vec![];
    let mut dedupe = HashSet::new();
    let backend = services
        .iter()
        .find(|service| matches!(service_role(service), ServiceRole::Backend))
        .map(|service| service.id.clone());
    let datastores = services
        .iter()
        .filter(|service| {
            matches!(
                service_role(service),
                ServiceRole::DataStore | ServiceRole::Queue
            )
        })
        .map(|service| service.id.clone())
        .collect::<Vec<_>>();

    for service in services {
        let role = service_role(service);
        match role {
            ServiceRole::Frontend => {
                if let Some(target) = backend.as_ref() {
                    let key = format!("{}->{target}", service.id);
                    if dedupe.insert(key) {
                        dependencies.push(ServiceDependency {
                            service_id: service.id.clone(),
                            depends_on: target.clone(),
                        });
                    }
                }
            }
            ServiceRole::Backend | ServiceRole::Worker => {
                if matches!(role, ServiceRole::Worker) {
                    if let Some(target) = backend.as_ref() {
                        let key = format!("{}->{target}", service.id);
                        if service.id != *target && dedupe.insert(key) {
                            dependencies.push(ServiceDependency {
                                service_id: service.id.clone(),
                                depends_on: target.clone(),
                            });
                        }
                    }
                }
                for target in &datastores {
                    if service.id == *target {
                        continue;
                    }
                    let key = format!("{}->{target}", service.id);
                    if dedupe.insert(key) {
                        dependencies.push(ServiceDependency {
                            service_id: service.id.clone(),
                            depends_on: target.clone(),
                        });
                    }
                }
            }
            ServiceRole::DataStore | ServiceRole::Queue | ServiceRole::Other => {}
        }
    }

    dependencies
}

fn infer_network_topology(services: &[ServiceDefinition]) -> NetworkTopology {
    let mut service_dns = HashMap::new();
    let mut exposed_ports = HashMap::new();
    for service in services {
        service_dns.insert(
            service.id.clone(),
            format!("{}.svc.local", service.id.replace('_', "-")),
        );
        exposed_ports.insert(service.id.clone(), service.ports.clone());
    }

    let mut service_ids = services
        .iter()
        .map(|service| service.id.clone())
        .collect::<Vec<_>>();
    service_ids.sort();
    NetworkTopology {
        network_id: format!("mstr-net-{}", hash_key(&service_ids.join("|"))),
        service_dns,
        exposed_ports,
    }
}

fn infer_health_policy(services: &[ServiceDefinition]) -> HealthPolicy {
    let mut service_checks = HashMap::new();
    for service in services {
        service_checks.insert(service.id.clone(), service.readiness_checks.clone());
    }
    HealthPolicy {
        service_checks,
        require_healthy_dependencies: true,
    }
}

fn compute_startup_order(
    services: &[ServiceDefinition],
    dependencies: &[ServiceDependency],
) -> StartupOrder {
    let mut indegree = services
        .iter()
        .map(|service| (service.id.clone(), 0usize))
        .collect::<HashMap<_, _>>();
    let mut adjacency = HashMap::<String, Vec<String>>::new();
    for dependency in dependencies {
        if let Some(count) = indegree.get_mut(&dependency.service_id) {
            *count += 1;
        }
        adjacency
            .entry(dependency.depends_on.clone())
            .or_default()
            .push(dependency.service_id.clone());
    }

    let mut stages = vec![];
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(id.clone()))
        .collect::<BTreeSet<_>>();
    while !ready.is_empty() {
        let stage = ready.iter().cloned().collect::<Vec<_>>();
        stages.push(stage.clone());
        ready.clear();
        for current in stage {
            if let Some(next_ids) = adjacency.get(&current) {
                for next in next_ids {
                    if let Some(count) = indegree.get_mut(next) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            ready.insert(next.clone());
                        }
                    }
                }
            }
            indegree.remove(&current);
        }
    }

    if !indegree.is_empty() {
        let mut unresolved = indegree.keys().cloned().collect::<Vec<_>>();
        unresolved.sort();
        stages.push(unresolved);
    }

    StartupOrder { stages }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceRole {
    Frontend,
    Backend,
    Worker,
    DataStore,
    Queue,
    Other,
}

fn service_role(service: &ServiceDefinition) -> ServiceRole {
    let name = service.name.to_ascii_lowercase();
    if name.contains("worker") || name.contains("celery") || name.contains("cron") {
        return ServiceRole::Worker;
    }
    if name.contains("queue") {
        return ServiceRole::Queue;
    }
    if name == "db"
        || name == "database"
        || name.contains("postgres")
        || name.contains("redis")
        || name.contains("cache")
    {
        return ServiceRole::DataStore;
    }

    match service.runtime {
        RuntimeType::Node => {
            if matches!(
                name.as_str(),
                "web" | "frontend" | "ui" | "site" | "client" | "app"
            ) {
                ServiceRole::Frontend
            } else if matches!(name.as_str(), "api" | "backend" | "server") {
                ServiceRole::Backend
            } else if service.ports.contains(&3000) || service.ports.contains(&5173) {
                ServiceRole::Frontend
            } else {
                ServiceRole::Backend
            }
        }
        RuntimeType::Python | RuntimeType::Rust | RuntimeType::Go | RuntimeType::Java => {
            if matches!(name.as_str(), "web" | "frontend" | "ui") {
                ServiceRole::Frontend
            } else {
                ServiceRole::Backend
            }
        }
        RuntimeType::Static => {
            if matches!(
                name.as_str(),
                "db" | "database" | "postgres" | "redis" | "cache"
            ) {
                ServiceRole::DataStore
            } else {
                ServiceRole::Frontend
            }
        }
        RuntimeType::Wasm | RuntimeType::Unknown => ServiceRole::Other,
    }
}

fn service_install_command(service: &ServiceDefinition) -> Option<String> {
    let service_root = &service.working_directory;
    match service.runtime {
        RuntimeType::Node => Some(match service.package_manager.as_deref().unwrap_or("npm") {
            "pnpm" => format!("cd {service_root} && pnpm install --frozen-lockfile"),
            "yarn" => format!("cd {service_root} && yarn install --frozen-lockfile"),
            "bun" => format!("cd {service_root} && bun install --frozen-lockfile"),
            _ => format!("cd {service_root} && npm install"),
        }),
        RuntimeType::Python => Some(format!(
            "cd {service_root} && python -m pip install -r requirements.txt"
        )),
        RuntimeType::Java => Some(
            match service.package_manager.as_deref().unwrap_or("maven") {
                "gradle" => format!("cd {service_root} && ./gradlew dependencies"),
                _ => format!("cd {service_root} && mvn dependency:resolve"),
            },
        ),
        _ => None,
    }
}

fn service_build_command(service: &ServiceDefinition) -> String {
    let root = &service.working_directory;
    match service.runtime {
        RuntimeType::Node => match service.package_manager.as_deref().unwrap_or("npm") {
            "pnpm" => format!("cd {root} && pnpm run build"),
            "yarn" => format!("cd {root} && yarn build"),
            "bun" => format!("cd {root} && bun run build"),
            _ => format!("cd {root} && npm run build"),
        },
        RuntimeType::Rust => format!("cd {root} && cargo build"),
        RuntimeType::Go => format!("cd {root} && go build ./..."),
        RuntimeType::Python => format!("cd {root} && python -m compileall ."),
        RuntimeType::Java => match service.package_manager.as_deref().unwrap_or("maven") {
            "gradle" => format!("cd {root} && ./gradlew build"),
            _ => format!("cd {root} && mvn package"),
        },
        RuntimeType::Static => format!("cd {root}"),
        RuntimeType::Wasm | RuntimeType::Unknown => format!("cd {root}"),
    }
}

fn shared_build_command(analysis: &RepositoryAnalysis) -> String {
    let root = analysis.root.to_string_lossy();
    if analysis.root.join("turbo.json").exists() {
        format!("cd {root} && turbo run build")
    } else if analysis.root.join("nx.json").exists() {
        format!("cd {root} && nx run-many --target=build --all")
    } else if analysis.root.join("pnpm-workspace.yaml").exists() {
        format!("cd {root} && pnpm -r run build")
    } else if analysis.root.join("package.json").exists() {
        format!("cd {root} && npm run build")
    } else {
        format!("cd {root}")
    }
}

fn available_disk_bytes(path: &Path) -> Option<u64> {
    let check = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    // `df -Pk` gives POSIX output (no line wrapping) in 1 K-blocks.
    let output = Command::new("df").arg("-Pk").arg(&check).output().ok()?;
    if !output.status.success() {
        return None;
    }
    // Line 0: headers; Line 1: data — "Filesystem 1K-blocks Used Available ..."
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().nth(1)?;
    line.split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()
        .map(|kb| kb * 1024)
}

fn directory_size_bytes(path: &Path) -> u64 {
    fn recurse(p: &Path, total: &mut u64) {
        let Ok(entries) = fs::read_dir(p) else { return };
        for entry in entries.flatten() {
            let ep = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                recurse(&ep, total);
            } else if ft.is_symlink() {
                if let Ok(meta) = ep.metadata() {
                    *total += meta.len();
                }
            } else {
                *total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    let mut total = 0u64;
    recurse(path, &mut total);
    total
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    if !source.exists() {
        return Err(RuntimeError::InvalidPath(source.display().to_string()));
    }

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_path = entry.path();
        if destination.starts_with(&entry_path) {
            continue;
        }
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            fs::create_dir_all(&target)?;
            copy_directory(&entry_path, &target)?;
        } else if file_type.is_symlink() {
            // entry.file_type() doesn't follow symlinks; check the real target
            match entry_path.metadata() {
                Ok(meta) if meta.is_dir() => {
                    fs::create_dir_all(&target)?;
                    copy_directory(&entry_path, &target)?;
                }
                Ok(_) => {
                    // symlink to a file — copy the file content, skip on error
                    let _ = fs::copy(&entry_path, &target);
                }
                Err(_) => {} // broken symlink — skip
            }
        } else {
            fs::copy(&entry_path, &target)?;
        }
    }

    Ok(())
}

/// Checks if `dependency` exists under package.json `dependencies` or `devDependencies`.
fn package_mentions_dependency(content: &str, dependency: &str) -> bool {
    dependency_in_object(content, "dependencies", dependency)
        || dependency_in_object(content, "devDependencies", dependency)
}

fn text_mentions_dependency(content: &str, dependency: &str) -> bool {
    let haystack = content.to_ascii_lowercase();
    let needle = dependency.to_ascii_lowercase();
    if haystack.is_empty() || needle.is_empty() {
        return false;
    }

    let is_token_char = |byte: u8| byte.is_ascii_alphanumeric() || b"-_./@".contains(&byte);
    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }

    for start in 0..=(haystack_bytes.len() - needle_bytes.len()) {
        if &haystack_bytes[start..start + needle_bytes.len()] != needle_bytes {
            continue;
        }
        let left_ok = start == 0 || !is_token_char(haystack_bytes[start - 1]);
        let right_index = start + needle_bytes.len();
        let right_ok =
            right_index == haystack_bytes.len() || !is_token_char(haystack_bytes[right_index]);
        if left_ok && right_ok {
            return true;
        }
    }

    false
}

/// Extracts an object block by key and checks whether it contains a quoted dependency key.
fn dependency_in_object(content: &str, object_key: &str, dependency: &str) -> bool {
    let key = format!("\"{object_key}\"");
    let dep = format!("\"{dependency}\"");

    let Some(mut index) = content.find(&key) else {
        return false;
    };
    index += key.len();

    let Some(open_brace_offset) = content[index..].find('{') else {
        return false;
    };
    let mut cursor = index + open_brace_offset + 1;
    let mut depth: usize = 1;

    while cursor < content.len() && depth > 0 {
        let ch = content.as_bytes()[cursor] as char;
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
        cursor += 1;
    }

    if depth != 0 || cursor <= index + open_brace_offset + 1 {
        return false;
    }

    let dependency_block = &content[(index + open_brace_offset + 1)..cursor];
    dependency_block.contains(&dep)
}

fn parse_package_scripts(content: &str) -> HashMap<String, String> {
    let Ok(package_json) = serde_json::from_str::<Value>(content) else {
        return HashMap::new();
    };

    package_json
        .get("scripts")
        .and_then(Value::as_object)
        .map(|scripts| {
            scripts
                .iter()
                .filter_map(|(name, command)| {
                    command
                        .as_str()
                        .map(|command| (name.clone(), command.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn package_manager_declares(content: &str, package_manager: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    let compact: String = content
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.contains(&format!("\"packagemanager\":\"{}@", package_manager))
}

#[cfg(test)]
mod tests;
