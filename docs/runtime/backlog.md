# Runtime Engine — Prioritized PR Backlog

Derived from a review of `docs/runtime/*.md`. Findings are doc-grounded; the docs
themselves flag assumptions. Items marked **[verify]** rest on an inference that
should be confirmed in code before scheduling work.

Priority key: **P0** correctness / user-facing breakage · **P1** architectural
drift with divergence risk · **P2** advertised-but-missing capabilities · **P3**
hygiene. Effort: S (<1d) / M (1–3d) / L (>3d). Risk = blast radius of the change.

---

## P0 — Correctness / user-facing

### PR-1 · Reconcile Docker: provider vs command vs UI
**Problem.** Docker is advertised in three places — portal runtime selector
(`portal/src/app/page.tsx:1121`), runtime-preference config (`auto|wasm|docker`,
`src/lib.rs:8320`, `9417`), and the `LocalDocker` execution tier
(`src/lib.rs:1747`, `3028-3029`) — but **no `DockerExecutionProvider` is
registered** in `WorkspaceManager::new` (`src/lib.rs:12311-12317`). The
`LocalDocker` tier therefore has no provider to match, so selection silently
skips it. Separately, Docker *can* still run by accident: analyze may set the
start command to `docker compose up` / `docker` (`src/analyze/manifest_builder.rs:117-121`,
`305-312`), which the local process launcher executes via `Command::new`
(`src/lib.rs:13260-13279`) — i.e. through the `LocalMachine` path, not
`LocalDocker`. The UI's mental model and the actual execution path disagree.

**Fix (pick one, make it consistent).**
- *Implement:* add a real `DockerExecutionProvider` wired to the `LocalDocker`
  tier, with health/readiness checks; route the command-injection behavior
  through it. **(L)**
- *Remove:* drop the Docker option from the portal selector, the preference
  config enum, and the `LocalDocker` tier, leaving only the documented
  command-injection behavior under `LocalMachine`. **(M)**

**Effort:** M–L · **Risk:** Med (user-visible selector + tier order).

### PR-2 · Connect provider selection to actual process spawn **[verify]**
**Problem.** `NodeRuntimeProvider`, `RustRuntimeProvider`, and
`StaticRuntimeProvider` are stubs whose `start` returns a `pid_hint` only and
spawn nothing (`src/lib.rs:17340-17344`, `17475-17479`, `17516-17520`). The
real process spawning lives separately in `spawn_supervised_process` /
`spawn_run_command` (`src/lib.rs:13207-13279`, `13382-13591`). If the workspace
layer always spawns regardless of which provider the router selected, then
provider selection is cosmetic and the "selected runtime" can differ from what
actually runs — a silent correctness gap.

**Fix.** First confirm whether provider selection feeds the spawn path. If it
doesn't: either (a) make the selected provider drive the spawn, or (b) collapse
the stub providers and treat the workspace spawn layer as the single execution
authority, updating runtime-status reporting (`src/lib.rs:13766-13783`)
accordingly so the reported provider matches reality.

**Effort:** M (investigation) + M–L (fix) · **Risk:** High (core launch path).

### PR-3 · Decide the canonical escalation chain
**Problem.** The requested chain `Instant → WASM → Native → Workspace → Remote`
is not implemented (`docs/runtime/runtime-selection.md`). The router escalates by
tier (`LocalMachine → LocalDocker → ExternalProvider → CloudPartner →
DDockitCloud`, `src/lib.rs:3026-3033`); the workspace router uses a *different*
order (`Dea → Docker → External → Cloud`, `src/lib.rs:11741-11746`). No `Instant`
provider exists.

**Fix.** Decide whether the requested spec or the current implementation is
canonical, then either update the spec/docs or refactor selection to match. This
is partly resolved by PR-4 (the two orderings are the two selection systems).

**Effort:** S (decision/doc) or folds into PR-4 · **Risk:** Low if doc-only.

---

## P1 — Architecture consolidation

### PR-4 · Unify the two runtime-selection systems
**Problem.** `ExecutionRouter` (tier + capability ranking,
`src/lib.rs:3011-3312`) and `WorkspaceRouter` (failover priority,
`src/lib.rs:11741-11746`, `11916-11924`) are both active with different
abstractions and different orderings. Divergent selection decisions are possible.

**Fix.** Define one authoritative selector, or a clear boundary (e.g.
ExecutionRouter for initial selection, WorkspaceRouter strictly for
post-launch failover) with a single shared priority source. Resolves the
ordering mismatch behind PR-3.

**Effort:** L · **Risk:** High (selection semantics).

### PR-5 · Consolidate the two spawn paths
**Problem.** `spawn_supervised_process` (`src/lib.rs:13207-13279`) and
`spawn_run_command` (`src/lib.rs:13382-13591`) both resolve and spawn run
commands, risking divergent env injection, port handling, and retry behavior.

**Fix.** Merge into one spawn path; route language-specific cases (python venv,
pnpm retry) as parameters of the unified path.

**Effort:** M–L · **Risk:** Med–High (launch behavior).

### PR-6 · Single provider taxonomy / mapping
**Problem.** Three parallel provider taxonomies that don't map 1:1: analyze
capability providers (`UserMachine`, `NativeSandbox`, …,
`src/analyze/blueprint_builder.rs:61-124`), workspace failover runtime types
(`Dea`, `Docker`, `External`, `Cloud`), and the execution tier model. Analyze
recommendations don't correspond to registered executable providers
(`src/lib.rs:12311-12317`).

**Fix.** Establish one source of truth mapping analyze capabilities → executable
providers; have analyze recommend only runtimes that map to a registered
provider.

**Effort:** M · **Risk:** Med.

---

## P2 — Advertised but not implemented

### PR-7 · WebSocket/SSE proxy: wire it or drop it
**Problem.** `WorkspaceProxyProtocol` exposes `WebSocket` and `Sse`
(`src/lib.rs:11685-11690`), but no WebSocket upgrade route is wired in the Axum
router — support is metadata-only (`docs/runtime/proxy-flow.md`). Clients
expecting WS proxying fail silently.

**Fix.** Add a WS upgrade route through `WorkspaceRouter::route_workspace_request`,
or remove the enum variants until supported.

**Effort:** M (implement) / S (remove) · **Risk:** Low–Med.

### PR-8 · Go / Java / uv: implement or mark detect-only
**Problem.** Go and uv are partially implemented (detected, command-synthesized,
no dedicated provider); Java is detected but has no provider at all
(`docs/runtime/runtime-capabilities.md` native matrix). Analyze can recommend a
runtime that nothing executes.

**Fix.** Either add execution providers or have analyze flag these as
detect-only so they aren't surfaced as runnable.

**Effort:** L (providers) / S (detect-only flag) · **Risk:** Med.

### PR-9 · CI guard: advertised capability ⇒ registered executing provider
**Problem.** Every drift item above is the same class of bug: something is
advertised (UI option, tier, config enum, analyze capability, test fixture)
without a backing registered provider.

**Fix.** Add a test that asserts each portal selector option, preference-config
variant, tier, and analyze-recommended provider maps to a provider registered in
`WorkspaceManager::new`. This locks in PR-1/6/8 and prevents recurrence.

**Effort:** S–M · **Risk:** Low. *High leverage — consider doing early.*

---

## P3 — Hygiene

### PR-10 · Remove/alias the legacy badge endpoint
`/api/badge/generate` and `/api/badges/generate` both exposed
(`src/bin/server.rs:1076-1077`). Alias one to the other or remove the legacy
route. **(S · Low)**

### PR-11 · Fix test-fixture drift
`browser-wasm` / `fly` / `codespaces` appear in the cache test fixture
(`src/bin/server.rs:1677`) with no provider structs. Update fixtures to reflect
real providers so tests don't enshrine phantom capabilities. **(S · Low)**

### PR-12 · Wire or document dangling analyze fields
Node Version is detected and persisted but has "no direct runtime usage found"
(`src/analyze/manifest_builder.rs:215-273`); Ports are only partially handled
(placeholder in analyze, real probing at launch, `src/analyze/manifest_builder.rs:209`,
`src/lib.rs:12491-12523`). Either consume node version at spawn and tighten
port discovery → injection, or document both as analyze-only metadata. **(S–M · Low)**

### PR-13 · Gate or document Terminal Access / Workspace Sync
Both are stub/partial (`docs/runtime/runtime-capabilities.md`). If roadmap, feature-gate
them; if not, document as unsupported so they aren't mistaken for live
capabilities. **(S · Low)**

---

## Suggested sequencing
1. **PR-9** first (cheap guard that makes the rest safe and surfaces the true
   extent of drift).
2. **PR-2 [verify]** investigation — it may reframe PR-1/4/5.
3. **PR-1** and **PR-6** (resolve the Docker + taxonomy story together).
4. **PR-4 / PR-5** (the heavy consolidations; do once selection semantics are
   decided).
5. P2 and P3 as fill-in / parallelizable work.
