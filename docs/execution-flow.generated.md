# Execution Flow (Generated, Code-Grounded)

## Entry points
- ExecutionEngine::start
- WorkspaceManager::launch
- WorkspaceManager::restart
- WorkspaceManager::stop

## Runtime behavior (derived from call paths)
- ArtifactStore is checked for existing outputs
- CacheKeyEngine computes node keys
- ExecutionGraph is generated via BuildPlanner
- ExecutionProvider is selected via can_handle()
- Provider executes node
- RepositoryAnalysis is produced by repository analyzer
- Result is stored in ArtifactStore

## Workspace state machine transitions (actual transitions only)
- Analyzing -> Failed
- Analyzing -> Planning
- Created -> Materializing
- Failed -> Destroyed
- Failed -> Starting
- Failed -> Stopping
- Materializing -> Analyzing
- Materializing -> Failed
- Paused -> Failed
- Paused -> Running
- Paused -> Stopping
- Planning -> Failed
- Planning -> Starting
- Running -> Failed
- Running -> Paused
- Running -> Stopping
- Starting -> Failed
- Starting -> Running
- Stopped -> Destroyed
- Stopped -> Starting
- Stopping -> Failed
- Stopping -> Stopped

If a transition or call is not listed above, it was not extracted from current code.