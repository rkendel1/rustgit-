CREATE INDEX IF NOT EXISTS idx_commits_repository_id ON commits(repository_id);
CREATE INDEX IF NOT EXISTS idx_commits_author_date ON commits(author_date DESC);
CREATE INDEX IF NOT EXISTS idx_fingerprints_repository_id ON fingerprints(repository_id);
CREATE INDEX IF NOT EXISTS idx_fingerprints_commit_hash ON fingerprints(commit_hash);
CREATE INDEX IF NOT EXISTS idx_services_fingerprint_id ON services(fingerprint_id);
CREATE INDEX IF NOT EXISTS idx_topologies_fingerprint_id ON topologies(fingerprint_id);
CREATE INDEX IF NOT EXISTS idx_executions_repository_id ON executions(repository_id);
CREATE INDEX IF NOT EXISTS idx_executions_commit_hash ON executions(commit_hash);
CREATE INDEX IF NOT EXISTS idx_executions_started_at ON executions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_execution_events_execution_id ON execution_events(execution_id);
CREATE INDEX IF NOT EXISTS idx_execution_events_created_at ON execution_events(created_at);
CREATE INDEX IF NOT EXISTS idx_warm_pool_usage_execution_id ON warm_pool_usage(execution_id);
CREATE INDEX IF NOT EXISTS idx_warm_pool_usage_image_id ON warm_pool_usage(image_id);
CREATE INDEX IF NOT EXISTS idx_healing_attempts_repository_id ON healing_attempts(repository_id);
CREATE INDEX IF NOT EXISTS idx_healing_attempts_execution_id ON healing_attempts(execution_id);
CREATE INDEX IF NOT EXISTS idx_healing_attempts_created_at ON healing_attempts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_url_allocations_execution_id ON url_allocations(execution_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_repository_id ON workspaces(repository_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_commit_hash ON workspaces(commit_hash);
CREATE INDEX IF NOT EXISTS idx_workspace_bindings_runtime_type ON workspace_runtime_bindings(runtime_type);
CREATE INDEX IF NOT EXISTS idx_workspace_bindings_lease_expires_at ON workspace_runtime_bindings(lease_expires_at);
CREATE INDEX IF NOT EXISTS idx_journey_results_repo_id ON journey_results(repo_id);
CREATE INDEX IF NOT EXISTS idx_journey_results_journey_type ON journey_results(journey_type);
CREATE INDEX IF NOT EXISTS idx_commit_execution_results_commit_hash ON commit_execution_results(commit_hash);
CREATE INDEX IF NOT EXISTS idx_commit_execution_results_recorded_at ON commit_execution_results(recorded_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS uq_runtime_images_hash_runtime_framework
    ON runtime_images(image_hash, runtime, COALESCE(framework, ''));
CREATE UNIQUE INDEX IF NOT EXISTS uq_journey_results_repo_journey_success_time
    ON journey_results(repo_id, journey_type, success, time_to_url_ms);
