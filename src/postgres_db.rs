use crate::{
    EidbCommitExecutionResultRecord, EidbCommitRecord, EidbExecutionEventRecord, EidbExecutionRecord,
    EidbHealingAttemptRecord, EidbJourneyResultRecord, EidbRepositoryRecord, EidbUrlAllocationRecord,
    EidbWarmPoolUsageRecord, EidbBillingEventRecord, ExecutionIntelligenceDatabase,
};
use postgres::NoTls;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum ExecutionIntelligencePersistenceError {
    MissingDatabaseUrl,
    InvalidDatabaseUrl(String),
    Pool(r2d2::Error),
    Postgres(postgres::Error),
    Io(std::io::Error),
    TimestampOutOfRange(i64),
    Serialization(String),
}

impl Display for ExecutionIntelligencePersistenceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDatabaseUrl => write!(f, "DATABASE_URL is required for PostgreSQL persistence"),
            Self::InvalidDatabaseUrl(err) => write!(f, "invalid DATABASE_URL: {err}"),
            Self::Pool(err) => write!(f, "postgres pool error: {err}"),
            Self::Postgres(err) => write!(f, "postgres error: {err}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::TimestampOutOfRange(value) => write!(f, "timestamp out of range for u64: {value}"),
            Self::Serialization(err) => write!(f, "serialization error: {err}"),
        }
    }
}

impl std::error::Error for ExecutionIntelligencePersistenceError {}

impl From<r2d2::Error> for ExecutionIntelligencePersistenceError {
    fn from(value: r2d2::Error) -> Self {
        Self::Pool(value)
    }
}

impl From<postgres::Error> for ExecutionIntelligencePersistenceError {
    fn from(value: postgres::Error) -> Self {
        Self::Postgres(value)
    }
}

impl From<std::io::Error> for ExecutionIntelligencePersistenceError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub type PersistenceResult<T> = std::result::Result<T, ExecutionIntelligencePersistenceError>;

struct Migration {
    version: &'static str,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: "0001",
        name: "baseline_schema",
        sql: include_str!("../migrations/0001_baseline_schema.sql"),
    },
    Migration {
        version: "0002",
        name: "indexes_and_constraints",
        sql: include_str!("../migrations/0002_indexes_and_constraints.sql"),
    },
    Migration {
        version: "0003",
        name: "seed_bootstrap",
        sql: include_str!("../migrations/0003_seed_bootstrap.sql"),
    },
    Migration {
        version: "0004",
        name: "billing_metering",
        sql: include_str!("../migrations/0004_billing_metering.sql"),
    },
    Migration {
        version: "0005",
        name: "anonymous_execution_identity",
        sql: include_str!("../migrations/0005_anonymous_execution_identity.sql"),
    },
    Migration {
        version: "0006",
        name: "repository_identity_and_healing_repairs",
        sql: include_str!("../migrations/0006_repository_identity_and_healing_repairs.sql"),
    },
];

pub trait ExecutionIntelligenceReadStore {
    fn repository(&self, repository_id: &str) -> PersistenceResult<Option<EidbRepositoryRecord>>;
    fn commits_for_repository(&self, repository_id: &str) -> PersistenceResult<Vec<EidbCommitRecord>>;
    fn executions_for_repository(&self, repository_id: &str)
        -> PersistenceResult<Vec<EidbExecutionRecord>>;
    fn journey_results_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbJourneyResultRecord>>;
    fn execution(&self, execution_id: &str) -> PersistenceResult<Option<EidbExecutionRecord>>;
    fn events_for_execution(&self, execution_id: &str)
        -> PersistenceResult<Vec<EidbExecutionEventRecord>>;
    fn url_allocations_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbUrlAllocationRecord>>;
    fn healing_attempts_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbHealingAttemptRecord>>;
    fn warm_pool_usage_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbWarmPoolUsageRecord>>;
    fn healing_attempts_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbHealingAttemptRecord>>;
    fn billing_events_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbBillingEventRecord>>;
    fn billing_events_for_org(&self, org_id: &str) -> PersistenceResult<Vec<EidbBillingEventRecord>>;
    fn billing_events(&self) -> PersistenceResult<Vec<EidbBillingEventRecord>>;
    fn last_good_commit_for_repository(&self, repository_id: &str) -> PersistenceResult<Option<String>>;
}

impl ExecutionIntelligenceReadStore for ExecutionIntelligenceDatabase {
    fn repository(&self, repository_id: &str) -> PersistenceResult<Option<EidbRepositoryRecord>> {
        Ok(self.repositories.get(repository_id).cloned())
    }

    fn commits_for_repository(&self, repository_id: &str) -> PersistenceResult<Vec<EidbCommitRecord>> {
        Ok(self
            .commits
            .iter()
            .filter(|commit| commit.repository_id == repository_id)
            .cloned()
            .collect())
    }

    fn executions_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbExecutionRecord>> {
        Ok(self
            .executions
            .iter()
            .filter(|execution| execution.repository_id == repository_id)
            .cloned()
            .collect())
    }

    fn journey_results_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbJourneyResultRecord>> {
        Ok(self
            .journey_results
            .iter()
            .filter(|journey| journey.repo_id == repository_id)
            .cloned()
            .collect())
    }

    fn execution(&self, execution_id: &str) -> PersistenceResult<Option<EidbExecutionRecord>> {
        Ok(self
            .executions
            .iter()
            .find(|execution| execution.execution_id == execution_id)
            .cloned())
    }

    fn events_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbExecutionEventRecord>> {
        Ok(self
            .execution_events
            .iter()
            .filter(|event| event.execution_id == execution_id)
            .cloned()
            .collect())
    }

    fn url_allocations_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbUrlAllocationRecord>> {
        Ok(self
            .url_allocations
            .iter()
            .filter(|allocation| allocation.execution_id == execution_id)
            .cloned()
            .collect())
    }

    fn healing_attempts_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbHealingAttemptRecord>> {
        Ok(self
            .healing_attempts
            .iter()
            .filter(|attempt| attempt.execution_id == execution_id)
            .cloned()
            .collect())
    }

    fn warm_pool_usage_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbWarmPoolUsageRecord>> {
        Ok(self
            .warm_pool_usage
            .iter()
            .filter(|usage| usage.execution_id == execution_id)
            .cloned()
            .collect())
    }

    fn healing_attempts_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbHealingAttemptRecord>> {
        Ok(self
            .healing_attempts
            .iter()
            .filter(|attempt| attempt.repository_id == repository_id)
            .cloned()
            .collect())
    }

    fn billing_events_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbBillingEventRecord>> {
        Ok(self
            .billing_events
            .iter()
            .filter(|event| event.execution_id == execution_id)
            .cloned()
            .collect())
    }

    fn billing_events_for_org(&self, org_id: &str) -> PersistenceResult<Vec<EidbBillingEventRecord>> {
        Ok(self
            .billing_events
            .iter()
            .filter(|event| event.org_id == org_id)
            .cloned()
            .collect())
    }

    fn billing_events(&self) -> PersistenceResult<Vec<EidbBillingEventRecord>> {
        Ok(self.billing_events.clone())
    }

    fn last_good_commit_for_repository(&self, repository_id: &str) -> PersistenceResult<Option<String>> {
        Ok(self
            .last_good_commit_for_repository(repository_id)
            .map(ToString::to_string))
    }
}

#[derive(Clone)]
pub struct ExecutionIntelligencePostgresStore {
    pool: Pool<PostgresConnectionManager<NoTls>>,
}

impl ExecutionIntelligencePostgresStore {
    pub fn from_env() -> PersistenceResult<Self> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| ExecutionIntelligencePersistenceError::MissingDatabaseUrl)?;
        Self::connect(&database_url)
    }

    pub fn connect(database_url: &str) -> PersistenceResult<Self> {
        let config = database_url
            .parse()
            .map_err(|err: postgres::Error| {
                ExecutionIntelligencePersistenceError::InvalidDatabaseUrl(err.to_string())
            })?;
        let manager = PostgresConnectionManager::new(config, NoTls);
        let pool = Pool::builder().max_size(16).build(manager)?;
        let store = Self { pool };
        store.initialize()?;
        Ok(store)
    }

    pub fn initialize(&self) -> PersistenceResult<()> {
        self.with_client(Self::run_migrations)
    }

    pub fn reset_for_development(&self) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.batch_execute(
                "
                DROP TABLE IF EXISTS commit_execution_results CASCADE;
                DROP TABLE IF EXISTS audit_logs CASCADE;
                DROP TABLE IF EXISTS journey_results CASCADE;
                DROP TABLE IF EXISTS agents CASCADE;
                DROP TABLE IF EXISTS workspace_runtime_bindings CASCADE;
                DROP TABLE IF EXISTS workspaces CASCADE;
                DROP TABLE IF EXISTS billing_events CASCADE;
                DROP TABLE IF EXISTS repair_artifacts CASCADE;
                DROP TABLE IF EXISTS repair_outcomes CASCADE;
                DROP TABLE IF EXISTS repair_plans CASCADE;
                DROP TABLE IF EXISTS repository_identities CASCADE;
                DROP TABLE IF EXISTS url_allocations CASCADE;
                DROP TABLE IF EXISTS healing_attempts CASCADE;
                DROP TABLE IF EXISTS warm_pool_usage CASCADE;
                DROP TABLE IF EXISTS runtime_images CASCADE;
                DROP TABLE IF EXISTS execution_events CASCADE;
                DROP TABLE IF EXISTS executions CASCADE;
                DROP TABLE IF EXISTS topologies CASCADE;
                DROP TABLE IF EXISTS services CASCADE;
                DROP TABLE IF EXISTS fingerprints CASCADE;
                DROP TABLE IF EXISTS commits CASCADE;
                DROP TABLE IF EXISTS repositories CASCADE;
                DROP TABLE IF EXISTS memberships CASCADE;
                DROP TABLE IF EXISTS organizations CASCADE;
                DROP TABLE IF EXISTS users CASCADE;
                DROP TABLE IF EXISTS schema_migrations CASCADE;
                ",
            )?;
            Ok(())
        })?;
        self.initialize()
    }

    pub fn migration_count(&self) -> PersistenceResult<i64> {
        self.with_client(|client| {
            let row = client.query_one("SELECT COUNT(*) FROM schema_migrations", &[])?;
            Ok(row.get(0))
        })
    }

    pub fn table_exists(&self, table_name: &str) -> PersistenceResult<bool> {
        self.with_client(|client| {
            let row = client.query_one(
                "SELECT EXISTS(
                    SELECT 1
                    FROM information_schema.tables
                    WHERE table_schema = 'public'
                      AND table_name = $1
                )",
                &[&table_name],
            )?;
            Ok(row.get(0))
        })
    }

    pub fn upsert_repository(&self, record: &EidbRepositoryRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO repositories (repo_id, repo_url, default_branch, first_seen, last_seen)
                 VALUES ($1, $2, $3, to_timestamp($4::double precision), to_timestamp($5::double precision))
                 ON CONFLICT (repo_id)
                 DO UPDATE SET
                    repo_url = EXCLUDED.repo_url,
                    default_branch = EXCLUDED.default_branch,
                    first_seen = EXCLUDED.first_seen,
                    last_seen = EXCLUDED.last_seen",
                &[
                    &record.repo_id,
                    &record.repo_url,
                    &record.default_branch,
                    &(record.first_seen as f64),
                    &(record.last_seen as f64),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_commit(&self, record: &EidbCommitRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO commits (commit_hash, repository_id, author_date, message, parent_commit)
                 VALUES ($1, $2, to_timestamp($3::double precision), $4, $5)
                 ON CONFLICT (commit_hash)
                 DO UPDATE SET
                    repository_id = EXCLUDED.repository_id,
                    author_date = EXCLUDED.author_date,
                    message = EXCLUDED.message,
                    parent_commit = EXCLUDED.parent_commit",
                &[
                    &record.commit_hash,
                    &record.repository_id,
                    &(record.author_date as f64),
                    &record.message,
                    &record.parent_commit,
                ],
            )?;
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_workspace(
        &self,
        workspace_id: &str,
        org_id: &str,
        repository_id: &str,
        commit_hash: &str,
        created_by: &str,
        visibility: &str,
        current_runtime: &str,
        status: &str,
        created_at: u64,
        last_healthy_at: Option<u64>,
    ) -> PersistenceResult<()> {
        self.with_client(|client| {
            let last_healthy_at = Self::optional_epoch_to_pg(last_healthy_at);
            client.execute(
                "INSERT INTO workspaces (
                    workspace_id, org_id, repository_id, commit_hash, created_by, visibility,
                    current_runtime, status, created_at, last_healthy_at
                 )
                 VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8,
                    to_timestamp($9::double precision),
                    CASE WHEN $10 IS NULL THEN NULL ELSE to_timestamp($10::double precision) END
                 )
                 ON CONFLICT (workspace_id)
                 DO UPDATE SET
                    org_id = EXCLUDED.org_id,
                    repository_id = EXCLUDED.repository_id,
                    commit_hash = EXCLUDED.commit_hash,
                    created_by = EXCLUDED.created_by,
                    visibility = EXCLUDED.visibility,
                    current_runtime = EXCLUDED.current_runtime,
                    status = EXCLUDED.status,
                    created_at = EXCLUDED.created_at,
                    last_healthy_at = EXCLUDED.last_healthy_at",
                &[
                    &workspace_id,
                    &org_id,
                    &repository_id,
                    &commit_hash,
                    &created_by,
                    &visibility,
                    &current_runtime,
                    &status,
                    &(created_at as f64),
                    &last_healthy_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_execution(&self, record: &EidbExecutionRecord) -> PersistenceResult<()> {
        if !record.has_owner() {
            return Err(ExecutionIntelligencePersistenceError::Serialization(
                "execution owner must include either user_id or anon_user_id".to_string(),
            ));
        }
        self.with_client(|client| {
            let completed_at = Self::optional_epoch_to_pg(record.completed_at);
            client.execute(
                "INSERT INTO executions (execution_id, org_id, user_id, anon_user_id, workspace_id, repository_id, commit_hash, started_at, completed_at, status, execution_tier)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, to_timestamp($8::double precision), CASE WHEN $9 IS NULL THEN NULL ELSE to_timestamp($9::double precision) END, $10, $11)
                 ON CONFLICT (execution_id)
                 DO UPDATE SET
                    org_id = EXCLUDED.org_id,
                    user_id = EXCLUDED.user_id,
                    anon_user_id = EXCLUDED.anon_user_id,
                    workspace_id = EXCLUDED.workspace_id,
                    repository_id = EXCLUDED.repository_id,
                    commit_hash = EXCLUDED.commit_hash,
                    started_at = EXCLUDED.started_at,
                    completed_at = EXCLUDED.completed_at,
                    status = EXCLUDED.status,
                    execution_tier = EXCLUDED.execution_tier",
                &[
                    &record.execution_id,
                    &record.org_id,
                    &record.user_id,
                    &record.anon_user_id,
                    &record.workspace_id,
                    &record.repository_id,
                    &record.commit_hash,
                    &(record.started_at as f64),
                    &completed_at,
                    &record.status,
                    &record.execution_tier,
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_execution_event(&self, record: &EidbExecutionEventRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO execution_events (execution_id, event_type, created_at)
                 VALUES ($1, $2, to_timestamp($3::double precision))",
                &[&record.execution_id, &record.event_type, &(record.created_at as f64)],
            )?;
            Ok(())
        })
    }

    pub fn insert_warm_pool_usage(&self, record: &EidbWarmPoolUsageRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO warm_pool_usage (execution_id, image_id, cache_hit, cold_start, startup_time_ms)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &record.execution_id,
                    &record.image_id,
                    &record.cache_hit,
                    &record.cold_start,
                    &(record.startup_time_ms as f64),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_healing_attempt(&self, record: &EidbHealingAttemptRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO healing_attempts (repository_id, execution_id, failure_class, repair_strategy, success, created_at)
                 VALUES ($1, $2, $3, $4, $5, to_timestamp($6::double precision))",
                &[
                    &record.repository_id,
                    &record.execution_id,
                    &record.failure_class,
                    &record.repair_strategy,
                    &record.success,
                    &(record.created_at as f64),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_url_allocation(&self, record: &EidbUrlAllocationRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            let released_at = Self::optional_epoch_to_pg(record.released_at);
            client.execute(
                "INSERT INTO url_allocations (workspace_url, execution_id, created_at, released_at)
                 VALUES ($1, $2, to_timestamp($3::double precision), CASE WHEN $4 IS NULL THEN NULL ELSE to_timestamp($4::double precision) END)
                 ON CONFLICT (workspace_url)
                 DO UPDATE SET
                    execution_id = EXCLUDED.execution_id,
                    created_at = EXCLUDED.created_at,
                    released_at = EXCLUDED.released_at",
                &[
                    &record.workspace_url,
                    &record.execution_id,
                    &(record.created_at as f64),
                    &released_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn upsert_agent(
        &self,
        agent_id: &str,
        capabilities: &[String],
        last_seen: u64,
        status: &str,
    ) -> PersistenceResult<()> {
        self.with_client(|client| {
            let capabilities = serde_json::to_value(capabilities)
                .map_err(|err| ExecutionIntelligencePersistenceError::Serialization(err.to_string()))?;
            client.execute(
                "INSERT INTO agents (agent_id, capabilities, last_seen, status)
                 VALUES ($1, $2, to_timestamp($3::double precision), $4)
                 ON CONFLICT (agent_id)
                 DO UPDATE SET
                    capabilities = EXCLUDED.capabilities,
                    last_seen = EXCLUDED.last_seen,
                    status = EXCLUDED.status",
                &[&agent_id, &capabilities, &(last_seen as f64), &status],
            )?;
            Ok(())
        })
    }

    pub fn insert_journey_result(&self, record: &EidbJourneyResultRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO journey_results (journey_type, repo_id, success, time_to_url_ms)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &record.journey_type,
                    &record.repo_id,
                    &record.success,
                    &(record.time_to_url_ms as i64),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_commit_execution_result(
        &self,
        record: &EidbCommitExecutionResultRecord,
    ) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO commit_execution_results (commit_hash, success, startup_time_ms, recorded_at)
                 VALUES ($1, $2, $3, to_timestamp($4::double precision))",
                &[
                    &record.commit_hash,
                    &record.success,
                    &(record.startup_time_ms as f64),
                    &(record.recorded_at as f64),
                ],
            )?;
            Ok(())
        })
    }

    pub fn insert_billing_event(&self, record: &EidbBillingEventRecord) -> PersistenceResult<()> {
        self.with_client(|client| {
            client.execute(
                "INSERT INTO billing_events (
                    event_id, org_id, user_id, workspace_id, execution_id,
                    event_type, runtime_type, resource_usage, cost_units, timestamp
                 )
                 VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, to_timestamp($10::double precision)
                 )
                 ON CONFLICT (event_id)
                 DO UPDATE SET
                    org_id = EXCLUDED.org_id,
                    user_id = EXCLUDED.user_id,
                    workspace_id = EXCLUDED.workspace_id,
                    execution_id = EXCLUDED.execution_id,
                    event_type = EXCLUDED.event_type,
                    runtime_type = EXCLUDED.runtime_type,
                    resource_usage = EXCLUDED.resource_usage,
                    cost_units = EXCLUDED.cost_units,
                    timestamp = EXCLUDED.timestamp",
                &[
                    &record.event_id,
                    &record.org_id,
                    &record.user_id,
                    &record.workspace_id,
                    &record.execution_id,
                    &record.event_type,
                    &record.runtime_type,
                    &record.resource_usage,
                    &(record.cost_units as f64),
                    &(record.timestamp as f64),
                ],
            )?;
            Ok(())
        })
    }

    fn with_client<T, F>(&self, mut f: F) -> PersistenceResult<T>
    where
        F: FnMut(&mut postgres::Client) -> PersistenceResult<T>,
    {
        let mut client = self.pool.get()?;
        f(&mut client)
    }

    fn run_migrations(client: &mut postgres::Client) -> PersistenceResult<()> {
        client.batch_execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )?;

        let rows = client.query("SELECT version FROM schema_migrations", &[])?;
        let applied: HashSet<String> = rows.into_iter().map(|row| row.get::<_, String>(0)).collect();

        for migration in MIGRATIONS {
            if applied.contains(migration.version) {
                continue;
            }

            let mut tx = client.transaction()?;
            tx.batch_execute(migration.sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name) VALUES ($1, $2)",
                &[&migration.version, &migration.name],
            )?;
            tx.commit()?;
        }

        Ok(())
    }

    fn to_u64(value: i64) -> PersistenceResult<u64> {
        u64::try_from(value).map_err(|_| ExecutionIntelligencePersistenceError::TimestampOutOfRange(value))
    }

    fn optional_epoch_to_pg(value: Option<u64>) -> Option<f64> {
        value.map(|epoch_seconds| epoch_seconds as f64)
    }
}

impl ExecutionIntelligenceReadStore for ExecutionIntelligencePostgresStore {
    fn repository(&self, repository_id: &str) -> PersistenceResult<Option<EidbRepositoryRecord>> {
        self.with_client(|client| {
            let row = client.query_opt(
                "SELECT repo_id, repo_url, default_branch,
                        EXTRACT(EPOCH FROM first_seen)::BIGINT,
                        EXTRACT(EPOCH FROM last_seen)::BIGINT
                 FROM repositories
                 WHERE repo_id = $1",
                &[&repository_id],
            )?;

            row.map(|row| {
                Ok(EidbRepositoryRecord {
                    repo_id: row.get(0),
                    repo_url: row.get(1),
                    default_branch: row.get(2),
                    first_seen: Self::to_u64(row.get::<_, i64>(3))?,
                    last_seen: Self::to_u64(row.get::<_, i64>(4))?,
                })
            })
            .transpose()
        })
    }

    fn commits_for_repository(&self, repository_id: &str) -> PersistenceResult<Vec<EidbCommitRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT commit_hash, repository_id,
                        EXTRACT(EPOCH FROM author_date)::BIGINT,
                        message, parent_commit
                 FROM commits
                 WHERE repository_id = $1
                 ORDER BY author_date",
                &[&repository_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbCommitRecord {
                        commit_hash: row.get(0),
                        repository_id: row.get(1),
                        author_date: Self::to_u64(row.get::<_, i64>(2))?,
                        message: row.get(3),
                        parent_commit: row.get(4),
                    })
                })
                .collect()
        })
    }

    fn executions_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbExecutionRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT execution_id, org_id, user_id, anon_user_id, workspace_id, repository_id, commit_hash,
                        EXTRACT(EPOCH FROM started_at)::BIGINT,
                        EXTRACT(EPOCH FROM completed_at)::BIGINT,
                        status, execution_tier
                 FROM executions
                 WHERE repository_id = $1
                 ORDER BY started_at",
                &[&repository_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    let completed_epoch: Option<i64> = row.get(8);
                    Ok(EidbExecutionRecord {
                        execution_id: row.get(0),
                        org_id: row.get(1),
                        user_id: row.get(2),
                        anon_user_id: row.get(3),
                        workspace_id: row.get(4),
                        repository_id: row.get(5),
                        commit_hash: row.get(6),
                        started_at: Self::to_u64(row.get::<_, i64>(7))?,
                        completed_at: completed_epoch.map(Self::to_u64).transpose()?,
                        status: row.get(9),
                        execution_tier: row.get(10),
                    })
                })
                .collect()
        })
    }

    fn journey_results_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbJourneyResultRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT journey_type, repo_id, success, time_to_url_ms
                 FROM journey_results
                 WHERE repo_id = $1
                 ORDER BY id",
                &[&repository_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    let time_to_url_ms = row.get::<_, i64>(3);
                    Ok(EidbJourneyResultRecord {
                        journey_type: row.get(0),
                        repo_id: row.get(1),
                        success: row.get(2),
                        time_to_url_ms: Self::to_u64(time_to_url_ms)?,
                    })
                })
                .collect()
        })
    }

    fn execution(&self, execution_id: &str) -> PersistenceResult<Option<EidbExecutionRecord>> {
        self.with_client(|client| {
            let row = client.query_opt(
                "SELECT execution_id, org_id, user_id, anon_user_id, workspace_id, repository_id, commit_hash,
                        EXTRACT(EPOCH FROM started_at)::BIGINT,
                        EXTRACT(EPOCH FROM completed_at)::BIGINT,
                        status, execution_tier
                 FROM executions
                 WHERE execution_id = $1",
                &[&execution_id],
            )?;

            row.map(|row| {
                let completed_epoch: Option<i64> = row.get(8);
                Ok(EidbExecutionRecord {
                    execution_id: row.get(0),
                    org_id: row.get(1),
                    user_id: row.get(2),
                    anon_user_id: row.get(3),
                    workspace_id: row.get(4),
                    repository_id: row.get(5),
                    commit_hash: row.get(6),
                    started_at: Self::to_u64(row.get::<_, i64>(7))?,
                    completed_at: completed_epoch.map(Self::to_u64).transpose()?,
                    status: row.get(9),
                    execution_tier: row.get(10),
                })
            })
            .transpose()
        })
    }

    fn events_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbExecutionEventRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT execution_id, event_type, EXTRACT(EPOCH FROM created_at)::BIGINT
                 FROM execution_events
                 WHERE execution_id = $1
                 ORDER BY created_at",
                &[&execution_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbExecutionEventRecord {
                        execution_id: row.get(0),
                        event_type: row.get(1),
                        created_at: Self::to_u64(row.get::<_, i64>(2))?,
                    })
                })
                .collect()
        })
    }

    fn url_allocations_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbUrlAllocationRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT workspace_url, execution_id,
                        EXTRACT(EPOCH FROM created_at)::BIGINT,
                        EXTRACT(EPOCH FROM released_at)::BIGINT
                 FROM url_allocations
                 WHERE execution_id = $1
                 ORDER BY created_at",
                &[&execution_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    let released_epoch: Option<i64> = row.get(3);
                    Ok(EidbUrlAllocationRecord {
                        workspace_url: row.get(0),
                        execution_id: row.get(1),
                        created_at: Self::to_u64(row.get::<_, i64>(2))?,
                        released_at: released_epoch.map(Self::to_u64).transpose()?,
                    })
                })
                .collect()
        })
    }

    fn healing_attempts_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbHealingAttemptRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT repository_id, execution_id, failure_class, repair_strategy, success,
                        EXTRACT(EPOCH FROM created_at)::BIGINT
                 FROM healing_attempts
                 WHERE execution_id = $1
                 ORDER BY created_at",
                &[&execution_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbHealingAttemptRecord {
                        repository_id: row.get(0),
                        execution_id: row.get(1),
                        failure_class: row.get(2),
                        repair_strategy: row.get(3),
                        success: row.get(4),
                        created_at: Self::to_u64(row.get::<_, i64>(5))?,
                    })
                })
                .collect()
        })
    }

    fn warm_pool_usage_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbWarmPoolUsageRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT execution_id, image_id, cache_hit, cold_start, startup_time_ms
                 FROM warm_pool_usage
                 WHERE execution_id = $1
                 ORDER BY id",
                &[&execution_id],
            )?;

            Ok(rows
                .into_iter()
                .map(|row| EidbWarmPoolUsageRecord {
                    execution_id: row.get(0),
                    image_id: row.get(1),
                    cache_hit: row.get(2),
                    cold_start: row.get(3),
                    startup_time_ms: row.get::<_, f64>(4),
                })
                .collect())
        })
    }

    fn healing_attempts_for_repository(
        &self,
        repository_id: &str,
    ) -> PersistenceResult<Vec<EidbHealingAttemptRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT repository_id, execution_id, failure_class, repair_strategy, success,
                        EXTRACT(EPOCH FROM created_at)::BIGINT
                 FROM healing_attempts
                 WHERE repository_id = $1
                 ORDER BY created_at",
                &[&repository_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbHealingAttemptRecord {
                        repository_id: row.get(0),
                        execution_id: row.get(1),
                        failure_class: row.get(2),
                        repair_strategy: row.get(3),
                        success: row.get(4),
                        created_at: Self::to_u64(row.get::<_, i64>(5))?,
                    })
                })
                .collect()
        })
    }

    fn billing_events_for_execution(
        &self,
        execution_id: &str,
    ) -> PersistenceResult<Vec<EidbBillingEventRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT event_id, org_id, user_id, workspace_id, execution_id,
                        event_type, runtime_type, resource_usage, cost_units,
                        EXTRACT(EPOCH FROM timestamp)::BIGINT
                 FROM billing_events
                 WHERE execution_id = $1
                 ORDER BY timestamp",
                &[&execution_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbBillingEventRecord {
                        event_id: row.get(0),
                        org_id: row.get(1),
                        user_id: row.get(2),
                        workspace_id: row.get(3),
                        execution_id: row.get(4),
                        event_type: row.get(5),
                        runtime_type: row.get(6),
                        resource_usage: row.get::<_, Value>(7),
                        cost_units: row.get::<_, f64>(8),
                        timestamp: Self::to_u64(row.get::<_, i64>(9))?,
                    })
                })
                .collect()
        })
    }

    fn billing_events_for_org(&self, org_id: &str) -> PersistenceResult<Vec<EidbBillingEventRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT event_id, org_id, user_id, workspace_id, execution_id,
                        event_type, runtime_type, resource_usage, cost_units,
                        EXTRACT(EPOCH FROM timestamp)::BIGINT
                 FROM billing_events
                 WHERE org_id = $1
                 ORDER BY timestamp",
                &[&org_id],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbBillingEventRecord {
                        event_id: row.get(0),
                        org_id: row.get(1),
                        user_id: row.get(2),
                        workspace_id: row.get(3),
                        execution_id: row.get(4),
                        event_type: row.get(5),
                        runtime_type: row.get(6),
                        resource_usage: row.get::<_, Value>(7),
                        cost_units: row.get::<_, f64>(8),
                        timestamp: Self::to_u64(row.get::<_, i64>(9))?,
                    })
                })
                .collect()
        })
    }

    fn billing_events(&self) -> PersistenceResult<Vec<EidbBillingEventRecord>> {
        self.with_client(|client| {
            let rows = client.query(
                "SELECT event_id, org_id, user_id, workspace_id, execution_id,
                        event_type, runtime_type, resource_usage, cost_units,
                        EXTRACT(EPOCH FROM timestamp)::BIGINT
                 FROM billing_events
                 ORDER BY timestamp",
                &[],
            )?;

            rows.into_iter()
                .map(|row| {
                    Ok(EidbBillingEventRecord {
                        event_id: row.get(0),
                        org_id: row.get(1),
                        user_id: row.get(2),
                        workspace_id: row.get(3),
                        execution_id: row.get(4),
                        event_type: row.get(5),
                        runtime_type: row.get(6),
                        resource_usage: row.get::<_, Value>(7),
                        cost_units: row.get::<_, f64>(8),
                        timestamp: Self::to_u64(row.get::<_, i64>(9))?,
                    })
                })
                .collect()
        })
    }

    fn last_good_commit_for_repository(&self, repository_id: &str) -> PersistenceResult<Option<String>> {
        self.with_client(|client| {
            let explicit = client.query_opt(
                "SELECT cer.commit_hash
                 FROM commit_execution_results cer
                 JOIN commits c ON c.commit_hash = cer.commit_hash
                 WHERE c.repository_id = $1 AND cer.success = TRUE
                 ORDER BY cer.recorded_at DESC
                 LIMIT 1",
                &[&repository_id],
            )?;

            if let Some(row) = explicit {
                return Ok(Some(row.get(0)));
            }

            let fallback = client.query_opt(
                "SELECT commit_hash
                 FROM executions
                 WHERE repository_id = $1
                   AND lower(status) IN ('success', 'succeeded', 'healthy')
                 ORDER BY started_at DESC
                 LIMIT 1",
                &[&repository_id],
            )?;

            Ok(fallback.map(|row| row.get(0)))
        })
    }
}

pub fn deserialize_string_array(value: Value) -> PersistenceResult<Vec<String>> {
    serde_json::from_value(value)
        .map_err(|err| ExecutionIntelligencePersistenceError::Serialization(err.to_string()))
}

pub fn infer_repository_from_commits(commits: &[EidbCommitRecord]) -> HashMap<String, String> {
    commits
        .iter()
        .map(|commit| (commit.commit_hash.clone(), commit.repository_id.clone()))
        .collect()
}
