-- Rows created by the pre-boundary mutation migration have no public
-- identity/readiness proof.  Quarantine them instead of allowing the
-- boundary's DEFAULT localhost endpoint or UNKNOWN status to become
-- dispatchable after an upgrade.
UPDATE fixture_mutation_generations
SET external_status = 'AMBIGUOUS'
WHERE state = 'ACTIVE' AND external_status <> 'ACTIVE';

INSERT OR IGNORE INTO fixture_mutation_intents (
    intent_id, operation, mutation_generation, generation_id,
    fixture_generation, request_bytes, status, error, created_at, updated_at
)
SELECT
    'upgrade-quarantine-' || mutation_generation,
    'realize',
    mutation_generation,
    generation_id,
    fixture_generation,
    CAST('{"reason":"legacy-mutation-generation-upgrade"}' AS BLOB),
    'AMBIGUOUS',
    'legacy mutation generation lacks public identity and readiness proof',
    created_at,
    created_at
FROM fixture_mutation_generations
WHERE state = 'ACTIVE' AND external_status = 'AMBIGUOUS';

-- The intermediate boundary migration did not serialize intents.  Keep the
-- oldest row for each generation and quarantine duplicate nonterminal rows
-- before attaching the unique index, so an upgrade cannot fail halfway.
WITH ranked AS (
    SELECT intent_id,
           ROW_NUMBER() OVER (
               PARTITION BY mutation_generation
               ORDER BY created_at ASC, intent_id ASC
           ) AS ordinal
    FROM fixture_mutation_intents
    WHERE mutation_generation IS NOT NULL
      AND status IN ('INTENT', 'DISPATCHED')
)
UPDATE fixture_mutation_intents
SET status = 'AMBIGUOUS',
    error = COALESCE(error, 'duplicate nonterminal mutation intent quarantined during upgrade'),
    updated_at = CURRENT_TIMESTAMP
WHERE intent_id IN (SELECT intent_id FROM ranked WHERE ordinal > 1);

-- Serialize every lifecycle operation for one mutation generation.  The
-- operation name is deliberately absent from this key: fault, reset,
-- recreate, and destroy must never dispatch concurrently for the same
-- externally-owned generation.
CREATE UNIQUE INDEX idx_fixture_mutation_intents_one_inflight_generation
    ON fixture_mutation_intents(mutation_generation)
    WHERE mutation_generation IS NOT NULL
      AND status IN ('INTENT', 'DISPATCHED');
