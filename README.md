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

## Dual Surface Experience (DSE)

TryThisSoftware is modeled as one product with two entry surfaces:

- **GitHub Overlay Extension** (activation surface): discover repositories on GitHub and launch runs quickly.
- **TryThisSoftware Portal** (management surface): monitor workspaces, executions, logs, URLs, and agents.

Both surfaces route through the same backend primitives:

- Shared **Execution API** (`/api/v1/executions`)
- Shared **Control Plane**
- Shared execution IDs, URLs, and runtime state

Surface UI contracts are rendered through a shared Surface Rendering System (SRS):

- Shared TryThisSoftware design system component model
- Shared component registry for contract-to-component mapping
- Unified renderer output for Portal shell and GitHub overlay shell

## Quick start

```bash
cargo test
cargo run --bin wasm-workspace-cli -- launch /absolute/path/to/repo
```

## PostgreSQL persistence

This repository now includes SQL migrations and a production-style PostgreSQL persistence layer for Execution Intelligence history.

### Migrations

Migrations are stored in `./migrations`:

- `0001_baseline_schema.sql` — core tables, PK/FK constraints, nullable rules, and check constraints
- `0002_indexes_and_constraints.sql` — performance indexes and uniqueness constraints
- `0003_seed_bootstrap.sql` — bootstrap seed rows

`ExecutionIntelligencePostgresStore::initialize()` runs migrations on startup and records applied versions in `schema_migrations`.

### Environment variables

- `DATABASE_URL` (required for runtime Postgres initialization)
- `RUSTGIT_EIDB_TEST_DATABASE_URL` (optional, used by integration tests in `tests/postgres_persistence.rs`)

## Production deployment and domain mapping

The production domain hierarchy is now unified under `trythissoftware.com`:

- Portal: `https://trythissoftware.com`
- API / extension backend: `https://api.trythissoftware.com`
- Workspace runtime: `https://workspace-{id}.trythissoftware.com`

Fly.io app configs are checked in under `deploy/fly/`:

- `api.fly.toml` (`trythissoftware-api`)
- `portal.fly.toml` (`trythissoftware-portal`)
- `workspaces.fly.toml` (`trythissoftware-workspaces`)

Required runtime environment variables:

- API: `DATABASE_URL`, `REDIS_URL` (optional), `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `JWT_SECRET`, `BASE_DOMAIN=trythissoftware.com`
- Portal: `NEXT_PUBLIC_API_URL=https://api.trythissoftware.com`, `NEXT_PUBLIC_BASE_DOMAIN=trythissoftware.com`

Store API credentials/secrets (`DATABASE_URL`, `REDIS_URL`, GitHub OAuth secrets, `JWT_SECRET`) as Fly secrets (`fly secrets set ...`) instead of committing them to Fly config files.

### Local initialization example

```bash
docker run --name rustgit-postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=rustgit -p 5432:5432 -d postgres:17
export DATABASE_URL=postgresql://<username>:<password>@localhost:5432/rustgit
cargo test
```
