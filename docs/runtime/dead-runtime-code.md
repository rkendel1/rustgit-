# Dead Runtime Code / Drift Report (Code-Grounded)

## Unused or metadata-only providers
1. **Analyze capability provider names are not runtime-engine provider IDs**
   - Analyze names: `UserMachine`, `NativeSandbox`, `RemoteWorkspace`, `ContainerizedBuild`, `LongRunningWorkspace` (`src/analyze/blueprint_builder.rs:61-124`)
   - Runtime engine registered providers: `WasmExecutionProvider`, `LocalAgentProvider`, `NodeRuntimeProvider`, `RustRuntimeProvider`, `StaticRuntimeProvider` (`src/lib.rs:12311-12317`)
   - Status: duplicated provider-model layer; analyze metadata does not map 1:1 to executable provider structs.

2. **`browser-wasm` / `fly` / `codespaces` fallback claims**
   - Found in test fixture payload (`src/bin/server.rs:1677`) but no corresponding provider structs registered in `WorkspaceManager::new` (`src/lib.rs:12311-12317`).
   - Status: documentation/test-fixture drift.

## Duplicate runtime logic
1. **Two spawn paths**
   - `spawn_supervised_process` (`src/lib.rs:13207-13279`)
   - `spawn_run_command` (`src/lib.rs:13382-13591`)
   - Both resolve and spawn run commands, creating overlap in launch behavior.

2. **Two runtime selection systems**
   - `ExecutionRouter` provider/tier selection (`src/lib.rs:3011-3312`)
   - `WorkspaceRouter` runtime failover priority (`src/lib.rs:11741-11746`, `11916-11924`)
   - Status: both active, but different abstractions.

## Deprecated/legacy path signals
- Legacy badge endpoint still exposed alongside new one (`/api/badge/generate` and `/api/badges/generate`) (`src/bin/server.rs:1076-1077`).

## Feature/config flags with possible drift
- Runtime preference config variants include `docker` (`src/lib.rs:8320`, `9417`) while no dedicated docker execution provider struct is registered in runtime provider list (`src/lib.rs:12311-12317`).

## Never-referenced modules (within runtime scope)
- No safe claim made here without full-program reference graph tooling; this report only flags concrete mismatches verified by direct source evidence above.
