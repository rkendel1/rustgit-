CREATE TABLE IF NOT EXISTS billing_events (
    event_id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    execution_id TEXT NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    runtime_type TEXT NOT NULL,
    resource_usage JSONB NOT NULL,
    cost_units DOUBLE PRECISION NOT NULL CHECK (cost_units >= 0),
    timestamp TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_billing_events_org_id ON billing_events(org_id);
CREATE INDEX IF NOT EXISTS idx_billing_events_execution_id ON billing_events(execution_id);
CREATE INDEX IF NOT EXISTS idx_billing_events_runtime_type ON billing_events(runtime_type);
CREATE INDEX IF NOT EXISTS idx_billing_events_timestamp ON billing_events(timestamp DESC);
