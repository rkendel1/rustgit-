# Runtime Capability Registry (Code-Grounded)

## Scope
Verified from source only. No new functionality was added.

## Provider registry found in analyze pipeline

| Provider | Purpose | Current Status | Entry Point | Dependencies | Launch Flow | Supported Frameworks | Supported Languages | Supported Package Managers | Health Checks | Fallback Behavior | Current Production Usage |
|---|---|---|---|---|---|---|---|---|---|---|---|
| UserMachine | Top-priority recommended provider in analyze blueprint | Implemented (analyze metadata) | `capability_registry()` (`src/analyze/blueprint_builder.rs:61-75`) | Analyze engine + blueprint builder (`src/analyze/analyzer.rs:64-79`) | `build_execution_plan` sorts by priority (`src/analyze/blueprint_builder.rs:145-188`) | node/vite/react/nextjs/python/rust/docker/native (`src/analyze/blueprint_builder.rs:69-71`) | implied by supports list | implied by runtime/package manager detection (`src/analyze/runtime_detector.rs:34-73`) | Static `healthy: true` in registry (`src/analyze/blueprint_builder.rs:67-68`) | Fallback list is the remaining ranked providers (`src/analyze/blueprint_builder.rs:205-210`) | Returned by `/api/analyze` payload and tests (`src/analyze/analyzer.rs:109-114`, `src/bin/server.rs:1617-1640`) |
| WASM | WASM suitability + recommendation metadata | Implemented (analyze metadata) | `capability_registry()` (`src/analyze/blueprint_builder.rs:77-86`) | Same as above | Same as above | react/vite/svelte/static/node (`src/analyze/blueprint_builder.rs:82-85`) | implied node/web + static | n/a | Static `healthy: true` (`src/analyze/blueprint_builder.rs:80`) | Included/excluded by filter in `build_execution_plan` (`src/analyze/blueprint_builder.rs:152-159`) | Returned by runtime capabilities endpoint (`src/bin/server.rs:401-405`, `src/bin/server.rs:1275-1287`) |
| NativeSandbox | Native-sandbox recommendation metadata | Implemented (analyze metadata) | `capability_registry()` (`src/analyze/blueprint_builder.rs:88-99`) | Same as above | Same as above | node/python/fastapi/django/bun/pnpm/rust/go (`src/analyze/blueprint_builder.rs:93-95`) | node/python/rust/go | bun/pnpm | Static `healthy: true` (`src/analyze/blueprint_builder.rs:91`) | Ranked fallback (`src/analyze/blueprint_builder.rs:170-184`) | Exposed in analyze capability JSON (`src/analyze/analyzer.rs:111-113`) |
| RemoteWorkspace | Catch-all fallback provider for analyze planning | Implemented (analyze metadata) | `capability_registry()` (`src/analyze/blueprint_builder.rs:101-107`) | Same as above | Chosen when no providers match (`src/analyze/blueprint_builder.rs:163-168`) | everything | everything | everything | Static `healthy: true` (`src/analyze/blueprint_builder.rs:104`) | Explicit default fallback/provider (`src/analyze/blueprint_builder.rs:166-167`, `204`) | Present in analyze artifacts (`src/analyze/analyzer.rs:150-175`) |
| ContainerizedBuild | Analyze metadata for docker/system packages | Implemented (analyze metadata) | `capability_registry()` (`src/analyze/blueprint_builder.rs:109-115`) | Same as above | Ranked by priority | docker/system-packages (`src/analyze/blueprint_builder.rs:114`) | n/a | docker/system packages | Static `healthy: true` | Ranked fallback | Included in capability payload (`src/analyze/analyzer.rs:111-113`) |
| LongRunningWorkspace | Lowest-priority catch-all metadata | Implemented (analyze metadata) | `capability_registry()` (`src/analyze/blueprint_builder.rs:117-123`) | Same as above | Ranked by priority | everything | everything | everything | Static `healthy: true` | Ranked fallback | Included in capability payload (`src/analyze/analyzer.rs:111-113`) |

## Actual execution providers wired into workspace runtime

| Provider | Purpose | Current Status | Entry Point | Dependencies | Launch Flow | Supported Frameworks/Languages | Package Managers | Health Checks | Fallback Behavior | Current Production Usage |
|---|---|---|---|---|---|---|---|---|---|---|
| WasmExecutionProvider | Executes wasm-targeted graph nodes; can dispatch native/static nodes with commands | Implemented | Registered in `WorkspaceManager::new` (`src/lib.rs:12311-12317`); impl (`src/lib.rs:17177-17300`) | `WasmRuntimeEngine`, compiled wasm artifacts, `NativeRuntimeEngine` (`src/lib.rs:17217-17279`) | `ExecutionEngine::start -> ExecutionRouter::dispatch_start -> provider.prepare/start/health` (`src/lib.rs:12232-12235`, `3201-3212`) | Requires `runtime_spec.requires_wasm` and graph compatibility (`src/lib.rs:17190-17202`) | Indirect (from execution graph commands) | Returns healthy=true (`src/lib.rs:17294-17299`) | Router chooses provider by tier/rank, builds fallback chain (`src/lib.rs:3077-3196`) | Used by runtime engine; provider inferred in runtime status by `pid_hint` (`src/lib.rs:13766-13783`) |
| LocalAgentProvider | Dispatches signed execution graph to local distributed execution agent | Implemented | Registered in `WorkspaceManager::new` (`src/lib.rs:12313`); impl (`src/lib.rs:17359-17444`) | `DistributedExecutionAgent` (`src/lib.rs:17143-17174`) | `start` signs + assigns graph (`src/lib.rs:17414-17424`) | Agent-driven (`agent.can_execute`) (`src/lib.rs:17405-17408`) | Agent capability-based | Health = trusted agent check (`src/lib.rs:17433-17442`) | Router fallback chain (`src/lib.rs:3153-3165`) | Wired by default in runtime provider list (`src/lib.rs:12311-12317`) |
| NodeRuntimeProvider | Node/js framework runtime provider | Implemented (stub process handle) | Registered in `WorkspaceManager::new` (`src/lib.rs:12314`); impl (`src/lib.rs:17302-17357`) | Router + execution graph cache key | `start` returns `pid_hint` only (`src/lib.rs:17340-17344`) | JS/TS + node web frameworks (`src/lib.rs:17315-17333`) | n/a in provider impl | Returns healthy=true (`src/lib.rs:17351-17355`) | Router fallback chain | Wired by default (`src/lib.rs:12314`) |
| RustRuntimeProvider | Rust runtime provider | Implemented (stub process handle) | Registered in `WorkspaceManager::new` (`src/lib.rs:12315`); impl (`src/lib.rs:17446-17492`) | Router + execution graph cache key | `start` returns `pid_hint` only (`src/lib.rs:17475-17479`) | Rust + Axum/Actix/Rocket/Leptos (`src/lib.rs:17459-17468`) | n/a in provider impl | Returns healthy=true (`src/lib.rs:17486-17490`) | Router fallback chain | Wired by default (`src/lib.rs:12315`) |
| StaticRuntimeProvider | Static-web runtime provider | Implemented (stub process handle) | Registered in `WorkspaceManager::new` (`src/lib.rs:12316`); impl (`src/lib.rs:17494-17533`) | static framework detection | `start` returns `pid_hint` only (`src/lib.rs:17516-17520`) | Static web frameworks (`src/lib.rs:17507-17510`) | n/a | Returns healthy=true (`src/lib.rs:17527-17531`) | Router fallback chain | Wired by default (`src/lib.rs:12316`) |

## Native execution verification matrix

| Runtime/tool | Status | Evidence |
|---|---|---|
| node | Implemented | Runtime detection + node provider + spawn path (`src/analyze/runtime_detector.rs:53-61`, `src/lib.rs:17302-17344`, `src/lib.rs:13267-13279`) |
| pnpm | Implemented | Lockfile detection + install command + retry (`src/analyze/runtime_detector.rs:11`, `src/analyze/registry.rs:136-142`, `src/lib.rs:14617`, `src/lib.rs:13145-13199`) |
| npm | Implemented | package-lock detection + install/build inference (`src/analyze/runtime_detector.rs:12`, `src/analyze/registry.rs:143-149`, `src/lib.rs:14620`, `src/lib.rs:13267-13279`) |
| bun | Implemented | bun lockfile/runtime mapping (`src/analyze/runtime_detector.rs:9-10`, `src/analyze/registry.rs:129-135`) |
| python | Implemented | runtime detection + venv + pip + python spawn (`src/analyze/runtime_detector.rs:14-15`, `src/lib.rs:13392-13457`, `src/lib.rs:13517-13533`) |
| uv | Partially implemented | Detected/inferred in repository analysis + command synthesis, but no dedicated execution provider (`src/lib.rs:15316`, `src/lib.rs:15387-15389`, `src/lib.rs:14623`) |
| cargo | Implemented | Cargo detection + command synthesis + Rust provider (`src/analyze/runtime_detector.rs:16`, `src/analyze/registry.rs:45-53`, `src/lib.rs:17446-17492`) |
| go | Partially implemented | Go detected/inferred; no dedicated Go execution provider type registered in `WorkspaceManager::new` (`src/analyze/runtime_detector.rs:17`, `src/analyze/registry.rs:55-63`, `src/lib.rs:12311-12317`) |
| java | Referenced only | Java detected in analyze runtime/framework registry but no java provider in runtime provider list (`src/analyze/runtime_detector.rs:18`, `src/analyze/framework_detector.rs:68-75`, `src/lib.rs:12311-12317`) |

## User machine execution verification

| Capability | Status | Evidence |
|---|---|---|
| Portal | Production path present | Portal run/analyze requests route through proxy API (`portal/src/app/page.tsx:657`, `portal/src/app/page.tsx:696-712`) |
| Browser Extension | Experimental/integration surface only | Extension origins are explicitly allowed in portal proxy (`portal/src/app/api/proxy/[...path]/route.ts:101-106`) and auth callback emits `chrome-extension://...` URL (`src/lib.rs:8434`) |
| Local Runtime | Production path present | Workspace spawns local process from resolved run command (`src/lib.rs:13207-13279`) |
| Native Bridge | Not implemented (no dedicated bridge module found) | No dedicated `native bridge` implementation; execution goes via local process spawn and providers (`src/lib.rs:13267-13279`, `src/lib.rs:12311-12317`) |
| Filesystem Access | Production path present | Workspace filesystem API routes + virtual FS access (`src/bin/server.rs:810-907`, `src/lib.rs:13943-13949`) |
| Workspace Sync | Partial | Workspace files can be read/written via API, but no separate sync daemon/protocol found (`src/bin/server.rs:810-907`) |
| Terminal Access | Stub/partial | Runtime status exposes stdout/stderr logs; no interactive terminal protocol route found (`src/lib.rs:13690-13762`, `src/bin/server.rs:786-796`) |

## Assumptions / unknowns
- “Current production usage” is inferred only from wiring/routes in this repository (no live telemetry source in-code).
- A provider appearing in analyze payloads is treated as metadata capability unless it is also registered in `WorkspaceManager::new`.
