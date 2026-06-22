# Provider Inventory (Code-Grounded)

## Actual execution providers (trait implementations)
- `WasmExecutionProvider` (`src/lib.rs:17177-17300`)
- `LocalAgentProvider` (`src/lib.rs:17359-17444`)
- `NodeRuntimeProvider` (`src/lib.rs:17302-17357`)
- `RustRuntimeProvider` (`src/lib.rs:17446-17492`)
- `StaticRuntimeProvider` (`src/lib.rs:17494-17533`)

Registered in runtime engine via `WorkspaceManager::new` (`src/lib.rs:12311-12317`).

## Runtime router/provider framework
- `ExecutionProvider` trait contract (`src/lib.rs:12147-12185`)
- `ExecutionRouter` selection + dispatch (`src/lib.rs:3011-3312`)
- `ExecutionEngine` orchestration (`src/lib.rs:12208-12297`)

## Additional provider-like models found
- Analyze capability providers (metadata): `UserMachine`, `WASM`, `NativeSandbox`, `RemoteWorkspace`, `ContainerizedBuild`, `LongRunningWorkspace` (`src/analyze/blueprint_builder.rs:61-124`)
- Workspace failover runtime types: `Dea`, `Docker`, `External`, `Cloud` (`src/lib.rs:11633-11682`, `src/lib.rs:11741-11746`)
- Execution tier model: `LocalMachine`, `LocalDocker`, `ExternalProvider`, `CloudPartner`, `DDockitCloud` (`src/lib.rs:1745-1751`, `src/lib.rs:3026-3033`)

## Terms requested vs evidence
- **WASM**: implemented as runtime engine + provider (`src/lib.rs:2040-2166`, `src/lib.rs:17177-17300`)
- **Native**: implemented as `NativeRuntimeEngine` and local process spawn (`src/lib.rs:2602-2629`, `src/lib.rs:13267-13279`)
- **Static**: implemented provider (`src/lib.rs:17494-17533`)
- **Docker**: represented in tier/runtime metadata and start-command inference (`src/lib.rs:1747`, `src/analyze/manifest_builder.rs:117-121`, `305-312`)
- **Codespaces**: only appears in test fixture payload (`src/bin/server.rs:1677`)
- **Workspace**: implemented (`WorkspaceManager`, `WorkspaceRouter`) (`src/lib.rs:12213-12219`, `11749-11925`)
- **Extension**: CORS origin allowance + auth callback URL (`portal/src/app/api/proxy/[...path]/route.ts:101-106`, `src/lib.rs:8434`)
- **Browser**: portal/frontend and app proxy implementation (`portal/src/app/page.tsx`, `portal/src/app/api/app-proxy/[id]/[[...path]]/route.ts`)
- **Sandbox**: WASM sandbox struct + limits (`src/lib.rs:1987-1991`, `17809-17815`)
- **Instant Runtime**: no concrete provider named “Instant” found
- **Executor/Launcher/Proxy/Runtime**: implemented via `ExecutionEngine`, `launch_execution`, `WorkspaceProxy`, `ExecutionGateway` (`src/lib.rs:12208-12235`, `src/bin/server.rs:424-446`, `src/lib.rs:6623-6645`, `6694-6739`)
