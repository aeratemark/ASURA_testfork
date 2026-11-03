-- A world's start time (S3): "the newest build" is the build of the most recently started fresh
-- world hosting the map, so a restart of an old build cannot masquerade as a newer one. Set by
-- register_world on every start or restart (docs/16-implementation-plan.md, S3).

ALTER TABLE world ADD COLUMN IF NOT EXISTS started_at timestamptz NOT NULL DEFAULT now();

INSERT INTO schema_version (version) VALUES (11) ON CONFLICT (version) DO NOTHING;
