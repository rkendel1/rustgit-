CREATE TABLE IF NOT EXISTS repository_identities (
    id TEXT PRIMARY KEY REFERENCES repositories(repo_id) ON DELETE CASCADE,
    github_owner TEXT NOT NULL,
    github_repo TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    repository_fingerprint TEXT NOT NULL,
    health_score DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (health_score >= 0 AND health_score <= 100),
    execution_score DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (execution_score >= 0 AND execution_score <= 100),
    healing_score DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (healing_score >= 0 AND healing_score <= 100),
    verification_state TEXT NOT NULL CHECK (verification_state IN ('unverified', 'verified')),
    badge_state TEXT NOT NULL CHECK (badge_state IN ('untested', 'runnable', 'verified', 'healed', 'production_ready')),
    current_workspace_id TEXT,
    latest_execution_id TEXT REFERENCES executions(execution_id) ON DELETE SET NULL,
    latest_successful_execution_id TEXT REFERENCES executions(execution_id) ON DELETE SET NULL,
    CHECK (last_seen_at >= first_seen_at)
);

CREATE TABLE IF NOT EXISTS repair_plans (
    plan_id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    execution_id TEXT NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    failure_class TEXT NOT NULL,
    plan_payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS repair_outcomes (
    outcome_id TEXT PRIMARY KEY,
    plan_id TEXT NOT NULL REFERENCES repair_plans(plan_id) ON DELETE CASCADE,
    success BOOLEAN NOT NULL,
    validation_log TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS repair_artifacts (
    artifact_id TEXT PRIMARY KEY,
    outcome_id TEXT NOT NULL REFERENCES repair_outcomes(outcome_id) ON DELETE CASCADE,
    artifact_type TEXT NOT NULL,
    artifact_url TEXT,
    artifact_payload JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
