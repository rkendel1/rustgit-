ALTER TABLE executions
    ALTER COLUMN org_id DROP NOT NULL,
    ALTER COLUMN user_id DROP NOT NULL;

ALTER TABLE executions
    ADD COLUMN IF NOT EXISTS anon_user_id TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'executions_owner_identity_check'
    ) THEN
        ALTER TABLE executions
            ADD CONSTRAINT executions_owner_identity_check
            CHECK (user_id IS NOT NULL OR anon_user_id IS NOT NULL);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_executions_anon_user_id ON executions(anon_user_id);
