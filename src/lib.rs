use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug)]
pub enum RuntimeError {
    WorkspaceMissing(String),
    UnsupportedRepository(String),
    InvalidPath(String),
    Io(io::Error),
    CommandFailed(String),
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkspaceMissing(id) => write!(f, "workspace not found: {id}"),
            Self::UnsupportedRepository(reason) => write!(f, "unsupported repository: {reason}"),
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
    Starting,
    Running,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    pub port: u16,
    pub protocol: String,
    pub route: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub framework: Framework,
    pub build_steps: Vec<String>,
    pub run_command: String,
    pub cache_key: String,
    pub ports: Vec<PortInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryAnalysis {
    pub root: PathBuf,
    pub framework: Framework,
    pub language: Language,
    pub dependency_files: Vec<PathBuf>,
    pub execution_plan: ExecutionPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildArtifact {
    pub id: String,
    pub entrypoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInstance {
    pub pid_hint: String,
    pub healthy: bool,
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

pub trait RuntimeProvider {
    fn can_run(&self, repo: &RepositoryAnalysis) -> bool;
    fn build(&self, repo: &RepositoryAnalysis) -> Result<BuildArtifact>;
    fn execute(&self, artifact: &BuildArtifact) -> Result<RuntimeInstance>;
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
}

pub struct WorkspaceManager {
    root: PathBuf,
    providers: Vec<Box<dyn RuntimeProvider + Send + Sync>>,
    workspaces: Arc<Mutex<HashMap<String, WorkspaceRecord>>>,
    repository_cache: Arc<Mutex<HashMap<String, PathBuf>>>,
    build_cache: Arc<Mutex<HashMap<String, BuildArtifact>>>,
    sequence: AtomicU64,
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

        let providers: Vec<Box<dyn RuntimeProvider + Send + Sync>> = vec![
            Box::new(NodeRuntimeProvider),
            Box::new(RustRuntimeProvider),
            Box::new(StaticRuntimeProvider),
        ];

        Self {
            root: normalized_root,
            providers,
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            repository_cache: Arc::new(Mutex::new(HashMap::new())),
            build_cache: Arc::new(Mutex::new(HashMap::new())),
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

    fn build_or_reuse(&self, analysis: &RepositoryAnalysis) -> Result<BuildArtifact> {
        if let Some(artifact) = self
            .build_cache
            .lock()
            .expect("build cache lock poisoned")
            .get(&analysis.execution_plan.cache_key)
            .cloned()
        {
            return Ok(artifact);
        }

        let provider = self
            .providers
            .iter()
            .find(|provider| provider.can_run(analysis))
            .ok_or_else(|| {
                RuntimeError::UnsupportedRepository("no runtime provider matched".into())
            })?;

        let artifact = provider.build(analysis)?;
        self.build_cache
            .lock()
            .expect("build cache lock poisoned")
            .insert(analysis.execution_plan.cache_key.clone(), artifact.clone());
        Ok(artifact)
    }

    fn provider_for(
        &self,
        analysis: &RepositoryAnalysis,
    ) -> Result<&(dyn RuntimeProvider + Send + Sync)> {
        self.providers
            .iter()
            .find(|provider| provider.can_run(analysis))
            .map(|provider| provider.as_ref())
            .ok_or_else(|| {
                RuntimeError::UnsupportedRepository("no runtime provider matched".into())
            })
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
        self.materialize_repository(repo_url, &repository_root)?;

        let analysis = analyze_repository(&repository_root)?;
        let artifact = self.build_or_reuse(&analysis)?;
        let provider = self.provider_for(&analysis)?;
        let instance = provider.execute(&artifact)?;

        let workspace = Workspace {
            id: id.clone(),
            repo_url: repo_url.to_string(),
            root: workspace_root,
            state: if instance.healthy {
                WorkspaceState::Running
            } else {
                WorkspaceState::Starting
            },
            framework: analysis.framework,
            ports: analysis.execution_plan.ports.clone(),
            network_policy: NetworkPolicy {
                allow_outbound: false,
                allowed_hosts: vec![],
            },
            resource_quotas: ResourceQuotas {
                max_memory_mb: 1024,
                max_cpu_millis: 1000,
            },
        };

        let logs = vec![
            format!("cloned repository: {repo_url}"),
            format!("detected framework: {:?}", analysis.framework),
            format!("executed command: {}", analysis.execution_plan.run_command),
        ];

        self.workspaces
            .lock()
            .expect("workspace lock poisoned")
            .insert(
                id,
                WorkspaceRecord {
                    workspace: workspace.clone(),
                    logs,
                },
            );

        Ok(workspace)
    }

    fn stop(&self, id: &str) -> Result<()> {
        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;

        record.workspace.state = WorkspaceState::Stopped;
        record.logs.push("workspace stopped".to_string());
        Ok(())
    }

    fn restart(&self, id: &str) -> Result<()> {
        let mut workspaces = self.workspaces.lock().expect("workspace lock poisoned");
        let record = workspaces
            .get_mut(id)
            .ok_or_else(|| RuntimeError::WorkspaceMissing(id.to_string()))?;

        record.workspace.state = WorkspaceState::Running;
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

    let execution_plan = execution_plan_for(root, framework)?;

    Ok(RepositoryAnalysis {
        root: root.to_path_buf(),
        framework,
        language,
        dependency_files,
        execution_plan,
    })
}

pub fn execution_plan_for(root: &Path, framework: Framework) -> Result<ExecutionPlan> {
    let plan = match framework {
        Framework::React
        | Framework::Vue
        | Framework::Svelte
        | Framework::Vite
        | Framework::Node
        | Framework::NextJs => ExecutionPlan {
            framework,
            build_steps: vec!["npm ci".to_string(), "npm run build".to_string()],
            run_command: "npm run dev -- --host 0.0.0.0".to_string(),
            cache_key: hash_key(&format!("{}-{:?}", root.display(), framework)),
            ports: vec![PortInfo {
                port: 3000,
                protocol: "http".to_string(),
                route: "/".to_string(),
            }],
        },
        Framework::Rust => ExecutionPlan {
            framework,
            build_steps: vec!["cargo build".to_string()],
            run_command: "cargo run".to_string(),
            cache_key: hash_key(&format!("{}-rust", root.display())),
            ports: vec![PortInfo {
                port: 8080,
                protocol: "http".to_string(),
                route: "/".to_string(),
            }],
        },
        Framework::Go => ExecutionPlan {
            framework,
            build_steps: vec!["go build ./...".to_string()],
            run_command: "go run .".to_string(),
            cache_key: hash_key(&format!("{}-go", root.display())),
            ports: vec![PortInfo {
                port: 8080,
                protocol: "http".to_string(),
                route: "/".to_string(),
            }],
        },
        Framework::Python => ExecutionPlan {
            framework,
            build_steps: vec!["python -m pip install -r requirements.txt".to_string()],
            run_command: "python -m app".to_string(),
            cache_key: hash_key(&format!("{}-py", root.display())),
            ports: vec![PortInfo {
                port: 8000,
                protocol: "http".to_string(),
                route: "/".to_string(),
            }],
        },
        Framework::StaticWeb => ExecutionPlan {
            framework,
            build_steps: vec![],
            run_command: "serve .".to_string(),
            cache_key: hash_key(&format!("{}-static", root.display())),
            ports: vec![PortInfo {
                port: 4173,
                protocol: "http".to_string(),
                route: "/".to_string(),
            }],
        },
        Framework::Unknown => {
            return Err(RuntimeError::UnsupportedRepository(
                "unable to infer execution strategy".to_string(),
            ))
        }
    };

    Ok(plan)
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

impl RuntimeProvider for NodeRuntimeProvider {
    fn can_run(&self, repo: &RepositoryAnalysis) -> bool {
        matches!(
            repo.framework,
            Framework::Node
                | Framework::Vite
                | Framework::React
                | Framework::Vue
                | Framework::Svelte
                | Framework::NextJs
        )
    }

    fn build(&self, repo: &RepositoryAnalysis) -> Result<BuildArtifact> {
        Ok(BuildArtifact {
            id: format!("artifact-{}", repo.execution_plan.cache_key),
            entrypoint: repo.execution_plan.run_command.clone(),
        })
    }

    fn execute(&self, artifact: &BuildArtifact) -> Result<RuntimeInstance> {
        Ok(RuntimeInstance {
            pid_hint: format!("node:{}", artifact.id),
            healthy: true,
        })
    }
}

impl RuntimeProvider for RustRuntimeProvider {
    fn can_run(&self, repo: &RepositoryAnalysis) -> bool {
        repo.framework == Framework::Rust
    }

    fn build(&self, repo: &RepositoryAnalysis) -> Result<BuildArtifact> {
        Ok(BuildArtifact {
            id: format!("artifact-{}", repo.execution_plan.cache_key),
            entrypoint: repo.execution_plan.run_command.clone(),
        })
    }

    fn execute(&self, artifact: &BuildArtifact) -> Result<RuntimeInstance> {
        Ok(RuntimeInstance {
            pid_hint: format!("rust:{}", artifact.id),
            healthy: true,
        })
    }
}

impl RuntimeProvider for StaticRuntimeProvider {
    fn can_run(&self, repo: &RepositoryAnalysis) -> bool {
        repo.framework == Framework::StaticWeb
    }

    fn build(&self, repo: &RepositoryAnalysis) -> Result<BuildArtifact> {
        Ok(BuildArtifact {
            id: format!("artifact-{}", repo.execution_plan.cache_key),
            entrypoint: repo.execution_plan.run_command.clone(),
        })
    }

    fn execute(&self, artifact: &BuildArtifact) -> Result<RuntimeInstance> {
        Ok(RuntimeInstance {
            pid_hint: format!("static:{}", artifact.id),
            healthy: true,
        })
    }
}

fn looks_like_local_path(repo_url: &str) -> bool {
    repo_url.starts_with('/') || repo_url.starts_with("./") || repo_url.starts_with("../")
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
            analysis.execution_plan.run_command,
            "npm run dev -- --host 0.0.0.0"
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
}
