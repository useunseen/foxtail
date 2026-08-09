-- Persist the read-only EC2 termination-protection fact that the public
-- DescribeInstanceAttribute surface returns. SQLite stores it as an integer,
-- but fixture serialization and validation expose only a strict boolean.
ALTER TABLE resources ADD COLUMN disable_api_termination INTEGER NOT NULL DEFAULT 0;
