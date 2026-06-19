# rustgit-

A Rust foundation for a Gitpod-compatible WebAssembly workspace runtime.

## What is included

- Repository lifecycle primitives (clone/materialize, analyze, execution planning, caching)
- Workspace runtime API (`WasmWorkspace`) with launch/stop/restart/logs/filesystem/ports
- Execution router + provider model (`ExecutionRouter`, `ExecutionProvider`) for WASM/native/static substrates
- Execution substrate foundation (`WasmRuntimeEngine`, `NativeRuntimeEngine`, `HybridExecutionBridge`) for concrete runtime dispatch
- Virtual filesystem with snapshot + restore
- Network policy and resource quota structures for sandbox controls
- REST API route surface definition (`RestApiSpec`)
- Example CLI (`wasm-workspace-cli`)

## Quick start

```bash
cargo test
cargo run --bin wasm-workspace-cli -- launch /absolute/path/to/repo
```
