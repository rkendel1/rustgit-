use rustgit_wasm_runtime::{
    execution_history_endpoint_with_store, repository_healing_history_endpoint_with_store,
    repository_history_endpoint_with_store, repository_last_good_commit_endpoint_with_store,
    EidbBillingEventRecord,
    EidbCommitExecutionResultRecord, EidbCommitRecord, EidbExecutionEventRecord, EidbExecutionRecord,
    EidbHealingAttemptRecord, EidbJourneyResultRecord, EidbRepositoryRecord, EidbUrlAllocationRecord,
    ExecutionIntelligencePostgresStore,
};
use serde_json::json;

fn test_database_url() -> Option<String> {
    std::env::var("RUSTGIT_EIDB_TEST_DATABASE_URL").ok()
}

#[test]
fn postgres_migrations_apply_and_enforce_foreign_keys() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = test_database_url() else {
        return Ok(());
    };

    let store = ExecutionIntelligencePostgresStore::connect(&database_url)?;
    store.reset_for_development()?;

    assert_eq!(store.migration_count()?, 4);
    assert!(store.table_exists("users")?);
    assert!(store.table_exists("organizations")?);
    assert!(store.table_exists("memberships")?);
    assert!(store.table_exists("repositories")?);
    assert!(store.table_exists("executions")?);
    assert!(store.table_exists("workspaces")?);
    assert!(store.table_exists("audit_logs")?);
    assert!(store.table_exists("workspace_runtime_bindings")?);
    assert!(store.table_exists("billing_events")?);
    assert!(store.table_exists("schema_migrations")?);

    let missing_repo_commit = store.insert_commit(&EidbCommitRecord {
        commit_hash: "fk-commit".to_string(),
        repository_id: "missing-repo".to_string(),
        author_date: 1,
        message: "missing repo".to_string(),
        parent_commit: None,
    });
    assert!(missing_repo_commit.is_err());

    Ok(())
}

#[test]
fn postgres_store_round_trips_history_endpoints() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = test_database_url() else {
        return Ok(());
    };

    let store = ExecutionIntelligencePostgresStore::connect(&database_url)?;
    store.reset_for_development()?;

    store.upsert_repository(&EidbRepositoryRecord {
        repo_id: "repo-eidb".to_string(),
        repo_url: "https://github.com/rkendel1/rustgit-example".to_string(),
        default_branch: "main".to_string(),
        first_seen: 1,
        last_seen: 2,
    })?;

    store.insert_commit(&EidbCommitRecord {
        commit_hash: "aaaaaaa".to_string(),
        repository_id: "repo-eidb".to_string(),
        author_date: 10,
        message: "initial".to_string(),
        parent_commit: None,
    })?;

    store.upsert_workspace(
        "ws-1",
        "org-bootstrap",
        "repo-eidb",
        "aaaaaaa",
        "user-bootstrap",
        "private",
        "DEA",
        "running",
        11,
        Some(11),
    )?;

    store.insert_execution(&EidbExecutionRecord {
        execution_id: "exec-1".to_string(),
        org_id: "org-bootstrap".to_string(),
        user_id: "user-bootstrap".to_string(),
        workspace_id: "ws-1".to_string(),
        repository_id: "repo-eidb".to_string(),
        commit_hash: "aaaaaaa".to_string(),
        started_at: 11,
        completed_at: Some(12),
        status: "success".to_string(),
        execution_tier: "CLOUD".to_string(),
    })?;

    store.insert_execution_event(&EidbExecutionEventRecord {
        execution_id: "exec-1".to_string(),
        event_type: "STARTED".to_string(),
        created_at: 11,
    })?;

    store.insert_billing_event(&EidbBillingEventRecord {
        event_id: "bill-postgres-1".to_string(),
        org_id: "org-bootstrap".to_string(),
        user_id: "user-bootstrap".to_string(),
        workspace_id: "ws-1".to_string(),
        execution_id: "exec-1".to_string(),
        event_type: "EXECUTION_COMPLETED".to_string(),
        runtime_type: "DEA_LOCAL".to_string(),
        resource_usage: json!({
            "duration_seconds": 60.0,
            "healing_cycles": 0,
            "warm_pool_hits": 1,
        }),
        cost_units: 1.5,
        timestamp: 12,
    })?;

    store.insert_healing_attempt(&EidbHealingAttemptRecord {
        repository_id: "repo-eidb".to_string(),
        execution_id: "exec-1".to_string(),
        failure_class: "WrongPackageManager".to_string(),
        repair_strategy: "switch-pnpm".to_string(),
        success: true,
        created_at: 12,
    })?;

    store.insert_url_allocation(&EidbUrlAllocationRecord {
        workspace_url: "https://workspace-1.trythissoftware.com".to_string(),
        execution_id: "exec-1".to_string(),
        created_at: 11,
        released_at: None,
    })?;

    store.insert_journey_result(&EidbJourneyResultRecord {
        journey_type: "journey-10-runtime-migration".to_string(),
        repo_id: "repo-eidb".to_string(),
        success: true,
        time_to_url_ms: 1200,
    })?;

    store.insert_commit_execution_result(&EidbCommitExecutionResultRecord {
        commit_hash: "aaaaaaa".to_string(),
        success: true,
        startup_time_ms: 4200.0,
        recorded_at: 12,
    })?;

    let (repo_history_path, repo_history_body) =
        repository_history_endpoint_with_store("repo-eidb", &store)?;
    assert_eq!(repo_history_path, "/repositories/repo-eidb/history");
    assert!(repo_history_body.contains("\"commit_hash\":\"aaaaaaa\""));

    let (execution_history_path, execution_history_body) =
        execution_history_endpoint_with_store("exec-1", &store)?;
    assert_eq!(execution_history_path, "/executions/exec-1/history");
    assert!(execution_history_body.contains("\"event_type\":\"STARTED\""));
    assert!(execution_history_body.contains("\"event_id\":\"bill-postgres-1\""));
    assert!(execution_history_body.contains("workspace-1.trythissoftware.com"));

    let (healing_path, healing_body) = repository_healing_history_endpoint_with_store("repo-eidb", &store)?;
    assert_eq!(healing_path, "/repositories/repo-eidb/healing");
    assert!(healing_body.contains("\"failure_class\":\"WrongPackageManager\""));

    let (last_good_path, last_good_body) =
        repository_last_good_commit_endpoint_with_store("repo-eidb", &store)?;
    assert_eq!(last_good_path, "/repositories/repo-eidb/last-good");
    assert!(last_good_body.contains("\"commit_hash\":\"aaaaaaa\""));

    Ok(())
}
