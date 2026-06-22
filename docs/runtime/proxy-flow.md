# Proxy Verification (Code-Grounded)

## Application proxy (`/api/app-proxy/...`)
- Validates workspace/runtime readiness via backend calls (`portal/src/app/api/app-proxy/[id]/[[...path]]/route.ts:74-103`).
- Forwards requests to `http://127.0.0.1:{port}` (`portal/src/app/api/app-proxy/[id]/[[...path]]/route.ts:106-127`).
- Uses 500ms fetch timeout (`MAX_PROBE_TIMEOUT_MS`) (`portal/src/app/api/app-proxy/[id]/[[...path]]/route.ts:8`, `118-123`).

## Workspace proxy model (backend)
- `WorkspaceProxy` stores workspace -> worker target bindings (`src/lib.rs:6623-6645`).
- `WorkspaceRouter::route_request` resolves workspace and proxy target (`src/lib.rs:11898-11914`).
- `route_workspace_request` updates runtime health timestamps (`src/lib.rs:11881-11895`).

## API proxy (`/api/proxy/...`)
- Generic upstream forwarder with request timeout (`DEFAULT_TIMEOUT_MS=30s`, `ANALYZE_TIMEOUT_MS=295s`) (`portal/src/app/api/proxy/[...path]/route.ts:20-31`).
- Retries once with `api/proxy/` prefix on upstream 404 (`portal/src/app/api/proxy/[...path]/route.ts:223-239`).
- Supports CORS allowlist including browser extension origins (`portal/src/app/api/proxy/[...path]/route.ts:77-109`).

## Health checks / readiness detection
- Workspace runtime readiness checks parse listening ports from `/proc`, then HTTP probe (`src/lib.rs:12440-12523`).
- Runtime initialization loop with 30s timeout (`src/lib.rs:13302-13378`).
- `WorkspaceHealthMonitor` maps health inputs to Running/Degraded state (`src/lib.rs:11935-11959`).

## Port discovery
- Reserved prebound port from `127.0.0.1:0` (`src/lib.rs:13026-13030`).
- Runtime status exposes requested and actual ports (`src/lib.rs:13749-13757`).

## Retry logic
- Proxy retry on 404 (see above).
- Workspace recovery helpers support restart/migrate state transitions (`src/lib.rs:11965-11980`).

## Streaming / WebSocket support
- Streamed log ingestion from child stdout/stderr is implemented (`src/lib.rs:12416-12437`).
- `WorkspaceProxyProtocol` enum includes `WebSocket` and `Sse` (`src/lib.rs:11685-11690`).
- No concrete WebSocket route/upgrade handler found in `src/bin/server.rs`.
