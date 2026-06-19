use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use wasmtime::{Config, Engine, Linker, Module, Store};

const WASM_FULL_MEMORY_LIMIT_MB: u64 = 512;
const WASM_FULL_CPU_LIMIT_UNITS: u32 = 1_000;
const WASM_PARTIAL_MEMORY_LIMIT_MB: u64 = 256;
const WASM_PARTIAL_CPU_LIMIT_UNITS: u32 = 750;
const CPU_UNIT_TO_TIME_LIMIT_MS: u64 = 10;
const CACHE_KEY_NODE_MODE_SEPARATOR: &str = "@";
const BYTES_PER_MB: u64 = 1024 * 1024;
const SESSION_GRAPH_EVENT_BUFFER_LIMIT: usize = 1_024;
const SESSION_WORKER_EVENT_BUFFER_LIMIT: usize = 1_024;
const DISTRIBUTED_ARTIFACT_STORE_POISONED: &str =
    "distributed artifact store lock poisoned: another thread panicked while holding the lock";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Node,
    StaticWeb,
    Vite,
    React,
    Vue,
    Svelte,
    NextJs,
    Rust,
    Go,
    Python,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Rust,
    Go,
    Python,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryFingerprint {
    pub repo_hash: String,
    pub lockfile_hash: Option<String>,
    pub dependency_hash: Option<String>,
    pub language_signature: String,
    pub framework_signature: Option<String>,
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
        let engine = Engine::new(&config)
            .map_err(|err| RuntimeError::WasmRuntime(format!("failed to initialize engine: {err}")))?;
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
                "wasm execution context requires non-empty node and workspace identifiers".to_string(),
            ));
        }
        if !Path::new(&ctx.env.repo_path).is_absolute() {
            return Err(RuntimeError::InvalidPath(ctx.env.repo_path.clone()));
        }
        if !ctx.sandbox.filesystem_scope.iter().any(|scope| scope == &ctx.env.repo_path) {
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

        let module = Module::from_binary(&self.engine, &ctx.module.bytes)
            .map_err(|err| RuntimeError::WasmRuntime(format!("module compilation failed: {err}")))?;
        self.enforce_memory_limits(&module, &effective_spec)?;

        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(u64::from(effective_spec.cpu_limit_units))
            .map_err(|err| RuntimeError::WasmRuntime(format!("failed to set fuel limits: {err}")))?;
        let instance = self
            .linker
            .instantiate(&mut store, &module)
            .map_err(|err| RuntimeError::WasmRuntime(format!("module instantiation failed: {err}")))?;

        if let Ok(entrypoint) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            entrypoint
                .call(&mut store, ())
                .map_err(|err| RuntimeError::WasmRuntime(format!("module execution failed: {err}")))?;
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
                time_limit_ms: u64::from(spec.cpu_limit_units).saturating_mul(CPU_UNIT_TO_TIME_LIMIT_MS),
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
                request.command,
                resources.max_memory_mb,
                resources.max_cpu_millis
            ),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceState {
    Created,
    Materializing,
    Analyzing,
    Planning,
    Starting,
    Running,
    Paused,
    Failed,
    Stopping,
    Stopped,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    pub port: u16,
    pub protocol: String,
    pub route: String,
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

pub struct ExecutionRouter;

impl ExecutionRouter {
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
                memory_limit_mb: 0,
                cpu_limit_units: 0,
                allowed_syscalls: vec![],
            },
        }
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
        for node in &mut self.nodes {
            node.cache_key = keys.get(&node.id).cloned();
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

#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryAnalysis {
    pub root: PathBuf,
    pub framework: Framework,
    pub language: Language,
    pub dependency_files: Vec<PathBuf>,
    pub fingerprint: RepositoryFingerprint,
    pub classification: RepositoryClassification,
    pub execution_profile: ExecutionProfile,
    pub build_intelligence: BuildIntelligence,
    pub execution_graph: ExecutionGraph,
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
        self.root.join("wasm").join(format!("{node_id}.artifact.json"))
    }

    fn wasm_binding_path(&self, node_id: &str) -> PathBuf {
        self.root.join("wasm").join(format!("{node_id}.binding.json"))
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkerNode {
    pub id: String,
    pub capabilities: WorkerCapabilities,
    pub status: WorkerStatus,
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
                .map(|required| required.iter().all(|key| artifact_store.fetch(key).is_some()))
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
            ordered_nodes,
            assignments,
            leases,
            worker_queues,
            partitions,
            unscheduled_nodes,
        }
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
    labels
        .iter()
        .all(|label| worker.capabilities.labels.iter().any(|worker_label| worker_label == label))
}

#[derive(Debug, Clone)]
pub struct ExecutionCoordinator {
    pub scheduler: DistributedScheduler,
    pub workers: Vec<WorkerNode>,
    pub artifact_store: DistributedArtifactStore,
}

impl ExecutionCoordinator {
    pub fn new(workers: Vec<WorkerNode>, artifact_store: DistributedArtifactStore) -> Self {
        Self {
            scheduler: DistributedScheduler,
            workers,
            artifact_store,
        }
    }

    pub fn plan(
        &self,
        graph: ExecutionGraph,
        config: &DistributedExecutionConfig,
        now: u64,
    ) -> ExecutionPlan {
        self.scheduler.schedule_with_context(
            graph,
            self.workers.clone(),
            &self.artifact_store,
            config,
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
        if let Some(worker) = self.workers.iter_mut().find(|worker| worker.id == failed_worker_id) {
            worker.status = WorkerStatus::Offline;
        }
        self.plan(graph, config, now)
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

        hash_key(&format!(
            "{}|{}|{}|{}|{}|{}",
            node_type_name(node.node_type),
            execution_mode_name(node.execution_mode),
            node.command.as_deref().unwrap_or_default(),
            format!("in:{}|out:{}", incoming.join(","), outgoing.join(",")),
            repo_hash,
            env_hash
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessHandle {
    pub pid_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStatus {
    pub healthy: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceQuotas {
    pub max_memory_mb: u32,
    pub max_cpu_millis: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPolicy {
    pub allow_outbound: bool,
    pub allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionContext {
    pub workspace_id: String,
    pub repo_path: String,
    pub analysis: RepositoryAnalysis,
    pub execution_graph: ExecutionGraph,
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
    /// Returns true when this provider owns runtime execution for `ctx`.
    fn can_handle(&self, ctx: &ExecutionContext) -> bool;
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

struct WorkspaceRecord {
    workspace: Workspace,
    logs: Vec<String>,
    execution_context: Option<ExecutionContext>,
    process_handle: Option<ProcessHandle>,
}

pub struct ExecutionEngine {
    providers: Vec<Box<dyn ExecutionProvider + Send + Sync>>,
    artifact_store: ArtifactStore,
}

pub struct WorkspaceManager {
    root: PathBuf,
    execution_engine: ExecutionEngine,
    workspaces: Arc<Mutex<HashMap<String, WorkspaceRecord>>>,
    repository_cache: Arc<Mutex<HashMap<String, PathBuf>>>,
    sequence: AtomicU64,
}

impl ExecutionEngine {
    pub fn new(
        providers: Vec<Box<dyn ExecutionProvider + Send + Sync>>,
        artifact_store: ArtifactStore,
    ) -> Self {
        Self {
            providers,
            artifact_store,
        }
    }

    fn provider_for(
        &self,
        ctx: &ExecutionContext,
    ) -> Result<&(dyn ExecutionProvider + Send + Sync)> {
        self.providers
            .iter()
            .find(|provider| provider.can_handle(ctx))
            .map(|provider| provider.as_ref())
            .ok_or_else(|| {
                RuntimeError::UnsupportedRepository(format!(
                    "no execution provider matched for workspace {} with framework {:?}",
                    ctx.workspace_id, ctx.analysis.framework
                ))
            })
    }

    pub fn start(&self, ctx: &mut ExecutionContext) -> Result<ProcessHandle> {
        self.prime_artifacts(ctx)?;
        let provider = self.provider_for(ctx)?;
        provider.prepare(ctx)?;
        let handle = provider.start(ctx)?;
        let health = provider.health(&handle)?;
        if health.healthy {
            Ok(handle)
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
        let provider = self.provider_for(ctx)?;
        provider.stop(handle)
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
            Box::new(NodeRuntimeProvider),
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

    fn materialize_repository(&self, repo_url: &str, destination: &Path) -> Result<()> {
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

        if looks_like_local_path(repo_url) {
            copy_directory(Path::new(repo_url), destination)?;
        } else {
            let status = Command::new("git")
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg(repo_url)
                .arg(destination)
                .status()
                .map_err(|e| RuntimeError::CommandFailed(format!("git clone failed: {e}")))?;

            if !status.success() {
                return Err(RuntimeError::CommandFailed(format!(
                    "git clone exited with status {status}"
                )));
            }
        }

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

        Ok(())
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
                WorkspaceRecord {
                    workspace: workspace.clone(),
                    logs: vec!["workspace created".to_string()],
                    execution_context: None,
                    process_handle: None,
                },
            );
        }

        let launch_result = (|| -> Result<(ExecutionContext, ProcessHandle)> {
            Self::transition_state(&mut workspace, WorkspaceState::Materializing)?;
            self.materialize_repository(repo_url, &repository_root)?;
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
                wasm_sandbox: None,
                resources: workspace.resource_quotas.clone(),
                network: workspace.network_policy.clone(),
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
            workspace.ports = ports_for_framework(ctx.analysis.framework);
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
                record.workspace = workspace.clone();
                record.logs.extend(logs);
                record.execution_context = Some(ctx);
                record.process_handle = Some(handle);
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
        if let (Some(ctx), Some(handle)) = (&record.execution_context, &record.process_handle) {
            self.execution_engine.stop(ctx, handle)?;
        }
        record.process_handle = None;
        Self::transition_state(&mut record.workspace, WorkspaceState::Stopped)?;
        record.logs.push("workspace stopped".to_string());
        Ok(())
    }

    fn restart(&self, id: &str) -> Result<()> {
        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;
        Self::transition_state(&mut record.workspace, WorkspaceState::Starting)?;
        let mut execution_context = record
            .execution_context
            .clone()
            .ok_or_else(|| RuntimeError::ExecutionContextMissing(id.to_string()))?;
        let handle = self.execution_engine.start(&mut execution_context)?;
        Self::transition_state(&mut record.workspace, WorkspaceState::Running)?;
        record.execution_context = Some(execution_context);
        record.process_handle = Some(handle);
        record.logs.push("workspace restarted".to_string());
        Ok(())
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

impl RepositoryRegistry {
    pub fn get_or_compute(repo_reference: &str) -> ExecutionProfile {
        let root = Path::new(repo_reference);
        if !root.exists() {
            return Self::default_profile(repo_reference);
        }

        let snapshot = collect_repository_snapshot(root);
        let (framework, language, package_content) = infer_framework_and_language(root);
        Self::compute_and_cache_profile(
            repo_reference,
            snapshot,
            framework,
            language,
            &package_content,
        )
    }

    fn compute_and_cache_profile(
        repo_reference: &str,
        snapshot: HashMap<String, String>,
        framework: Framework,
        language: Language,
        package_content: &str,
    ) -> ExecutionProfile {
        let fingerprint = build_repository_fingerprint(&snapshot, framework, language);

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
    let package_content = fs::read_to_string(&package_json).unwrap_or_default();

    let framework = if package_mentions_dependency(&package_content, "next")
        || package_mentions_dependency(&package_content, "nextjs")
    {
        Framework::NextJs
    } else if package_mentions_dependency(&package_content, "svelte") {
        Framework::Svelte
    } else if package_mentions_dependency(&package_content, "vue") {
        Framework::Vue
    } else if package_mentions_dependency(&package_content, "react") {
        Framework::React
    } else if package_mentions_dependency(&package_content, "vite") {
        Framework::Vite
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
        | Framework::Node
        | Framework::Vite
        | Framework::NextJs => {
            if package_mentions_dependency(&package_content, "typescript")
                || root.join("tsconfig.json").exists()
            {
                Language::TypeScript
            } else {
                Language::JavaScript
            }
        }
        Framework::Rust => Language::Rust,
        Framework::Go => Language::Go,
        Framework::Python => Language::Python,
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
    snapshot: &HashMap<String, String>,
    framework: Framework,
    language: Language,
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

    RepositoryFingerprint {
        repo_hash: hash_key(&normalized.join("|")),
        lockfile_hash,
        dependency_hash,
        language_signature: language_signature(snapshot, language),
        framework_signature: Some(format!("{framework:?}")),
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
            | Framework::Vite => (RepoClass::NodeApp, 0.9),
            Framework::Rust => (RepoClass::RustBinary, 0.92),
            Framework::Python => (RepoClass::PythonApp, 0.9),
            Framework::StaticWeb => (RepoClass::StaticSite, 0.88),
            Framework::Unknown => (RepoClass::Unknown, 0.2),
            Framework::Go => (RepoClass::Unknown, 0.4),
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
        | Framework::NextJs => RuntimeType::Node,
        Framework::Rust => RuntimeType::Rust,
        Framework::Go => RuntimeType::Go,
        Framework::Python => RuntimeType::Python,
        Framework::StaticWeb => RuntimeType::Static,
        Framework::Unknown => RuntimeType::Unknown,
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

    let package_json = root.join("package.json");
    let cargo_toml = root.join("Cargo.toml");
    let go_mod = root.join("go.mod");
    let requirements = root.join("requirements.txt");
    let pyproject = root.join("pyproject.toml");

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

    let (framework, language, package_content) = infer_framework_and_language(root);

    if framework == Framework::Unknown {
        return Err(RuntimeError::UnsupportedRepository(
            "unable to infer execution strategy".to_string(),
        ));
    }

    let scripts = parse_package_scripts(&package_content);
    let package_manager = if root.join("pnpm-lock.yaml").exists() {
        Some("pnpm".to_string())
    } else if root.join("yarn.lock").exists() {
        Some("yarn".to_string())
    } else if package_json.exists() {
        Some("npm".to_string())
    } else if cargo_toml.exists() {
        Some("cargo".to_string())
    } else if go_mod.exists() {
        Some("go".to_string())
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
        | Framework::NextJs => vec!["node".to_string(), "npm".to_string()],
        Framework::Rust => vec!["cargo".to_string()],
        Framework::Go => vec!["go".to_string()],
        Framework::Python => vec!["python".to_string(), "pip".to_string()],
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
        snapshot,
        framework,
        language,
        &package_content,
    );
    let mut analysis = RepositoryAnalysis {
        root: root.to_path_buf(),
        framework,
        language,
        dependency_files,
        fingerprint: execution_profile.fingerprint.clone(),
        classification: execution_profile.classification.clone(),
        execution_profile,
        build_intelligence,
        execution_graph: ExecutionGraph::default(),
    };
    analysis.execution_graph =
        BuildPlanner::build_graph(&analysis).with_cache_keys_for(Some(&analysis.fingerprint));

    Ok(analysis)
}

impl BuildPlanner {
    pub fn build_graph(analysis: &RepositoryAnalysis) -> ExecutionGraph {
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
                    _ => format!("npm run {name}"),
                }
            } else {
                fallback.to_string()
            }
        };

        let js_install = match package_manager {
            "pnpm" => "pnpm install --frozen-lockfile".to_string(),
            "yarn" => "yarn install --frozen-lockfile".to_string(),
            _ => "npm ci".to_string(),
        };
        let js_build_fallback = match package_manager {
            "pnpm" => "pnpm run build".to_string(),
            "yarn" => "yarn build".to_string(),
            _ => "npm run build".to_string(),
        };
        let js_dev_fallback = match package_manager {
            "pnpm" => "pnpm run dev -- --host 0.0.0.0".to_string(),
            "yarn" => "yarn dev --host 0.0.0.0".to_string(),
            _ => "npm run dev -- --host 0.0.0.0".to_string(),
        };
        let js_test_fallback = match package_manager {
            "pnpm" => "pnpm run test".to_string(),
            "yarn" => "yarn test".to_string(),
            _ => "npm test".to_string(),
        };

        match framework {
            Framework::React
            | Framework::Vue
            | Framework::Svelte
            | Framework::Vite
            | Framework::Node
            | Framework::NextJs => {
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
                };
                let dev = ExecutionNode {
                    id: "dev".to_string(),
                    node_type: ExecutionNodeType::DevServer,
                    command: Some(js_script("dev", &js_dev_fallback)),
                    execution_mode: ExecutionMode::Native,
                    inputs: build.outputs.clone(),
                    outputs: vec!["http://0.0.0.0:3000/".to_string()],
                    cache_key: None,
                };
                let test = ExecutionNode {
                    id: "test".to_string(),
                    node_type: ExecutionNodeType::Test,
                    command: Some(js_script("test", &js_test_fallback)),
                    execution_mode: ExecutionMode::Hybrid,
                    inputs: vec!["node_modules".to_string()],
                    outputs: vec!["test-report".to_string()],
                    cache_key: None,
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
            Framework::Rust => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "build".to_string(),
                        node_type: ExecutionNodeType::Build,
                        command: Some("cargo build".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()],
                        outputs: vec!["target".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("cargo run".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["target".to_string()],
                        outputs: vec!["http://0.0.0.0:8080/".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("cargo test".to_string()),
                        execution_mode: ExecutionMode::Hybrid,
                        inputs: vec!["target".to_string()],
                        outputs: vec!["test-report".to_string()],
                        cache_key: None,
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
            Framework::Go => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "build".to_string(),
                        node_type: ExecutionNodeType::Build,
                        command: Some("go build ./...".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["go.mod".to_string(), "go.sum".to_string()],
                        outputs: vec!["go-build-cache".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("go run .".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["go-build-cache".to_string()],
                        outputs: vec!["http://0.0.0.0:8080/".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("go test ./...".to_string()),
                        execution_mode: ExecutionMode::Hybrid,
                        inputs: vec!["go-build-cache".to_string()],
                        outputs: vec!["test-report".to_string()],
                        cache_key: None,
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
            Framework::Python => ExecutionGraph {
                nodes: vec![
                    ExecutionNode {
                        id: "install".to_string(),
                        node_type: ExecutionNodeType::InstallDependencies,
                        command: Some("python -m pip install -r requirements.txt".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["requirements.txt|pyproject.toml".to_string()],
                        outputs: vec!["site-packages".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("python -m app".to_string()),
                        execution_mode: ExecutionMode::Native,
                        inputs: vec!["site-packages".to_string()],
                        outputs: vec!["http://0.0.0.0:8000/".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("python -m pytest".to_string()),
                        execution_mode: ExecutionMode::Hybrid,
                        inputs: vec!["site-packages".to_string()],
                        outputs: vec!["test-report".to_string()],
                        cache_key: None,
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
                    },
                    ExecutionNode {
                        id: "serve".to_string(),
                        node_type: ExecutionNodeType::StaticServe,
                        command: Some("serve .".to_string()),
                        execution_mode: ExecutionMode::Wasm,
                        inputs: vec!["pkg/app_bg.wasm".to_string()],
                        outputs: vec!["http://0.0.0.0:4173/".to_string()],
                        cache_key: None,
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
}

pub struct VirtualFileSystem {
    root: PathBuf,
}

impl VirtualFileSystem {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn read(&self, relative_path: &str) -> Result<Vec<u8>> {
        let path = self.resolve(relative_path)?;
        Ok(fs::read(path)?)
    }

    pub fn write(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(relative_path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<WorkspaceSnapshot> {
        let mut entries = HashMap::new();
        collect_files(&self.root, &self.root, &mut entries)?;
        Ok(WorkspaceSnapshot { entries })
    }

    pub fn restore(&self, snapshot: &WorkspaceSnapshot) -> Result<()> {
        for (relative, bytes) in &snapshot.entries {
            let path = self.resolve(relative)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)?;
        }
        Ok(())
    }

    fn resolve(&self, relative_path: &str) -> Result<PathBuf> {
        let path = self.root.join(relative_path);
        if !path.starts_with(&self.root) {
            return Err(RuntimeError::InvalidPath(relative_path.to_string()));
        }
        Ok(path)
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceSnapshot {
    pub entries: HashMap<String, Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct RestApiSpec {
    pub routes: Vec<&'static str>,
}

impl Default for RestApiSpec {
    fn default() -> Self {
        Self {
            routes: vec![
                "POST /workspaces",
                "POST /workspaces/{id}/stop",
                "POST /workspaces/{id}/restart",
                "GET /workspaces/{id}/logs",
                "GET /workspaces/{id}/ports",
                "GET /workspaces/{id}/filesystem/*path",
            ],
        }
    }
}

struct NodeRuntimeProvider;
struct RustRuntimeProvider;
struct StaticRuntimeProvider;

impl ExecutionProvider for WasmExecutionProvider {
    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        if ctx.execution_graph.nodes.is_empty() {
            return false;
        }
        let has_wasm = ctx.execution_graph.nodes.iter().any(|node| {
            matches!(
                ExecutionRouter::route(node, &ctx.analysis.execution_profile),
                ExecutionTarget::Wasm(_)
            )
        });
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
            let Some(node) = ctx.execution_graph.nodes.iter().find(|node| node.id == node_id) else {
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
            RuntimeError::CommandFailed("execution graph contains no dispatchable nodes".to_string())
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
    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        matches!(
            ctx.analysis.framework,
            Framework::Node
                | Framework::Vite
                | Framework::React
                | Framework::Vue
                | Framework::Svelte
                | Framework::NextJs
        )
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("node:{}", ctx.execution_graph.cache_key()),
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

impl ExecutionProvider for RustRuntimeProvider {
    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        ctx.analysis.framework == Framework::Rust
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("rust:{}", ctx.execution_graph.cache_key()),
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
    fn can_handle(&self, ctx: &ExecutionContext) -> bool {
        ctx.analysis.framework == Framework::StaticWeb
    }

    fn prepare(&self, _ctx: &mut ExecutionContext) -> Result<()> {
        Ok(())
    }

    fn start(&self, ctx: &ExecutionContext) -> Result<ProcessHandle> {
        Ok(ProcessHandle {
            pid_hint: format!("static:{}", ctx.execution_graph.cache_key()),
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
            matches!(to, WorkspaceState::Analyzing | WorkspaceState::Failed)
        }
        WorkspaceState::Analyzing => {
            matches!(to, WorkspaceState::Planning | WorkspaceState::Failed)
        }
        WorkspaceState::Planning => matches!(to, WorkspaceState::Starting | WorkspaceState::Failed),
        WorkspaceState::Starting => matches!(to, WorkspaceState::Running | WorkspaceState::Failed),
        WorkspaceState::Running => {
            matches!(
                to,
                WorkspaceState::Paused | WorkspaceState::Stopping | WorkspaceState::Failed
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
                WorkspaceState::Starting | WorkspaceState::Stopping | WorkspaceState::Destroyed
            )
        }
        WorkspaceState::Stopping => matches!(to, WorkspaceState::Stopped | WorkspaceState::Failed),
        WorkspaceState::Stopped => {
            matches!(to, WorkspaceState::Starting | WorkspaceState::Destroyed)
        }
        WorkspaceState::Destroyed => false,
    }
}

fn looks_like_local_path(repo_url: &str) -> bool {
    repo_url.starts_with('/') || repo_url.starts_with("./") || repo_url.starts_with("../")
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

fn ports_for_framework(framework: Framework) -> Vec<PortInfo> {
    match framework {
        Framework::Node
        | Framework::Vite
        | Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::NextJs => vec![PortInfo {
            port: 3000,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Rust | Framework::Go => vec![PortInfo {
            port: 8080,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Python => vec![PortInfo {
            port: 8000,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::StaticWeb => vec![PortInfo {
            port: 4173,
            protocol: "http".to_string(),
            route: "/".to_string(),
        }],
        Framework::Unknown => vec![],
    }
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

        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&target)?;
            copy_directory(&entry_path, &target)?;
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

fn collect_files(
    root: &Path,
    current: &Path,
    entries: &mut HashMap<String, Vec<u8>>,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(root, &path, entries)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| RuntimeError::InvalidPath(path.display().to_string()))?
                .to_string_lossy()
                .to_string();
            entries.insert(relative, fs::read(path)?);
        }
    }
    Ok(())
}

fn current_unix_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wat::parse_str;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustgit_wasm_runtime-{}-{}",
            name,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn test_analysis(
        graph: ExecutionGraph,
        compatibility: WasmCompatibility,
        framework: Framework,
    ) -> RepositoryAnalysis {
        let fingerprint = RepositoryFingerprint {
            repo_hash: "repo".to_string(),
            lockfile_hash: None,
            dependency_hash: None,
            language_signature: "Unknown".to_string(),
            framework_signature: Some(format!("{framework:?}")),
        };
        let classification = RepositoryClassification {
            class: RepoClass::StaticSite,
            confidence: 1.0,
            primary_runtime: RuntimeType::Static,
            secondary_runtimes: vec![],
        };
        let execution_profile = ExecutionProfile {
            fingerprint: fingerprint.clone(),
            classification: classification.clone(),
            recommended_graph_strategy: GraphStrategy::Linear,
            runtime_affinity: RuntimeAffinity {
                preferred_provider: "WasmExecutionProvider".to_string(),
                fallback_providers: vec!["StaticRuntimeProvider".to_string()],
            },
            wasm_compatibility: compatibility,
        };
        RepositoryAnalysis {
            root: PathBuf::from("/tmp/repo"),
            framework,
            language: Language::Unknown,
            dependency_files: vec![],
            fingerprint,
            classification,
            execution_profile,
            build_intelligence: BuildIntelligence {
                framework,
                package_manager: None,
                build_tooling: vec![],
                entrypoints: vec![],
                scripts: HashMap::new(),
            },
            execution_graph: graph,
        }
    }

    #[test]
    fn detects_react_framework_from_package_json() {
        let repo = temp_dir("react-detect");
        fs::write(
            repo.join("package.json"),
            r#"{"dependencies":{"react":"18.0.0"}}"#,
        )
        .expect("write package.json");

        let analysis = analyze_repository(&repo).expect("analyze repo");
        assert_eq!(analysis.framework, Framework::React);
        assert_eq!(analysis.language, Language::JavaScript);
        assert_eq!(
            analysis.execution_graph.primary_run_command().as_deref(),
            Some("npm run dev -- --host 0.0.0.0")
        );
        assert_eq!(
            analysis.build_intelligence.package_manager.as_deref(),
            Some("npm")
        );
    }

    #[test]
    fn js_graph_contains_deterministic_dependencies() {
        let repo = temp_dir("js-graph");
        fs::write(
            repo.join("package.json"),
            r#"{"scripts":{"build":"vite build","dev":"vite"},"dependencies":{"vite":"5.0.0"}}"#,
        )
        .expect("write package.json");

        let analysis = analyze_repository(&repo).expect("analyze repo");
        let graph = &analysis.execution_graph;
        let ordered = graph.ordered_node_ids();
        assert_eq!(ordered.first().map(String::as_str), Some("install"));
        assert_eq!(ordered.get(1).map(String::as_str), Some("build"));
        assert!(ordered.contains(&"dev".to_string()));
        assert!(ordered.contains(&"test".to_string()));
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "build")
                .and_then(|node| node.command.as_deref()),
            Some("npm run build")
        );
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.from == "install" && edge.to == "build"));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.from == "install" && edge.to == "test"));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.from == "build" && edge.to == "dev"));
        assert!(!graph
            .edges
            .iter()
            .any(|edge| edge.from == "test" && edge.to == "dev"));
        assert_eq!(graph.primary_run_command().as_deref(), Some("npm run dev"));
        assert!(graph.nodes.iter().all(|node| node.cache_key.is_some()));
    }

    #[test]
    fn static_web_graph_includes_wasm_compile_binding_step() {
        let repo = temp_dir("static-web-graph");
        fs::write(repo.join("index.html"), "<!doctype html><title>static</title>")
            .expect("write index.html");

        let analysis = analyze_repository(&repo).expect("analyze repo");
        let graph = &analysis.execution_graph;
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.node_type == ExecutionNodeType::WasmCompile));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.from == "wasm-compile" && edge.to == "serve"));
    }

    #[test]
    fn js_graph_uses_detected_package_manager_commands() {
        let repo = temp_dir("js-pnpm-graph");
        fs::write(
            repo.join("package.json"),
            r#"{"dependencies":{"vite":"5.0.0"}}"#,
        )
        .expect("write package.json");
        fs::write(repo.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'")
            .expect("write pnpm lockfile");

        let analysis = analyze_repository(&repo).expect("analyze repo");
        let graph = &analysis.execution_graph;
        assert_eq!(
            analysis.build_intelligence.package_manager.as_deref(),
            Some("pnpm")
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "install")
                .and_then(|node| node.command.as_deref()),
            Some("pnpm install --frozen-lockfile")
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "build")
                .and_then(|node| node.command.as_deref()),
            Some("pnpm run build")
        );
    }

    #[test]
    fn lifecycle_transitions_start_stop_restart() {
        let runtime_root = temp_dir("runtime-root");
        let local_repo = temp_dir("local-repo");
        fs::write(
            local_repo.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write Cargo.toml");

        let manager = WorkspaceManager::new(&runtime_root);
        let workspace = manager
            .launch(local_repo.to_string_lossy().as_ref())
            .expect("launch workspace");
        assert_eq!(workspace.state, WorkspaceState::Running);

        manager.stop(&workspace.id).expect("stop workspace");
        manager.restart(&workspace.id).expect("restart workspace");

        let logs = manager.logs(&workspace.id).expect("workspace logs");
        assert!(logs.iter().any(|line| line.contains("workspace stopped")));
        assert!(logs.iter().any(|line| line.contains("workspace restarted")));
    }

    #[test]
    fn stop_requires_running_or_paused_state() {
        let runtime_root = temp_dir("runtime-root-stop-guard");
        let local_repo = temp_dir("local-repo-stop-guard");
        fs::write(
            local_repo.join("Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .expect("write Cargo.toml");

        let manager = WorkspaceManager::new(&runtime_root);
        let workspace = manager
            .launch(local_repo.to_string_lossy().as_ref())
            .expect("launch workspace");

        manager.stop(&workspace.id).expect("first stop succeeds");
        let err = manager
            .stop(&workspace.id)
            .expect_err("second stop must fail");
        assert!(matches!(
            err,
            RuntimeError::InvalidTransition {
                from: WorkspaceState::Stopped,
                to: WorkspaceState::Stopping,
            },
        ));
    }

    #[test]
    fn state_machine_allows_and_rejects_expected_transitions() {
        assert!(can_transition(
            WorkspaceState::Created,
            WorkspaceState::Materializing
        ));
        assert!(can_transition(
            WorkspaceState::Materializing,
            WorkspaceState::Analyzing
        ));
        assert!(can_transition(
            WorkspaceState::Analyzing,
            WorkspaceState::Planning
        ));
        assert!(can_transition(
            WorkspaceState::Planning,
            WorkspaceState::Starting
        ));
        assert!(can_transition(
            WorkspaceState::Starting,
            WorkspaceState::Running
        ));
        assert!(can_transition(
            WorkspaceState::Paused,
            WorkspaceState::Running
        ));
        assert!(can_transition(
            WorkspaceState::Failed,
            WorkspaceState::Starting
        ));
        assert!(can_transition(
            WorkspaceState::Stopped,
            WorkspaceState::Destroyed
        ));

        assert!(!can_transition(
            WorkspaceState::Created,
            WorkspaceState::Running
        ));
        assert!(!can_transition(
            WorkspaceState::Running,
            WorkspaceState::Created
        ));
        assert!(!can_transition(
            WorkspaceState::Stopped,
            WorkspaceState::Stopping
        ));
        assert!(!can_transition(
            WorkspaceState::Destroyed,
            WorkspaceState::Created
        ));
    }

    #[test]
    fn virtual_filesystem_snapshot_and_restore() {
        let root = temp_dir("vfs");
        let fs = VirtualFileSystem::new(root.clone());
        fs.write("src/main.rs", b"fn main() {}")
            .expect("write source file");

        let snapshot = fs.snapshot().expect("snapshot");
        fs.write("src/main.rs", b"fn main(){println!(\"changed\");}")
            .expect("mutate file");

        fs.restore(&snapshot).expect("restore snapshot");
        let bytes = fs.read("src/main.rs").expect("read restored file");
        assert_eq!(bytes, b"fn main() {}");
    }

    #[test]
    fn cache_key_engine_changes_with_command() {
        let mut graph = ExecutionGraph {
            nodes: vec![ExecutionNode {
                id: "build".to_string(),
                node_type: ExecutionNodeType::Build,
                command: Some("cargo build".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["Cargo.toml".to_string()],
                outputs: vec!["target".to_string()],
                cache_key: None,
            }],
            edges: vec![],
        };
        let first = graph.compute_cache_keys();
        graph.nodes[0].command = Some("cargo build --release".to_string());
        let second = graph.compute_cache_keys();

        assert_ne!(first.get("build"), second.get("build"));
    }

    #[test]
    fn cache_key_engine_changes_with_repository_fingerprint() {
        let graph = ExecutionGraph {
            nodes: vec![ExecutionNode {
                id: "build".to_string(),
                node_type: ExecutionNodeType::Build,
                command: Some("cargo build".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["Cargo.toml".to_string()],
                outputs: vec!["target".to_string()],
                cache_key: None,
            }],
            edges: vec![],
        };
        let first = graph.compute_cache_keys_with_fingerprint(Some(&RepositoryFingerprint {
            repo_hash: "repo-a".to_string(),
            lockfile_hash: None,
            dependency_hash: None,
            language_signature: "Rust".to_string(),
            framework_signature: Some("Rust".to_string()),
        }));
        let second = graph.compute_cache_keys_with_fingerprint(Some(&RepositoryFingerprint {
            repo_hash: "repo-b".to_string(),
            lockfile_hash: None,
            dependency_hash: None,
            language_signature: "Rust".to_string(),
            framework_signature: Some("Rust".to_string()),
        }));

        assert_ne!(first.get("build"), second.get("build"));
    }

    #[test]
    fn repository_registry_classifies_and_tracks_repo_delta() {
        let repo = temp_dir("registry-monorepo");
        fs::write(
            repo.join("package.json"),
            r#"{"dependencies":{"next":"14.2.0","react":"18.2.0"}}"#,
        )
        .expect("write package manifest");
        fs::write(repo.join("pnpm-workspace.yaml"), "packages:\n  - apps/*\n")
            .expect("write workspace manifest");
        fs::create_dir_all(repo.join("apps/web")).expect("create apps dir");
        fs::write(repo.join("apps/web/package.json"), r#"{"name":"web"}"#)
            .expect("write nested package manifest");

        let profile = RepositoryRegistry::get_or_compute(repo.to_string_lossy().as_ref());
        assert_eq!(profile.classification.class, RepoClass::Monorepo);
        assert_eq!(
            profile.recommended_graph_strategy,
            GraphStrategy::MonorepoSegmented
        );

        fs::write(
            repo.join("apps/web/package.json"),
            r#"{"name":"web","private":true}"#,
        )
        .expect("modify nested package manifest");
        let _ = RepositoryRegistry::get_or_compute(repo.to_string_lossy().as_ref());
        let delta = RepositoryRegistry::latest_delta(repo.to_string_lossy().as_ref())
            .expect("delta should be available");
        assert!(delta
            .modified_files
            .iter()
            .any(|path| path == "apps/web/package.json"));
    }

    #[test]
    fn analyze_repository_emits_execution_profile() {
        let repo = temp_dir("analysis-profile");
        fs::write(
            repo.join("package.json"),
            r#"{"dependencies":{"react":"18.0.0"}}"#,
        )
        .expect("write package.json");

        let analysis = analyze_repository(&repo).expect("analyze repo");
        assert!(!analysis.fingerprint.repo_hash.is_empty());
        assert_eq!(analysis.classification.class, RepoClass::NodeApp);
        assert_eq!(
            analysis
                .execution_profile
                .runtime_affinity
                .preferred_provider,
            "NodeRuntimeProvider"
        );
        assert_eq!(
            analysis.execution_profile.wasm_compatibility,
            WasmCompatibility::Partial
        );
    }

    #[test]
    fn artifact_store_round_trips_execution_artifact() {
        let root = temp_dir("artifact-store");
        let store = ArtifactStore::new(root.clone());
        let artifact = ExecutionArtifact {
            key: "cache-key".to_string(),
            node_id: "build".to_string(),
            artifact_type: ArtifactType::BuildOutput,
            path: root.join("build-output").to_string_lossy().to_string(),
            created_at: 42,
        };

        store.put(artifact.clone());

        assert!(store.exists("cache-key"));
        assert_eq!(store.get("cache-key"), Some(artifact));
    }

    #[test]
    fn artifact_store_registers_wasm_artifact_and_binding() {
        let root = temp_dir("artifact-store-wasm");
        let store = ArtifactStore::new(root);

        store.register_wasm_artifact(WasmArtifact {
            node_id: "serve".to_string(),
            module_path: "/repo/pkg/app_bg.wasm".to_string(),
            hash: "abc123".to_string(),
        });
        store.register_wasm_artifact_binding(WasmArtifactBinding {
            node_id: "serve".to_string(),
            artifact_key: "cache-key".to_string(),
            build_fingerprint: "repo-fp".to_string(),
            source_files_hash: "src-fp".to_string(),
        });

        assert_eq!(
            store.get_wasm_artifact("serve"),
            Some(WasmArtifact {
                node_id: "serve".to_string(),
                module_path: "/repo/pkg/app_bg.wasm".to_string(),
                hash: "abc123".to_string(),
            })
        );
        assert_eq!(
            store.get_wasm_artifact_binding("serve"),
            Some(WasmArtifactBinding {
                node_id: "serve".to_string(),
                artifact_key: "cache-key".to_string(),
                build_fingerprint: "repo-fp".to_string(),
                source_files_hash: "src-fp".to_string(),
            })
        );
    }

    #[test]
    fn distributed_scheduler_respects_dependency_capability_and_label_constraints() {
        let graph = ExecutionGraph {
            nodes: vec![
                ExecutionNode {
                    id: "build".to_string(),
                    node_type: ExecutionNodeType::Build,
                    command: Some("cargo build".to_string()),
                    execution_mode: ExecutionMode::Native,
                    inputs: vec!["Cargo.toml".to_string()],
                    outputs: vec!["target".to_string()],
                    cache_key: None,
                },
                ExecutionNode {
                    id: "wasm-test".to_string(),
                    node_type: ExecutionNodeType::Test,
                    command: Some("wasm-test-runner".to_string()),
                    execution_mode: ExecutionMode::Wasm,
                    inputs: vec!["target".to_string()],
                    outputs: vec!["report".to_string()],
                    cache_key: None,
                },
            ],
            edges: vec![ExecutionEdge {
                from: "build".to_string(),
                to: "wasm-test".to_string(),
            }],
        };

        let workers = vec![
            WorkerNode {
                id: "native-a".to_string(),
                capabilities: WorkerCapabilities {
                    wasm: false,
                    native: true,
                    cpu_cores: 8,
                    memory_mb: 8192,
                    labels: vec!["high-cpu".to_string()],
                },
                status: WorkerStatus::Ready,
            },
            WorkerNode {
                id: "wasm-b".to_string(),
                capabilities: WorkerCapabilities {
                    wasm: true,
                    native: false,
                    cpu_cores: 4,
                    memory_mb: 4096,
                    labels: vec!["wasm".to_string()],
                },
                status: WorkerStatus::Ready,
            },
        ];

        let artifact_store = DistributedArtifactStore::default();
        artifact_store.store(ExecutionArtifact {
            key: "build-out".to_string(),
            node_id: "build".to_string(),
            artifact_type: ArtifactType::BuildOutput,
            path: "/tmp/build".to_string(),
            created_at: 1,
        });

        let mut config = DistributedExecutionConfig::default();
        config
            .required_worker_labels
            .insert("wasm-test".to_string(), vec!["wasm".to_string()]);
        config
            .required_artifacts
            .insert("wasm-test".to_string(), vec!["build-out".to_string()]);
        config.lease_ttl_secs = 30;

        let plan = DistributedScheduler.schedule_with_context(
            graph,
            workers,
            &artifact_store,
            &config,
            100,
        );

        assert_eq!(plan.ordered_nodes, vec!["build", "wasm-test"]);
        assert!(plan.unscheduled_nodes.is_empty());
        assert_eq!(plan.assignments.len(), 2);
        assert_eq!(plan.assignments[0].node_id, "build");
        assert_eq!(plan.assignments[0].worker_id, "native-a");
        assert_eq!(plan.assignments[1].node_id, "wasm-test");
        assert_eq!(plan.assignments[1].worker_id, "wasm-b");
        assert_eq!(
            plan.leases.get("wasm-test").map(|lease| lease.expires_at),
            Some(130)
        );
    }

    #[test]
    fn distributed_scheduler_blocks_nodes_when_required_artifacts_are_missing() {
        let graph = ExecutionGraph {
            nodes: vec![
                ExecutionNode {
                    id: "build".to_string(),
                    node_type: ExecutionNodeType::Build,
                    command: Some("cargo build".to_string()),
                    execution_mode: ExecutionMode::Native,
                    inputs: vec!["Cargo.toml".to_string()],
                    outputs: vec!["target".to_string()],
                    cache_key: None,
                },
                ExecutionNode {
                    id: "test".to_string(),
                    node_type: ExecutionNodeType::Test,
                    command: Some("cargo test".to_string()),
                    execution_mode: ExecutionMode::Native,
                    inputs: vec!["target".to_string()],
                    outputs: vec!["report".to_string()],
                    cache_key: None,
                },
            ],
            edges: vec![ExecutionEdge {
                from: "build".to_string(),
                to: "test".to_string(),
            }],
        };
        let workers = vec![WorkerNode {
            id: "native-a".to_string(),
            capabilities: WorkerCapabilities {
                wasm: false,
                native: true,
                cpu_cores: 8,
                memory_mb: 8192,
                labels: vec![],
            },
            status: WorkerStatus::Ready,
        }];
        let artifact_store = DistributedArtifactStore::default();
        let mut config = DistributedExecutionConfig::default();
        config
            .required_artifacts
            .insert("test".to_string(), vec!["missing-build-out".to_string()]);

        let plan = DistributedScheduler.schedule_with_context(
            graph,
            workers,
            &artifact_store,
            &config,
            10,
        );

        assert_eq!(plan.assignments.len(), 1);
        assert_eq!(plan.assignments[0].node_id, "build");
        assert_eq!(plan.unscheduled_nodes, vec!["test"]);
    }

    #[test]
    fn execution_coordinator_reassigns_work_when_worker_goes_offline() {
        let graph = ExecutionGraph {
            nodes: vec![ExecutionNode {
                id: "wasm-build".to_string(),
                node_type: ExecutionNodeType::Build,
                command: Some("wasm-pack build".to_string()),
                execution_mode: ExecutionMode::Wasm,
                inputs: vec!["src".to_string()],
                outputs: vec!["pkg".to_string()],
                cache_key: None,
            }],
            edges: vec![],
        };
        let workers = vec![
            WorkerNode {
                id: "worker-a".to_string(),
                capabilities: WorkerCapabilities {
                    wasm: true,
                    native: false,
                    cpu_cores: 2,
                    memory_mb: 2048,
                    labels: vec!["wasm".to_string()],
                },
                status: WorkerStatus::Ready,
            },
            WorkerNode {
                id: "worker-b".to_string(),
                capabilities: WorkerCapabilities {
                    wasm: true,
                    native: false,
                    cpu_cores: 2,
                    memory_mb: 2048,
                    labels: vec!["wasm".to_string()],
                },
                status: WorkerStatus::Ready,
            },
        ];
        let mut coordinator = ExecutionCoordinator::new(workers, DistributedArtifactStore::default());
        let config = DistributedExecutionConfig::default();

        let initial = coordinator.plan(graph.clone(), &config, 50);
        assert_eq!(initial.assignments[0].worker_id, "worker-a");

        let recovered = coordinator.recover_failed_worker(graph, "worker-a", &config, 55);
        assert_eq!(recovered.assignments[0].worker_id, "worker-b");
    }

    #[test]
    fn static_site_routes_to_wasm_target() {
        let profile = ExecutionProfile {
            fingerprint: RepositoryFingerprint {
                repo_hash: "repo".to_string(),
                lockfile_hash: None,
                dependency_hash: None,
                language_signature: "Unknown".to_string(),
                framework_signature: Some("StaticWeb".to_string()),
            },
            classification: RepositoryClassification {
                class: RepoClass::StaticSite,
                confidence: 0.9,
                primary_runtime: RuntimeType::Static,
                secondary_runtimes: vec![],
            },
            recommended_graph_strategy: GraphStrategy::Linear,
            runtime_affinity: RuntimeAffinity {
                preferred_provider: "WasmExecutionProvider".to_string(),
                fallback_providers: vec!["StaticRuntimeProvider".to_string()],
            },
            wasm_compatibility: WasmCompatibility::Full,
        };
        let node = ExecutionNode {
            id: "serve".to_string(),
            node_type: ExecutionNodeType::StaticServe,
            command: Some("serve .".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec![],
            outputs: vec![],
            cache_key: None,
        };

        match ExecutionRouter::route(&node, &profile) {
            ExecutionTarget::Wasm(spec) => {
                assert!(spec.enabled);
                assert!(spec.wasi);
            }
            other => panic!("expected wasm routing target, got {other:?}"),
        }
    }

    #[test]
    fn wasm_runtime_engine_executes_module_and_reports_exports() {
        let wasm_bytes = parse_str(
            r#"
            (module
                (func (export "run"))
            )
            "#,
        )
        .expect("compile wat");
        let spec = WasmRuntimeSpec {
            enabled: true,
            wasi: true,
            memory_limit_mb: 64,
            cpu_limit_units: 2_000,
            allowed_syscalls: vec!["fd_read".to_string(), "fd_write".to_string()],
        };
        let runtime = WasmRuntimeEngine::new().expect("create wasm runtime engine");
        let result = runtime
            .execute_module(&wasm_bytes, &spec, &WasiContext::default())
            .expect("execute wasm module");

        assert!(result.exported_functions.contains(&"run".to_string()));
    }

    #[test]
    fn wasm_runtime_engine_enforces_memory_limit() {
        let wasm_bytes = parse_str(
            r#"
            (module
                (memory (export "memory") 32)
            )
            "#,
        )
        .expect("compile wat");
        let spec = WasmRuntimeSpec {
            enabled: true,
            wasi: true,
            memory_limit_mb: 1,
            cpu_limit_units: 1_000,
            allowed_syscalls: vec![],
        };
        let runtime = WasmRuntimeEngine::new().expect("create wasm runtime engine");
        let err = runtime
            .execute_module(&wasm_bytes, &spec, &WasiContext::default())
            .expect_err("memory limit should be enforced");

        assert!(matches!(err, RuntimeError::WasmRuntime(message) if message.contains("exceeds limit")));
    }

    #[test]
    fn hybrid_execution_bridge_dispatches_wasm_nodes() {
        let node = ExecutionNode {
            id: "serve".to_string(),
            node_type: ExecutionNodeType::StaticServe,
            command: Some("serve".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec![],
            outputs: vec![],
            cache_key: None,
        };
        let graph = ExecutionGraph {
            nodes: vec![node.clone()],
            edges: vec![],
        }
        .with_cache_keys();
        let ctx = ExecutionContext {
            workspace_id: "ws-1".to_string(),
            repo_path: "/tmp/repo".to_string(),
            analysis: test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb),
            execution_graph: graph,
            wasm_sandbox: None,
            resources: ResourceQuotas {
                max_memory_mb: 512,
                max_cpu_millis: 1000,
            },
            network: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
        };
        let bridge = HybridExecutionBridge::new().expect("create hybrid bridge");
        let wasm_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");

        let handle = bridge
            .dispatch(&node, &ctx, &WasiContext::default(), &wasm_bytes)
            .expect("dispatch wasm node");
        assert!(handle.pid_hint.starts_with("wasm:"));
    }

    #[test]
    fn wasm_execution_provider_uses_compiled_wasm_artifact() {
        let repo_root = temp_dir("wasm-provider-artifact");
        let pkg = repo_root.join("pkg");
        fs::create_dir_all(&pkg).expect("create wasm output dir");
        let wasm_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
        fs::write(pkg.join("app_bg.wasm"), wasm_bytes).expect("write wasm artifact");

        let node = ExecutionNode {
            id: "serve".to_string(),
            node_type: ExecutionNodeType::StaticServe,
            command: Some("serve".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec![],
            outputs: vec!["pkg".to_string()],
            cache_key: None,
        };
        let graph = ExecutionGraph {
            nodes: vec![node],
            edges: vec![],
        }
        .with_cache_keys();
        let ctx = ExecutionContext {
            workspace_id: "ws-1".to_string(),
            repo_path: repo_root.to_string_lossy().to_string(),
            analysis: test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb),
            execution_graph: graph,
            wasm_sandbox: None,
            resources: ResourceQuotas {
                max_memory_mb: 512,
                max_cpu_millis: 1000,
            },
            network: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
        };

        let provider = WasmExecutionProvider;
        let handle = provider.start(&ctx).expect("start wasm provider");
        assert!(handle.pid_hint.starts_with("wasm:"));
    }

    #[test]
    fn wasm_execution_provider_handles_wasm_compile_then_serve_graph() {
        let repo_root = temp_dir("wasm-provider-compile-then-serve");
        let pkg = repo_root.join("pkg");
        fs::create_dir_all(&pkg).expect("create wasm output dir");
        let wasm_bytes = parse_str("(module (func (export \"run\")))").expect("compile wat");
        fs::write(pkg.join("app_bg.wasm"), wasm_bytes).expect("write wasm artifact");

        let graph = ExecutionGraph {
            nodes: vec![
                ExecutionNode {
                    id: "wasm-compile".to_string(),
                    node_type: ExecutionNodeType::WasmCompile,
                    command: Some("wasm-pack build --target web".to_string()),
                    execution_mode: ExecutionMode::Native,
                    inputs: vec!["src".to_string()],
                    outputs: vec!["pkg/app_bg.wasm".to_string()],
                    cache_key: None,
                },
                ExecutionNode {
                    id: "serve".to_string(),
                    node_type: ExecutionNodeType::StaticServe,
                    command: Some("serve .".to_string()),
                    execution_mode: ExecutionMode::Wasm,
                    inputs: vec!["pkg/app_bg.wasm".to_string()],
                    outputs: vec!["http://0.0.0.0:4173/".to_string()],
                    cache_key: None,
                },
            ],
            edges: vec![ExecutionEdge {
                from: "wasm-compile".to_string(),
                to: "serve".to_string(),
            }],
        }
        .with_cache_keys();
        let ctx = ExecutionContext {
            workspace_id: "ws-1".to_string(),
            repo_path: repo_root.to_string_lossy().to_string(),
            analysis: test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb),
            execution_graph: graph,
            wasm_sandbox: None,
            resources: ResourceQuotas {
                max_memory_mb: 512,
                max_cpu_millis: 1000,
            },
            network: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
        };

        let provider = WasmExecutionProvider;
        assert!(provider.can_handle(&ctx));
        let handle = provider
            .start(&ctx)
            .expect("start mixed wasm compile/serve provider");
        assert!(handle.pid_hint.starts_with("wasm:serve:"));
    }

    #[test]
    fn wasm_execution_provider_requires_compiled_wasm_artifact() {
        let repo_root = temp_dir("wasm-provider-missing-artifact");
        let node = ExecutionNode {
            id: "serve".to_string(),
            node_type: ExecutionNodeType::StaticServe,
            command: Some("serve".to_string()),
            execution_mode: ExecutionMode::Wasm,
            inputs: vec![],
            outputs: vec!["pkg".to_string()],
            cache_key: None,
        };
        let graph = ExecutionGraph {
            nodes: vec![node],
            edges: vec![],
        }
        .with_cache_keys();
        let ctx = ExecutionContext {
            workspace_id: "ws-1".to_string(),
            repo_path: repo_root.to_string_lossy().to_string(),
            analysis: test_analysis(graph.clone(), WasmCompatibility::Full, Framework::StaticWeb),
            execution_graph: graph,
            wasm_sandbox: None,
            resources: ResourceQuotas {
                max_memory_mb: 512,
                max_cpu_millis: 1000,
            },
            network: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
        };

        let provider = WasmExecutionProvider;
        let err = provider
            .start(&ctx)
            .expect_err("expected missing artifact to fail");
        assert!(matches!(err, RuntimeError::WasmRuntime(message) if message.contains("no compiled wasm artifact found")));
    }

    #[test]
    fn cache_key_engine_changes_with_execution_mode() {
        let mut graph = ExecutionGraph {
            nodes: vec![ExecutionNode {
                id: "build".to_string(),
                node_type: ExecutionNodeType::Build,
                command: Some("cargo build".to_string()),
                execution_mode: ExecutionMode::Native,
                inputs: vec!["Cargo.toml".to_string()],
                outputs: vec!["target".to_string()],
                cache_key: None,
            }],
            edges: vec![],
        };

        let first = graph.compute_cache_keys();
        graph.nodes[0].execution_mode = ExecutionMode::Wasm;
        let second = graph.compute_cache_keys();
        assert_ne!(first.get("build"), second.get("build"));
    }

    #[test]
    fn workspace_session_tracks_graph_events_and_controls() {
        let mut session = WorkspaceSession::new(
            "session-1",
            "repo-1",
            "graph-1",
            "http://coordinator:8080",
        );
        assert_eq!(session.sync_state, WorkspaceSessionSyncState::Connecting);

        session.record_graph_event(GraphEvent {
            node_id: "build".to_string(),
            event_type: GraphEventType::NodeStarted,
            timestamp: 10,
        });
        assert_eq!(session.stream_for_node("build").len(), 1);

        session.apply_control(ExecutionControl::Pause);
        assert_eq!(session.sync_state, WorkspaceSessionSyncState::Paused);

        session.apply_control(ExecutionControl::RetryNode {
            node_id: "build".to_string(),
        });
        assert_eq!(session.sync_state, WorkspaceSessionSyncState::Live);
        assert_eq!(
            session
                .stream_for_node("build")
                .last()
                .map(|event| event.event_type),
            Some(GraphEventType::NodeQueued)
        );
    }

    #[test]
    fn workspace_session_event_buffers_are_bounded() {
        let mut session = WorkspaceSession::new(
            "session-2",
            "repo-2",
            "graph-2",
            "http://coordinator:8080",
        );
        for index in 0..=SESSION_GRAPH_EVENT_BUFFER_LIMIT {
            session.record_graph_event(GraphEvent {
                node_id: format!("node-{index}"),
                event_type: GraphEventType::NodeStarted,
                timestamp: index as u64,
            });
        }
        for index in 0..=SESSION_WORKER_EVENT_BUFFER_LIMIT {
            session.record_worker_event(WorkerEvent {
                worker_id: "worker-1".to_string(),
                message: format!("event-{index}"),
                timestamp: index as u64,
            });
        }

        assert_eq!(session.graph_events.len(), SESSION_GRAPH_EVENT_BUFFER_LIMIT);
        assert_eq!(session.worker_events.len(), SESSION_WORKER_EVENT_BUFFER_LIMIT);
        assert_eq!(
            session.graph_events.front().map(|event| event.node_id.as_str()),
            Some("node-1")
        );
        assert_eq!(
            session
                .worker_events
                .front()
                .map(|event| event.message.as_str()),
            Some("event-1")
        );
    }

    #[test]
    fn execution_graph_view_applies_live_state_events() {
        let mut graph_view = ExecutionGraphView {
            nodes: vec![UIExecutionNode {
                id: "test".to_string(),
            }],
            ..ExecutionGraphView::default()
        };
        let event = GraphEvent {
            node_id: "test".to_string(),
            event_type: GraphEventType::NodeCompleted,
            timestamp: 25,
        };

        graph_view.apply_graph_event(&event);

        assert_eq!(
            graph_view.live_states.get("test"),
            Some(&GraphEventType::NodeCompleted)
        );
    }

    #[test]
    fn browser_ide_syncs_files_and_streams_logs() {
        let mut ide = BrowserIDE::default();
        ide.sync_file_content("src/lib.rs", "pub fn sample() {}");
        assert_eq!(ide.file_tree.files, vec!["src/lib.rs".to_string()]);
        assert_eq!(ide.editor.active_path.as_deref(), Some("src/lib.rs"));
        assert!(!ide.editor.dirty);

        ide.append_log(WorkerEvent {
            worker_id: "worker-1".to_string(),
            message: "node build started".to_string(),
            timestamp: 100,
        });
        assert_eq!(ide.log_stream.entries.len(), 1);
    }
}
