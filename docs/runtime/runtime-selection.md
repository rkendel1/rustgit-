# Runtime Selection Logic (Code-Grounded)

## Selection path
1. `ExecutionEngine::start` calls `ExecutionRouter::dispatch_start` (`src/lib.rs:12232-12235`)
2. `dispatch_start` calls `select` (`src/lib.rs:3201-3203`)
3. `select` iterates `tier_order()` and ranked providers (`src/lib.rs:3026-3033`, `3084-3127`)

## Priority order
Tier order is hardcoded:
1. `LocalMachine`
2. `LocalDocker`
3. `ExternalProvider`
4. `CloudPartner`
5. `DDockitCloud`  
(`src/lib.rs:3026-3033`)

## Provider ranking inside a tier
Ranking uses proximity + reliability + affinity bonuses - latency - cost (`src/lib.rs:2950-3008`):
- Preferred provider bonus: `+30` (`src/lib.rs:2946`, `2974-2976`)
- Fallback affinity bonus: `+20` (`src/lib.rs:2947`, `2977-2982`)
- Runtime capability match bonus: `+10` (`src/lib.rs:2948`, `2985-2988`)

## Fallback order
- Selected provider is first matched provider in first eligible tier.
- Remaining matched providers become fallback chain (`src/lib.rs:3153-3165`).
- Analyze endpoint separately exposes fallback list from ranked providers (`src/analyze/blueprint_builder.rs:205-210`).

## Health checks
- Router lifecycle calls `provider.health` after `start` (`src/lib.rs:3209-3212`).
- Unhealthy result triggers `provider.stop` and returns error (`src/lib.rs:3219-3228`).
- Workspace runtime readiness also uses port+HTTP probing (`src/lib.rs:13302-13378`, `12440-12523`).

## Retry behavior
- `pnpm install --frozen-lockfile` retries with plain `pnpm install` (`src/lib.rs:13145-13199`).
- Portal API proxy retries with `/api/proxy/...` prefix after upstream 404 (`portal/src/app/api/proxy/[...path]/route.ts:223-239`).
- Git clone retries without `--branch` when branch not found (`src/lib.rs:12715-12745`).

## Escalation logic
- Escalation policy gates external/cloud tiers (`src/lib.rs:3067-3074`).
- Policy defaults allow external + cloud fallback (`src/lib.rs:1939-1946`).
- Escalation trace is recorded in `RuntimeSelection` (`src/lib.rs:3080-3090`, `3195`).
- Workspace router has separate runtime failover priority: Dea -> Docker -> External -> Cloud (`src/lib.rs:11741-11746`, `11916-11924`).

## Configuration/feature flags observed
- Escalation policy fields: `max_local_wait_ms`, `allow_external_fallback`, `allow_cloud_fallback`, `prefer_local` (`src/lib.rs:1932-1937`).
- Badge/runtime preference surface exposes `auto|wasm|docker` but this is UI/config metadata (`src/lib.rs:8232-8242`, `8320-8321`, `9417-9418`).

## Requested escalation chain verification
Requested chain: `Instant -> WASM -> Native -> Workspace -> Remote`.

Verified implementation is different:
- Primary runtime-router escalation is by **tier** (`LocalMachine -> LocalDocker -> ExternalProvider -> CloudPartner -> DDockitCloud`) (`src/lib.rs:3026-3033`).
- Workspace failover priority is `Dea -> Docker -> External -> Cloud` (`src/lib.rs:11741-11746`).
- No concrete `Instant` provider was found.
