# Feature Maturity Registry

This document is the canonical inventory of implemented system features in this repository, with code-grounded implementation status and maturity grading.

## Maturity model

- **Level 0 — Not Implemented**: design-only, no code evidence
- **Level 1 — Scaffolded**: interfaces/stubs exist, no real execution path
- **Level 2 — Partially Implemented**: isolated execution paths, missing integration/edge coverage
- **Level 3 — Functionally Working**: end-to-end execution in at least one real path
- **Level 4 — Production Ready**: robust, integrated, recoverable/observable

## Registry summary

| Level | Count |
|---|---:|
| 0 | 0 |
| 1 | 0 |
| 2 | 5 |
| 3 | 14 |
| 4 | 2 |

- Last verified: 2026-06-19

## Feature inventory

The following sections enumerate each implemented system feature with direct code evidence, architecture-readiness status, execution-readiness status, and maturity grade.

## Feature: Repository analysis and classification
- **Code evidence:** `src/lib.rs` (`analyze_repository`, `build_repository_fingerprint`, `classify_repository`), tests `detects_*`, `analyze_repository_emits_execution_profile`
- **Architectural readiness:** Fully modeled in analysis + profile pipeline
- **Execution readiness:** Proven by unit tests across frameworks
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** No explicit external fixture corpus beyond golden catalog coverage.

## Feature: Execution graph planning
- **Code evidence:** `src/lib.rs` (`BuildPlanner::build_graph`, `ExecutionGraph`, topology helpers), tests `js_graph_*`, `static_web_graph_includes_wasm_compile_binding_step`
- **Architectural readiness:** Graph model and node/edge types are explicit
- **Execution readiness:** Deterministic graph generation paths are tested
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Dynamic runtime adaptation policies remain limited.

## Feature: Runtime routing and fallback
- **Code evidence:** `src/lib.rs` (`ExecutionRouter`, provider trait impls, `ExecutionEngine::start`)
- **Architectural readiness:** Tiered provider strategy implemented
- **Execution readiness:** Fallback/escalation paths tested (`execution_router_*`, `execution_engine_uses_router_fallback_provider`)
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Production telemetry hooks are lightweight.

## Feature: Workspace lifecycle/state machine
- **Code evidence:** `src/lib.rs` (`WorkspaceManager`, `WorkspaceState`, `can_transition`)
- **Architectural readiness:** Lifecycle states/transitions explicitly defined
- **Execution readiness:** Transition and lifecycle tests pass (`lifecycle_transitions_*`, `state_machine_allows_and_rejects_expected_transitions`)
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Crash-recovery persistence of in-flight state is partial.

## Feature: Artifact caching and wasm artifact bindings
- **Code evidence:** `src/lib.rs` (`ArtifactStore`, `CacheKeyEngine`, wasm artifact binding helpers)
- **Architectural readiness:** Cache identity + artifact model implemented
- **Execution readiness:** Cache and binding round-trip tests pass
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Distributed cache invalidation strategy is basic.

## Feature: Distributed scheduling and worker assignment
- **Code evidence:** `src/lib.rs` (`DistributedScheduler`, `WorkerRegistry`, `ExecutionPlan`)
- **Architectural readiness:** Scheduler and worker capability/lease model implemented
- **Execution readiness:** Prioritization, backpressure, and requeue behavior tested
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** No external queue backend or large-scale soak evidence.

## Feature: Worker heartbeat failure handling
- **Code evidence:** `src/lib.rs` (`ExecutionCoordinator`, heartbeat/lease expiry handling)
- **Architectural readiness:** Failure detection integrated with reassignment
- **Execution readiness:** Failure/reassignment tests pass
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Lacks distributed consensus for multi-coordinator deployments.

## Feature: Wasm runtime execution
- **Code evidence:** `src/lib.rs` (`WasmRuntimeEngine`, `WasmExecutionProvider`)
- **Architectural readiness:** Runtime-specific execution substrate implemented
- **Execution readiness:** Module execution, limits, and compile/serve flow tested
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Sandbox hardening and observability could be deeper.

## Feature: Native runtime and hybrid dispatch bridge
- **Code evidence:** `src/lib.rs` (`NativeRuntimeEngine`, `HybridExecutionBridge`)
- **Architectural readiness:** Native and hybrid paths modeled and wired
- **Execution readiness:** Hybrid wasm dispatch validated in tests
- **Maturity:** **Level 2 — Partially Implemented**
- **Gap notes:** Native execution path breadth is narrower than wasm path.

## Feature: Stable execution URL gateway and rebinding
- **Code evidence:** `src/lib.rs` (`ExecutionGateway`, routing/rebinding helpers)
- **Architectural readiness:** Canonical URL ownership is explicit
- **Execution readiness:** Routing, affinity, and rebinding tests pass
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Multi-region/global edge routing not represented.

## Feature: UCPE/TI control plane contract
- **Code evidence:** `src/lib.rs` (`RestApiSpec` endpoints for unified control plane)
- **Architectural readiness:** Contract-level routes and payloads defined
- **Execution readiness:** Contract tests verify route surface
- **Maturity:** **Level 2 — Partially Implemented**
- **Gap notes:** Mostly API contract + simulation; limited backing runtime services.

## Feature: Dual-surface experience (extension + portal)
- **Code evidence:** `src/lib.rs` (`dual_surface_contract_endpoint`, surface endpoints), `ddockit-extension/*`
- **Architectural readiness:** Shared contract and surface APIs defined
- **Execution readiness:** Endpoint and shared-ID behavior tested
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Frontend integration depth is scaffold-heavy in extension package.

## Feature: Badge generation and seed bootstrap flow
- **Code evidence:** `src/lib.rs` (`badge_generator_endpoint`, `badge_svg`, `seed_repository_endpoint`)
- **Architectural readiness:** Badge and seed routes mapped to execution pipeline
- **Execution readiness:** Endpoint contract tests validate payload/path behavior
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** No production traffic/error budget evidence in-repo.

## Feature: Authentication and OAuth callback contracts
- **Code evidence:** `src/lib.rs` (`auth_login_endpoint`, `auth_me_endpoint`, GitHub/Google callback endpoints)
- **Architectural readiness:** Auth route contracts and payload schema defined
- **Execution readiness:** Contract tests validate callback payloads
- **Maturity:** **Level 2 — Partially Implemented**
- **Gap notes:** Callback exchange is modeled contractually; external provider integration is not end-to-end verified here.

## Feature: Anonymous identity merge for executions
- **Code evidence:** `src/lib.rs` (`claim_anonymous_execution_endpoint`), migration `migrations/0005_anonymous_execution_identity.sql`, `src/postgres_db.rs`
- **Architectural readiness:** Identity model supports user or anonymous ownership
- **Execution readiness:** Anonymous execution and claim behavior tested
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Organization policy enforcement is still minimal.

## Feature: EIDB persistence and history APIs
- **Code evidence:** `src/lib.rs` (`ExecutionIntelligenceDatabase` history endpoints), `src/postgres_db.rs` (`ExecutionIntelligencePostgresStore`), `migrations/*.sql`
- **Architectural readiness:** Schema + store abstraction + endpoints implemented
- **Execution readiness:** Integration tests in `tests/postgres_persistence.rs` pass
- **Maturity:** **Level 4 — Production Ready**
- **Gap notes:** Operational tuning (retention/partitioning) is not exhaustive.

## Feature: Temporal recovery to last-known-good commit
- **Code evidence:** `src/lib.rs` (temporal router + history fallback helpers)
- **Architectural readiness:** Last-good recovery model integrated into routing
- **Execution readiness:** Recovery tests pass (`temporal_execution_router_recovers_last_known_good_commit`)
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Cross-repo recovery orchestration is limited.

## Feature: Healing coordinator and TRE escalation
- **Code evidence:** `src/lib.rs` (`HealingCoordinator`, `RepairExecutor` contract)
- **Architectural readiness:** Failure classification + repair/escalation model exists
- **Execution readiness:** Deterministic recovery and escalation tests pass
- **Maturity:** **Level 2 — Partially Implemented**
- **Gap notes:** Real agent-backed repair providers are abstracted/stub-like.

## Feature: Execution image compiler and warm runtime pool
- **Code evidence:** `src/lib.rs` (`ExecutionImageCompiler`, warm pool manager + endpoints)
- **Architectural readiness:** Compiler and cache binding model implemented
- **Execution readiness:** Deterministic compile and warm-pool lifecycle tests pass
- **Maturity:** **Level 3 — Functionally Working**
- **Gap notes:** Artifact provenance/signing flow is still basic.

## Feature: Usage metering and billing summaries
- **Code evidence:** `src/lib.rs` (`ExecutionMeter`, billing endpoint helpers)
- **Architectural readiness:** Cost model and invoice payload model exist
- **Execution readiness:** Usage and billing contract tests pass
- **Maturity:** **Level 2 — Partially Implemented**
- **Gap notes:** No payment-provider reconciliation flow in repository.

## Feature: Golden-repo quality gate and customer journey runner
- **Code evidence:** `tests/golden_repos/catalog.yaml`, `src/lib.rs` (`load_golden_repository_catalog`, journey runner)
- **Architectural readiness:** Certified fixture schema and journey definitions implemented
- **Execution readiness:** Catalog load + default journey execution tested
- **Maturity:** **Level 4 — Production Ready**
- **Gap notes:** Catalog breadth can continue to expand.

## Designed vs implemented gap highlights

1. **Control plane and auth are contract-strong but integration-light** (mostly validated by route/payload tests).
2. **Healing and native runtime paths are architecturally present but less execution-complete than wasm/eidb paths.**
3. **Feature maturity tracking itself is currently manual** (this document is canonical but not auto-generated).

## Registry maintenance note

- The maturity counts above are a point-in-time snapshot and must be re-verified on every registry update until automation exists.
