CREATE TABLE IF NOT EXISTS users (
    user_id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    auth_provider TEXT NOT NULL CHECK (auth_provider IN ('github', 'google', 'microsoft')),
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS organizations (
    org_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    plan TEXT NOT NULL CHECK (plan IN ('free', 'pro', 'enterprise')),
    max_workspaces INTEGER NOT NULL DEFAULT 3 CHECK (max_workspaces >= 1),
    max_concurrent_executions INTEGER NOT NULL DEFAULT 5 CHECK (max_concurrent_executions >= 1),
    max_runtime_minutes INTEGER NOT NULL DEFAULT 1000 CHECK (max_runtime_minutes >= 1),
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS memberships (
    user_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    org_id TEXT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'developer', 'viewer')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, org_id)
);

CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    repo_url TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    first_seen TIMESTAMPTZ NOT NULL,
    last_seen TIMESTAMPTZ NOT NULL,
    CHECK (last_seen >= first_seen)
);

CREATE TABLE IF NOT EXISTS commits (
    commit_hash TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    author_date TIMESTAMPTZ NOT NULL,
    message TEXT NOT NULL,
    parent_commit TEXT
);

CREATE TABLE IF NOT EXISTS fingerprints (
    fingerprint_id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    commit_hash TEXT NOT NULL REFERENCES commits(commit_hash) ON DELETE CASCADE,
    frameworks JSONB NOT NULL,
    languages JSONB NOT NULL,
    services JSONB NOT NULL,
    confidence DOUBLE PRECISION NOT NULL,
    CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE TABLE IF NOT EXISTS services (
    service_id TEXT PRIMARY KEY,
    fingerprint_id TEXT NOT NULL REFERENCES fingerprints(fingerprint_id) ON DELETE CASCADE,
    service_type TEXT NOT NULL,
    framework TEXT,
    runtime TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS topologies (
    topology_id TEXT PRIMARY KEY,
    fingerprint_id TEXT NOT NULL REFERENCES fingerprints(fingerprint_id) ON DELETE CASCADE,
    service_count INTEGER NOT NULL,
    edge_count INTEGER NOT NULL,
    CHECK (service_count >= 0),
    CHECK (edge_count >= 0)
);

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    repository_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    commit_hash TEXT NOT NULL REFERENCES commits(commit_hash) ON DELETE CASCADE,
    created_by TEXT NOT NULL REFERENCES users(user_id) ON DELETE RESTRICT,
    visibility TEXT NOT NULL CHECK (visibility IN ('private', 'org', 'public')),
    current_runtime TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    last_healthy_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS executions (
    execution_id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    repository_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    commit_hash TEXT NOT NULL REFERENCES commits(commit_hash) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL,
    execution_tier TEXT NOT NULL,
    CHECK (completed_at IS NULL OR completed_at >= started_at)
);

CREATE TABLE IF NOT EXISTS execution_events (
    id BIGSERIAL PRIMARY KEY,
    execution_id TEXT NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_images (
    image_id TEXT PRIMARY KEY,
    image_hash TEXT NOT NULL,
    runtime TEXT NOT NULL,
    framework TEXT
);

CREATE TABLE IF NOT EXISTS warm_pool_usage (
    id BIGSERIAL PRIMARY KEY,
    execution_id TEXT NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    image_id TEXT NOT NULL REFERENCES runtime_images(image_id) ON DELETE CASCADE,
    cache_hit BOOLEAN NOT NULL,
    cold_start BOOLEAN NOT NULL,
    startup_time_ms DOUBLE PRECISION NOT NULL,
    CHECK (startup_time_ms >= 0)
);

CREATE TABLE IF NOT EXISTS healing_attempts (
    id BIGSERIAL PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    execution_id TEXT NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    failure_class TEXT NOT NULL,
    repair_strategy TEXT NOT NULL,
    success BOOLEAN NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS url_allocations (
    workspace_url TEXT PRIMARY KEY,
    execution_id TEXT NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
    released_at TIMESTAMPTZ,
    CHECK (released_at IS NULL OR released_at >= created_at)
);

CREATE TABLE IF NOT EXISTS workspace_runtime_bindings (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE CASCADE,
    runtime_type TEXT NOT NULL CHECK (runtime_type IN ('DEA', 'CLOUD', 'DOCKER', 'EXTERNAL')),
    runtime_instance_id TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    runtime_heartbeat TIMESTAMPTZ,
    last_request_time TIMESTAMPTZ,
    execution_health BOOLEAN
);

CREATE TABLE IF NOT EXISTS agents (
    agent_id TEXT PRIMARY KEY,
    capabilities JSONB NOT NULL,
    last_seen TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS journey_results (
    id BIGSERIAL PRIMARY KEY,
    journey_type TEXT NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    success BOOLEAN NOT NULL,
    time_to_url_ms BIGINT NOT NULL,
    CHECK (time_to_url_ms >= 0)
);

CREATE TABLE IF NOT EXISTS commit_execution_results (
    id BIGSERIAL PRIMARY KEY,
    commit_hash TEXT NOT NULL REFERENCES commits(commit_hash) ON DELETE CASCADE,
    success BOOLEAN NOT NULL,
    startup_time_ms DOUBLE PRECISION NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL,
    CHECK (startup_time_ms >= 0)
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id BIGSERIAL PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE RESTRICT,
    org_id TEXT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    resource TEXT NOT NULL,
    ip_address TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
