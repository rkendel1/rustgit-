# System Architecture (Generated, Code-Grounded)

## 1. Actual module structure
- Crate layout: architecture_docs, tests
- Module declarations discovered directly from source declarations only.

## 2. Real execution components
- ExecutionEngine: IMPLEMENTED
- ExecutionProvider: IMPLEMENTED
- WorkspaceManager: IMPLEMENTED
- ExecutionGraph: IMPLEMENTED
- ArtifactStore: IMPLEMENTED
- RepositoryRegistry: IMPLEMENTED

## 3. Real data model inventory
- Structs (76): ApplicationTopology, ArtifactStore, BrowserIDE, BuildArtifact, BuildIntelligence, BuildPlanner, CacheKeyEngine, DistributedArtifactStore, DistributedExecutionConfig, DistributedScheduler, ExecutionArtifact, ExecutionContext, ExecutionCoordinator, ExecutionEdge, ExecutionEngine, ExecutionGraph, ExecutionGraphView, ExecutionNode, ExecutionPlan, ExecutionProfile, ExecutionRouter, FileTree, GraphEvent, GraphPartition, HealthStatus, HybridExecutionBridge, LogStream, MonacoEditor, NativeExecutionRequest, NativeRuntimeEngine, NetworkPolicy, NodeAssignment, NodeLease, NodeRuntimeProvider, PortInfo, ProcessHandle, RepoDelta, RepositoryAnalysis, RepositoryClassification, RepositoryFingerprint, RepositoryRegistry, RepositoryRegistryState, ResourceQuotas, RestApiSpec, RuntimeAffinity, RuntimeSelection, RustRuntimeProvider, ServiceDefinition, ServiceDependency, StartupOrder, StaticRuntimeProvider, TerminalSession, UIExecutionEdge, UIExecutionNode, VirtualFileSystem, WasiContext, WasmArtifact, WasmArtifactBinding, WasmExecutionContext, WasmExecutionEnvironment, WasmExecutionProvider, WasmExecutionResult, WasmModule, WasmRuntimeEngine, WasmRuntimeSpec, WasmSandbox, WorkerCapabilities, WorkerEvent, WorkerNode, WorkerQueue, WorkerRegistry, Workspace, WorkspaceManager, WorkspaceRecord, WorkspaceSession, WorkspaceSnapshot
- Traits (2): ExecutionProvider, WasmWorkspace
- Enums (18): ArtifactType, ExecutionControl, ExecutionMode, ExecutionNodeType, ExecutionTarget, Framework, GraphEventType, GraphStrategy, Language, ReadinessCheck, RepoClass, RuntimeError, RuntimeType, ServiceRole, WasmCompatibility, WorkerStatus, WorkspaceSessionSyncState, WorkspaceState

## 4. Call graph edges (code-reachable patterns)
- ExecutionEngine::prime_artifacts -> ArtifactStore::exists
- ExecutionEngine::start -> ExecutionRouter::dispatch_start
- ExecutionRouter::dispatch_start -> ExecutionProvider::health
- ExecutionRouter::dispatch_start -> ExecutionProvider::prepare
- ExecutionRouter::dispatch_start -> ExecutionProvider::start
- WorkspaceManager::launch -> ExecutionEngine::start
- WorkspaceManager::launch -> analyze_repository
- analyze_repository -> BuildPlanner::build_graph

## 5. Runtime abstraction truth
- WasmExecutionProvider: IMPLEMENTED
- NativeRuntimeEngine: IMPLEMENTED
- ExecutionRouter: IMPLEMENTED

All statements above are derived from declarations or call patterns in `src/lib.rs` only.