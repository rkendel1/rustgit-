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

## README badge execution + healing loop

The API surface now includes a badge-driven execution seed flow:

- `POST /api/badges/generate` — portal badge generator for markdown, HTML, badge URL, and seed trigger snippets
- `GET /badge/{owner}/{repo}.svg` — dynamic runtime status badge (untested / runnable / verified / healed / production ready)
- `GET /badge/healed/{owner}/{repo}.svg` — healed badge variant
- `GET /seed/{owner}/{repo}` — badge click bootstrap into anonymous execution + analyze/plan/start pipeline
- `GET /api/repositories/{id}/intelligence` — repository intelligence panel data (execution score, runtime, launch/heal/adopt actions)

Example badge embed:

```html
<a href="https://trythissoftware.com/seed/{owner}/{repo}">
  <img src="https://cdn.trythissoftware.com/badge/{owner}/{repo}.svg" />
</a>
```

Badge screenshots:

![Execution status badge screenshot](https://cdn.trythissoftware.com/badge/vercel/next.js.svg)
![Healed badge screenshot](https://cdn.trythissoftware.com/badge/healed/vercel/next.js.svg)

This badge updates automatically based on repository execution health.

## Quick start

```bash
cargo test
cargo run --bin wasm-workspace-cli -- launch /absolute/path/to/repo
```

## Portal (Next.js)

The management portal now lives in `./portal` as a standalone Next.js app.

```bash
cd portal
npm install
npm run dev
```

### Local platform development

Run the API and portal together in separate terminals:

```bash
# Terminal 1 (API)
cargo run --bin server

# Terminal 2 (Portal)
cd portal
npm install
npm run dev
```

By default, the portal uses `http://localhost:8080` for API requests in development.

Production/Fly deploy for the portal uses `portal/Dockerfile` through `deploy/fly/portal.fly.toml`.

Portal screenshot:

![TryThisSoftware portal](docs/screenshots/portal-home.png)

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
- `postgres.fly.toml` (`trythissoftware-db`) — self-managed Postgres, runs on Fly's private network

Required runtime environment variables:

- API: `DATABASE_URL`, `REDIS_URL` (optional), `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `JWT_SECRET`, `BASE_DOMAIN=trythissoftware.com`
- Portal: `NEXT_PUBLIC_API_URL=https://api.trythissoftware.com`, `NEXT_PUBLIC_BASE_DOMAIN=trythissoftware.com`

OAuth callback endpoints (API):

- `GET https://api.trythissoftware.com/auth/github/callback`
- `GET https://api.trythissoftware.com/auth/google/callback`

Store API credentials/secrets as Fly secrets instead of committing them to config files:

```bash
# Deploy Postgres first
fly deploy --config deploy/fly/postgres.fly.toml

# Set the DATABASE_URL pointing to Fly's private network (.flycast)
fly secrets set --app trythissoftware-api \
  DATABASE_URL="postgresql://postgres:<password>@trythissoftware-db.flycast:5432/rustgit" \
  GITHUB_CLIENT_ID=<your-github-client-id> \
  GITHUB_CLIENT_SECRET=<your-github-client-secret> \
  JWT_SECRET=<your-jwt-secret>

# Deploy the API
fly deploy --config deploy/fly/api.fly.toml
```

`POSTGRES_PASSWORD` must be set as a secret on the Postgres app before first deploy:

```bash
fly secrets set --app trythissoftware-db POSTGRES_PASSWORD=<password>
```

### Local initialization example

```bash
docker run --name rustgit-postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=rustgit -p 5432:5432 -d postgres:17
export DATABASE_URL=postgresql://<username>:<password>@localhost:5432/rustgit
cargo test
```
