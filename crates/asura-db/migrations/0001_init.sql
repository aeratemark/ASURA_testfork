-- Initial schema for ASURA.
--
-- PostgreSQL 15 or later. This is a deliberate, recorded deviation from AzerothCore's MySQL:
-- their structure is followed in full (registry, pool, async queue, transactions), and the vendor is
-- chosen for this workload (docs/adr/0008-database-vendor.md).
--
-- Three logical databases in production -- auth, game, content -- mirroring AzerothCore's Login /
-- Character / World split. This file is their union so it can be applied to one database during
-- development and split later without rewriting statements.
--
-- The registry and pool tables are the coordination channel between the auth service and the world
-- servers (docs/02-architecture.md): placement must be atomic across processes, and the database is
-- the only arbiter both kinds of program agree on.

CREATE TABLE IF NOT EXISTS schema_version (
    version     int         PRIMARY KEY,
    applied_at  timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------- auth domain

CREATE TABLE IF NOT EXISTS account (
    id          uuid        PRIMARY KEY,
    username    text        NOT NULL UNIQUE,
    -- Login token the world server validates a reconnecting client against.
    token       uuid,
    token_at    timestamptz,
    last_ip     inet,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS session (
    id          uuid        PRIMARY KEY,
    account_id  uuid        NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    char_id     uuid,
    world_id    int,
    started_at  timestamptz NOT NULL DEFAULT now(),
    -- The newest session for an account wins; older ones are closed, never honoured
    -- (docs/05-gw2-lessons.md, rule 1).
    ended_at    timestamptz
);

-- A partial index: only live sessions are indexed, because that is the only shape we query.
-- MySQL has no equivalent, which is one of the reasons for the vendor choice.
CREATE INDEX IF NOT EXISTS session_active
    ON session (account_id) WHERE ended_at IS NULL;

-- ---------------------------------------------------------------- registry

CREATE TABLE IF NOT EXISTS world (
    id            int         PRIMARY KEY,
    name          text        NOT NULL,
    address       text        NOT NULL,
    port          int         NOT NULL,
    build         text        NOT NULL,
    draining      boolean     NOT NULL DEFAULT false,
    population    int         NOT NULL DEFAULT 0,
    heartbeat_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS world_pool (
    world_id   int  NOT NULL REFERENCES world (id) ON DELETE CASCADE,
    map_id     int  NOT NULL,
    capacity   int  NOT NULL CHECK (capacity >= 0),
    leased     int  NOT NULL DEFAULT 0 CHECK (leased >= 0),
    PRIMARY KEY (world_id, map_id),
    -- The invariant that makes a single atomic UPDATE safe: a map can never be oversold, even if the
    -- application is wrong (docs/02-architecture.md, "Travel, end to end").
    CONSTRAINT world_pool_never_oversold CHECK (leased <= capacity)
);

-- ---------------------------------------------------------------- game domain

CREATE TABLE IF NOT EXISTS characters (
    id          uuid        PRIMARY KEY,
    account_id  uuid        NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    name        text        NOT NULL,
    map_id      int         NOT NULL,
    pos_x       real        NOT NULL DEFAULT 0,
    pos_y       real        NOT NULL DEFAULT 0,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS characters_by_account ON characters (account_id);

-- ---------------------------------------------------------------- content domain

CREATE TABLE IF NOT EXISTS map_config (
    map_id        int     PRIMARY KEY,
    map_type      int     NOT NULL,          -- 0 = outpost/town, 1 = explorable
    map_file_id   bigint  NOT NULL,
    spawn_count   int     NOT NULL DEFAULT 0
);

-- ---------------------------------------------------------------- bookkeeping

-- Record that this migration ran, so a database can say what state it is in instead of being
-- guessed at (crates/asura-db/src/migrations.rs). The version is the file's number: 0001 is 1.
-- ON CONFLICT DO NOTHING keeps it idempotent, like every other statement here.
INSERT INTO schema_version (version) VALUES (1) ON CONFLICT (version) DO NOTHING;

