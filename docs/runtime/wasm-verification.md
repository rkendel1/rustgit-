# WASM Verification (Code-Grounded)

## Current WASM modules
`WasmRuntimeCompiler::compile` builds component modules, including:
- Base: `filesystem.wasm`, `network.wasm`, `process.wasm` (`src/lib.rs:16320-16348`)
- Language-specific: `nodejs.wasm` / `python.wasm` / `rust.wasm` (`src/lib.rs:16350-16377`)
- Package-manager module: `{package_manager}.wasm` (`src/lib.rs:16381-16389`)
- Framework module: `{framework}.wasm` (`src/lib.rs:16390-16398`)
- WASI module: `wasi.wasm` when `requires_wasm` (`src/lib.rs:16400-16408`)

## Invocation path
- Router maps node mode to `ExecutionTarget::Wasm(...)` based on `WasmCompatibility` (`src/lib.rs:3243-3267`).
- `WasmExecutionProvider::start` loads compiled wasm artifacts and instantiates runtime (`src/lib.rs:17235-17261`).
- Core execution API: `WasmRuntimeEngine::execute_module` (`src/lib.rs:2126-2166`).

## Execution limits
- Runtime spec carries memory/cpu/syscall limits (`src/lib.rs:1980-1983`).
- Full vs partial wasm limits defined in router runtime spec generation (`src/lib.rs:3271-3303`).
- Sandbox enforces memory/time/filesystem scope (`src/lib.rs:17809-17815`).

## Supported frameworks
- Analyze capability metadata lists WASM support for `react`, `vite`, `svelte`, `static`, `node` (`src/analyze/blueprint_builder.rs:77-86`).
- Execution runtime uses `requires_wasm` + compatibility to choose wasm path (`src/lib.rs:16243`, `3243-3267`).

## Current production usage evidence in code
- WASM provider is registered in default runtime providers (`src/lib.rs:12311-12313`).
- Runtime capability endpoint reports provider health/enabled statuses including WASM metadata (`src/bin/server.rs:401-405`).

## Fallback behavior
- `ExecutionMode::Wasm` falls back to Native when compatibility is `NotSupported` (`src/lib.rs:3254-3259`).
- `ExecutionMode::Hybrid` falls back to Native unless compatibility is `Full` (`src/lib.rs:3260-3267`).
