-- Preserve the stable EC2 observation metadata needed by the fixture's
-- AWS-compatible DescribeInstances surface. Existing hand-seeded rows use the
-- deterministic defaults; generator-discovered rows overwrite them with the
-- public LocalStack values.
ALTER TABLE resources ADD COLUMN instance_state TEXT NOT NULL DEFAULT 'running';
ALTER TABLE resources ADD COLUMN instance_type TEXT NOT NULL DEFAULT 'm6i.large';
ALTER TABLE resources ADD COLUMN availability_zone TEXT;
