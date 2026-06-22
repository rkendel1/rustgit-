# Execution Flow (Code-Grounded)

## End-to-end runtime flow (portal-triggered)

1. **Portal UI**  
   `handleRun()` posts to `/api/proxy/api/v1/executions` (`portal/src/app/page.tsx:684-712`)
2. **Portal API proxy**  
   Forwards request to backend API base URL with timeout/retry behavior (`portal/src/app/api/proxy/[...path]/route.ts:165-239`)
3. **Backend execution endpoint**  
   `/api/proxy/api/v1/executions` route is bound to `launch_execution` (`src/bin/server.rs:1067-1073`)
4. **Workspace launch orchestration**  
   `launch_execution` calls `begin_launch_with_overrides` then background `complete_launch_with_overrides` (`src/bin/server.rs:424-446`)
5. **Repository analyze + planning**  
   `complete_launch_with_overrides` analyzes repo and builds `ExecutionContext` (`src/lib.rs:12893-12917`)
6. **Execution engine dispatch**  
   `ExecutionEngine::start` -> `ExecutionRouter::dispatch_start` (`src/lib.rs:12232-12235`)
7. **Provider selection**  
   `ExecutionRouter::select` ranks providers by tier + capability and chooses first match (`src/lib.rs:3026-3127`, `src/lib.rs:3153-3196`)
8. **Provider lifecycle**  
   `prepare -> start -> health` called in `dispatch_start` (`src/lib.rs:3209-3212`)
9. **Workspace process runtime**  
   Supervised local process is spawned from resolved run command (`src/lib.rs:13207-13279`)
10. **Readiness + runtime exposure**  
    readiness loop (`src/lib.rs:13302-13378`) and runtime status API (`src/bin/server.rs:798-808`)
11. **Application proxy**  
    app proxy forwards to `127.0.0.1:{actual_port}` once ready (`portal/src/app/api/app-proxy/[id]/[[...path]]/route.ts:79-127`)

## Arrow map with code locations
- Portal -> Run Request: `portal/src/app/page.tsx:684-712`
- Run Request -> Proxy Forward: `portal/src/app/api/proxy/[...path]/route.ts:165-239`
- Proxy Forward -> Backend Route: `src/bin/server.rs:1067-1073`
- Backend Route -> Workspace Launch: `src/bin/server.rs:424-446`
- Workspace Launch -> Analyze/Planner: `src/lib.rs:12893-12920`
- Analyze/Planner -> Execution Engine: `src/lib.rs:12232-12235`
- Execution Engine -> Runtime Provider Selection: `src/lib.rs:3077-3196`
- Provider Selection -> Provider Start/Health: `src/lib.rs:3209-3229`
- Provider Start -> Local Workspace Runtime: `src/lib.rs:13207-13279`
- Local Workspace Runtime -> Readiness: `src/lib.rs:13302-13378`
- Readiness -> App Proxy -> Application: `portal/src/app/api/app-proxy/[id]/[[...path]]/route.ts:79-127`

## Notes
- Analyze-only execution planning artifacts (`.execution-plan.json`, `.runtime-capabilities.json`, `.launch-plan.json`) are written by `AnalyzeEngine` (`src/analyze/analyzer.rs:150-175`) and do not directly invoke a runtime.
