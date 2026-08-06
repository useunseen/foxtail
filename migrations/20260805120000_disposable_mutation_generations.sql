-- Disposable, fixture-owned mutation generations.  Read-only fixture rows
-- remain in `resources`; these tables are the authoritative lifecycle ledger
-- for qualification-only EC2 targets and their auditable receipts.
ALTER TABLE fixture_realizations ADD COLUMN mutation_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE fixture_realizations ADD COLUMN mutation_generation_id TEXT;
ALTER TABLE fixture_realizations ADD COLUMN complete_estate_fingerprint TEXT;

CREATE TABLE fixture_mutation_generations (
    mutation_generation INTEGER PRIMARY KEY,
    generation_id TEXT NOT NULL UNIQUE,
    fixture_generation INTEGER NOT NULL,
    manifest_digest TEXT NOT NULL,
    complete_estate_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('ACTIVE', 'DESTROYED')),
    resource_ids TEXT NOT NULL,
    public_absence TEXT,
    created_at TEXT NOT NULL,
    destroyed_at TEXT
);

CREATE TABLE fixture_mutation_resources (
    resource_id TEXT PRIMARY KEY,
    mutation_generation INTEGER NOT NULL,
    generation_id TEXT NOT NULL,
    control_id TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('stop', 'resize', 'recovery', 'restoration')),
    instance_state TEXT NOT NULL CHECK (instance_state IN ('running', 'stopped')),
    instance_type TEXT NOT NULL,
    initial_state TEXT NOT NULL,
    initial_type TEXT NOT NULL,
    terminal_state TEXT NOT NULL,
    terminal_type TEXT NOT NULL,
    restored_state TEXT NOT NULL,
    restored_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    retired_at TEXT,
    FOREIGN KEY (mutation_generation) REFERENCES fixture_mutation_generations(mutation_generation)
);

CREATE INDEX idx_fixture_mutation_resources_generation
    ON fixture_mutation_resources(mutation_generation, control_id);

CREATE TABLE fixture_faults (
    receipt_id TEXT PRIMARY KEY,
    mutation_generation INTEGER NOT NULL,
    generation_id TEXT NOT NULL,
    manifest_digest TEXT NOT NULL,
    control_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    fault_kind TEXT NOT NULL CHECK (fault_kind IN ('stop', 'resize')),
    applied_at TEXT NOT NULL,
    reset_token TEXT NOT NULL,
    prior_state TEXT NOT NULL,
    terminal_state TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ACTIVE', 'RESET')),
    reset_at TEXT,
    reset_receipt_id TEXT
);

CREATE INDEX idx_fixture_faults_active
    ON fixture_faults(mutation_generation, status, target_id);

CREATE TABLE fixture_operation_receipts (
    receipt_id TEXT PRIMARY KEY,
    operation TEXT NOT NULL,
    mutation_generation INTEGER,
    generation_id TEXT,
    manifest_digest TEXT,
    receipt_bytes BLOB NOT NULL,
    created_at TEXT NOT NULL
);
