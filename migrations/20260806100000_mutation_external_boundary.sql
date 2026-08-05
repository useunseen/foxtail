-- Bind the disposable mutation ledger to the public EC2 boundary.  The
-- SQLite rows are a reconciliation record; external_status and
-- external_identity_verified are never a substitute for a public describe.
ALTER TABLE fixture_mutation_generations ADD COLUMN endpoint_url TEXT NOT NULL DEFAULT 'http://localhost:4566';
ALTER TABLE fixture_mutation_generations ADD COLUMN region TEXT NOT NULL DEFAULT 'us-east-1';
ALTER TABLE fixture_mutation_generations ADD COLUMN account_id TEXT NOT NULL DEFAULT '123456789012';
ALTER TABLE fixture_mutation_generations ADD COLUMN external_status TEXT NOT NULL DEFAULT 'UNKNOWN'
    CHECK (external_status IN ('UNKNOWN', 'PROVISIONING', 'PROVISIONED', 'ACTIVE', 'DESTROYED', 'ABSENT', 'FAILED', 'AMBIGUOUS'));

-- SQLite cannot alter a CHECK constraint. Rebuild this small ledger table and
-- translate the pre-boundary names while retaining existing archaeology.
ALTER TABLE fixture_mutation_resources RENAME TO fixture_mutation_resources_legacy;
CREATE TABLE fixture_mutation_resources (
    resource_id TEXT PRIMARY KEY,
    mutation_generation INTEGER NOT NULL,
    generation_id TEXT NOT NULL,
    control_id TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('stop', 'resize', 'stop-recovery', 'resize-restoration')),
    setup_fault_kind TEXT NOT NULL CHECK (setup_fault_kind IN ('stop', 'resize')),
    instance_state TEXT NOT NULL CHECK (instance_state IN ('running', 'stopped', 'terminated')),
    instance_type TEXT NOT NULL,
    initial_state TEXT NOT NULL,
    initial_type TEXT NOT NULL,
    terminal_state TEXT NOT NULL,
    terminal_type TEXT NOT NULL,
    restored_state TEXT NOT NULL,
    restored_type TEXT NOT NULL,
    external_status TEXT NOT NULL DEFAULT 'UNKNOWN'
        CHECK (external_status IN ('UNKNOWN', 'PROVISIONED', 'ACTIVE', 'FAULTED', 'RESTORED', 'DESTROYED', 'ABSENT', 'FAILED', 'AMBIGUOUS')),
    external_identity_verified INTEGER NOT NULL DEFAULT 0 CHECK (external_identity_verified IN (0, 1)),
    created_at TEXT NOT NULL,
    retired_at TEXT,
    FOREIGN KEY (mutation_generation) REFERENCES fixture_mutation_generations(mutation_generation)
);

INSERT INTO fixture_mutation_resources (
    resource_id, mutation_generation, generation_id, control_id, target_kind,
    setup_fault_kind, instance_state, instance_type, initial_state, initial_type,
    terminal_state, terminal_type, restored_state, restored_type, external_status,
    external_identity_verified, created_at, retired_at
)
SELECT resource_id, mutation_generation, generation_id, control_id,
       CASE target_kind
           WHEN 'recovery' THEN 'stop-recovery'
           WHEN 'restoration' THEN 'resize-restoration'
           ELSE target_kind
       END,
       CASE target_kind
           WHEN 'resize' THEN 'resize'
           WHEN 'restoration' THEN 'resize'
           ELSE 'stop'
       END,
       instance_state, instance_type, initial_state, initial_type,
       terminal_state, terminal_type, restored_state, restored_type,
       'UNKNOWN', 0, created_at, retired_at
FROM fixture_mutation_resources_legacy;
DROP TABLE fixture_mutation_resources_legacy;

CREATE INDEX idx_fixture_mutation_resources_generation
    ON fixture_mutation_resources(mutation_generation, control_id);

-- An intent is committed before any external dispatch. A second transaction
-- can only finalize it after public state has been reconciled. AMBIGUOUS is a
-- terminal, fail-closed state and requires an operator to inspect/repair it.
CREATE TABLE fixture_mutation_intents (
    intent_id TEXT PRIMARY KEY,
    operation TEXT NOT NULL CHECK (operation IN ('realize', 'recreate', 'fault', 'reset', 'destroy')),
    mutation_generation INTEGER,
    generation_id TEXT,
    fixture_generation INTEGER,
    target_id TEXT,
    request_bytes BLOB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('INTENT', 'DISPATCHED', 'SUCCEEDED', 'FAILED', 'AMBIGUOUS')),
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_fixture_mutation_intents_active
    ON fixture_mutation_intents(status, operation, mutation_generation);
