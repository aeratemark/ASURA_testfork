-- Lease ledger for placement (M2).
--
-- world_pool.leased is a fast counter that makes one reservation statement atomic. This table is the
-- same information spelled out one row per lease, so that a lease can be *identified*: released by its
-- session when the player leaves, and reaped when the owning world stops heartbeating
-- (docs/02-architecture.md, "Travel, end to end"; docs/05-gw2-lessons.md, rule 2: leases are
-- reconciled from the worlds, not trusted blindly).
--
-- The invariant between the two: every row here was preceded by a counter increment in
-- world_pool.leased, and released/decremented together in one transaction, so
-- `count(lease) <= world_pool.leased <= capacity` at every commit.

CREATE TABLE IF NOT EXISTS lease (
    id          bigserial   PRIMARY KEY,
    world_id    int         NOT NULL REFERENCES world (id) ON DELETE CASCADE,
    map_id      int         NOT NULL,
    -- The session that owns this lease. A routing hint with a lifetime: the newest session for a
    -- player wins, and an older session's release is a no-op (docs/02-architecture.md, "Identity").
    session_id  uuid        NOT NULL,
    leased_at   timestamptz NOT NULL DEFAULT now()
);

-- One live slot per session: a session re-reserving (a second travel) supersedes its previous lease
-- rather than stacking two.
CREATE UNIQUE INDEX IF NOT EXISTS lease_one_per_session ON lease (session_id);

-- The reaper's query: every lease of a world that went silent.
CREATE INDEX IF NOT EXISTS lease_by_world ON lease (world_id, map_id);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (2) ON CONFLICT (version) DO NOTHING;
