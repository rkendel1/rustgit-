# Docker Verification (Code-Grounded)

## Is Docker ever invoked?
**Yes, potentially.**

### Where
- Analyze manifest synthesizer can set command to `docker compose up` or `docker` (`src/analyze/manifest_builder.rs:117-121`, `305-312`).
- Runtime process launcher executes the resolved command programmatically via `Command::new(program)` (`src/lib.rs:13260-13279`).

### Under what conditions
- `docker compose up` when `docker-compose.yml`/`.yaml` exists (`src/analyze/manifest_builder.rs:305-307`).
- `docker` when `Dockerfile` exists (`src/analyze/manifest_builder.rs:308-310`).
- `.devcontainer/devcontainer.json` yields `devcontainer up --workspace-folder .` (`src/analyze/manifest_builder.rs:311-312`).

### What command
- `docker compose up`
- `docker`
- Also generated helper remediation suggestions include `docker run ...` strings (`src/lib.rs:18659-18662`).

### What runtime
- Analyze manifest marks preferred runtime as `docker` when docker/devcontainer signals are present (`src/analyze/manifest_builder.rs:104-113`).
- Execution tier model includes `LocalDocker` (`src/lib.rs:1747`, `3028-3029`).

### What fallback
- Analyze manifest fallback is package manager/runtime token (`src/analyze/manifest_builder.rs:113-116`).
- Analyze execution plan fallback is lower-ranked providers (`src/analyze/blueprint_builder.rs:205-210`).

## Places UI/docs claim Docker execution
- Badge/runtime preference variants include `docker` (`src/lib.rs:8320`, `9417`).
- Analyze cache test fixture uses fallback list containing `docker` (`src/bin/server.rs:1677`).
- Portal runtime selector includes a Docker option (`portal/src/app/page.tsx:1121`).

## Mismatch notes
- Concrete execution providers registered for runtime are `WasmExecutionProvider`, `LocalAgentProvider`, `NodeRuntimeProvider`, `RustRuntimeProvider`, `StaticRuntimeProvider` (`src/lib.rs:12311-12317`).
- There is no dedicated `DockerExecutionProvider` type in the registered provider list.
