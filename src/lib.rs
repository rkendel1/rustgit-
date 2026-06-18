use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::{json, Value};

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
    DevServer,
    Test,
    StaticServe,
    CustomCommand,
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
        let mut normalized = self.ordered_node_ids();
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
        self.nodes
            .iter()
            .map(|node| (node.id.clone(), CacheKeyEngine::compute_node_key(node, self)))
            .collect()
    }

    pub fn with_cache_keys(mut self) -> Self {
        let keys = self.compute_cache_keys();
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryAnalysis {
    pub root: PathBuf,
    pub framework: Framework,
    pub language: Language,
    pub dependency_files: Vec<PathBuf>,
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
    pub path: String,
    pub created_at: u64,
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

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.json"))
    }
}

pub struct CacheKeyEngine;

impl CacheKeyEngine {
    /// Computes a deterministic cache key for one node by hashing:
    /// node type, command, immediate graph position, graph/repository hash,
    /// and an environment fingerprint stable for a given runtime configuration.
    pub fn compute_node_key(node: &ExecutionNode, graph: &ExecutionGraph) -> String {
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

        let repo_hash = graph.cache_key();
        let env_hash = hash_key(&format!(
            "{}|{}|{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            // Optional cache namespace partitioning (for example dev/staging/prod).
            std::env::var("RUSTGIT_RUNTIME_ENV").unwrap_or_default()
        ));

        hash_key(&format!(
            "{}|{}|{}|{}|{}",
            node_type_name(node.node_type),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionContext {
    pub workspace_id: String,
    pub repo_path: String,
    pub analysis: RepositoryAnalysis,
    pub execution_graph: ExecutionGraph,
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
            self.artifact_store.put(ExecutionArtifact {
                key: key.clone(),
                node_id: node.id.clone(),
                path: artifact_path.to_string_lossy().to_string(),
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
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

    if framework == Framework::Unknown {
        return Err(RuntimeError::UnsupportedRepository(
            "unable to infer execution strategy".to_string(),
        ));
    }

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

    let mut analysis = RepositoryAnalysis {
        root: root.to_path_buf(),
        framework,
        language,
        dependency_files,
        build_intelligence,
        execution_graph: ExecutionGraph::default(),
    };
    analysis.execution_graph = BuildPlanner::build_graph(&analysis).with_cache_keys();

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
                    inputs: build.outputs.clone(),
                    outputs: vec!["http://0.0.0.0:3000/".to_string()],
                    cache_key: None,
                };
                let test = ExecutionNode {
                    id: "test".to_string(),
                    node_type: ExecutionNodeType::Test,
                    command: Some(js_script("test", &js_test_fallback)),
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
                        inputs: vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()],
                        outputs: vec!["target".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("cargo run".to_string()),
                        inputs: vec!["target".to_string()],
                        outputs: vec!["http://0.0.0.0:8080/".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("cargo test".to_string()),
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
                        inputs: vec!["go.mod".to_string(), "go.sum".to_string()],
                        outputs: vec!["go-build-cache".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("go run .".to_string()),
                        inputs: vec!["go-build-cache".to_string()],
                        outputs: vec!["http://0.0.0.0:8080/".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("go test ./...".to_string()),
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
                        inputs: vec!["requirements.txt|pyproject.toml".to_string()],
                        outputs: vec!["site-packages".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "dev".to_string(),
                        node_type: ExecutionNodeType::DevServer,
                        command: Some("python -m app".to_string()),
                        inputs: vec!["site-packages".to_string()],
                        outputs: vec!["http://0.0.0.0:8000/".to_string()],
                        cache_key: None,
                    },
                    ExecutionNode {
                        id: "test".to_string(),
                        node_type: ExecutionNodeType::Test,
                        command: Some("python -m pytest".to_string()),
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
                nodes: vec![ExecutionNode {
                    id: "serve".to_string(),
                    node_type: ExecutionNodeType::StaticServe,
                    command: Some("serve .".to_string()),
                    inputs: vec!["index.html".to_string()],
                    outputs: vec!["http://0.0.0.0:4173/".to_string()],
                    cache_key: None,
                }],
                edges: vec![],
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
        ExecutionNodeType::DevServer => "dev-server",
        ExecutionNodeType::Test => "test",
        ExecutionNodeType::StaticServe => "static-serve",
        ExecutionNodeType::CustomCommand => "custom-command",
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            graph.primary_run_command().as_deref(),
            Some("npm run dev")
        );
        assert!(graph.nodes.iter().all(|node| node.cache_key.is_some()));
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
        assert_eq!(analysis.build_intelligence.package_manager.as_deref(), Some("pnpm"));
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
    fn artifact_store_round_trips_execution_artifact() {
        let root = temp_dir("artifact-store");
        let store = ArtifactStore::new(root.clone());
        let artifact = ExecutionArtifact {
            key: "cache-key".to_string(),
            node_id: "build".to_string(),
            path: root.join("build-output").to_string_lossy().to_string(),
            created_at: 42,
        };

        store.put(artifact.clone());

        assert!(store.exists("cache-key"));
        assert_eq!(store.get("cache-key"), Some(artifact));
    }
}
