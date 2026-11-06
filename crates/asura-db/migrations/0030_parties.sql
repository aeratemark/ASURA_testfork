-- Parties that travel: the ledger a party crosses from one instance into another with.
--
-- WHY A TABLE. A party lives in the world process that runs its instance (`asura_world::party`). A
-- travel ends that process's knowledge of it: every member reconnects, maybe to another world, and the
-- auth service decides where a member that logs in again enters. The database is the one thing the auth
-- service and every world agree on (docs/02-architecture.md), so the party crosses through here
-- (owner, 2026-11-06: "Database ledger"). Nothing reads it while a party stays in one instance.
--
-- WHAT A ROW IS. `t_parties` is one move of one party: who leads it, which world and which map it is
-- going to or standing in. `t_party_members` is one character's place in it, in the party's order
-- (`seat`, the leader first), and how far it has come:
--
--   state 0  travelling: told to go, not arrived yet;
--   state 1  in the map;
--   state 2  held: its connection dropped in an explorable and the party keeps its place until
--            `held_until` (docs/22-world-model.md, "Member disconnect"; ledger R11).
--
-- ONE PARTY PER CHARACTER is the member table's primary key. A new move replaces the old party of every
-- character in it, held members included: when a party changes areas, a held member is left behind.
--
-- KEYED BY CHARACTER, not by session: a member that logs in again has a new session, and the hold is what
-- puts it back.
--
-- OPERATIONAL STATE, NOT CONTENT (docs/38-layer-decisions.md, D21), with D31's `t_` prefix.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS t_parties (
    id        uuid        PRIMARY KEY,
    leader    uuid        NOT NULL REFERENCES t_characters (id) ON DELETE CASCADE,
    world_id  integer     NOT NULL,
    map_id    integer     NOT NULL,
    moved_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS t_party_members (
    character_id uuid        PRIMARY KEY REFERENCES t_characters (id) ON DELETE CASCADE,
    party_id     uuid        NOT NULL REFERENCES t_parties (id) ON DELETE CASCADE,
    seat         smallint    NOT NULL CHECK (seat >= 0),
    state        smallint    NOT NULL CHECK (state BETWEEN 0 AND 2),
    held_until   timestamptz NULL
);

CREATE INDEX IF NOT EXISTS t_party_members_party ON t_party_members (party_id);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (30) ON CONFLICT (version) DO NOTHING;
