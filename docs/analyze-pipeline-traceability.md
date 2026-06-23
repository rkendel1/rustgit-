# Analyze Pipeline Simplification Traceability

This document maps issue requirements to implementation locations.

## Phase 1 — Single Analyze Engine
- `POST /api/analyze` added as canonical route in `/home/runner/work/rustgit/rustgit/src/bin/server.rs`.
- Legacy analyze routes now call the same handler (`analyze_repository`) for a single source of truth.

## Phase 2 — File-based detection
- File lock/runtime detection in `/home/runner/work/rustgit/rustgit/src/analyze/runtime_detector.rs`.
- Framework/language detection in `/home/runner/work/rustgit/rustgit/src/analyze/framework_detector.rs`.

## Phase 3 — Manifest-first
- Manifest emitted to `.ddockit/manifest.json` by `/home/runner/work/rustgit/rustgit/src/analyze/manifest_builder.rs`.

## Phase 4 — Multi-stage analyzer
- Staged execution and per-stage timeout trace metadata in `/home/runner/work/rustgit/rustgit/src/analyze/analyzer.rs`.

## Phase 5 — No workspace recursion
- Analyze handler no longer launches workspace providers; it directly inspects repository files in `/home/runner/work/rustgit/rustgit/src/bin/server.rs`.

## Phase 6 — Progressive enhancement
- Analyze response returns runtime manifest metadata immediately, and execution planning/provider intelligence is generated in background jobs from `/home/runner/work/rustgit/rustgit/src/bin/server.rs` and `/home/runner/work/rustgit/rustgit/src/analyze/analyzer.rs`.

## Phase 7 — Cache
- Commit-keyed deterministic analyze cache (`sha256(repo_url + branch + resolved_commit + analyze_version)`) in `/home/runner/work/rustgit/rustgit/src/analyze/cache.rs`.

## Phase 8 — Provider decoupling
- Analyze response is provider-independent (`execution.provider = local`) while provider scoring/blueprint artifacts are generated asynchronously (`src/analyze/blueprint_builder.rs`, `src/analyze/analyzer.rs`).

## Phase 9 — Deterministic runtime registry
- Runtime registry for framework/lockfile mapping in `/home/runner/work/rustgit/rustgit/src/analyze/registry.rs`.

## Phase 10 — Analyze response contract
- Response shape assembled in `/home/runner/work/rustgit/rustgit/src/analyze/analyzer.rs`.

## Test traceability
- Route + cache + manifest verification in `/home/runner/work/rustgit/rustgit/src/bin/server.rs` test: `analyze_route_generates_manifest_and_uses_cache`.
