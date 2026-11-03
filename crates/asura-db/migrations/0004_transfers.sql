-- The pending transfer (M4).
--
-- When the auth service places a travel it issues a `player_token` and tells the client where to go.
-- The client's next connection presents that token (plus the world's id as `map_token`, the map, and
-- both UUIDs) in `GAME_CMSG_VERSION`, and the world server has to decide whether the pair is real.
--
-- GWPS answers that question in memory, in the auth process, and then hands the *socket itself* to the
-- game server over its control connection (`CtrlMsgId_TransferConnection`, AuthSrv.c:1515). That is
-- one way to keep the token private. This project's coordination is the database
-- (`docs/03-protocol.md` section 8), so the pending transfer is a row: the auth service writes it, the
-- world claims it, and the row is the thing both processes agree on. The observable protocol on the
-- wire does not change -- the client still presents a token it was just given.
--
-- Why `player_token` is the primary key: it is the only value the client presents that the world
-- cannot already know, and one live travel issues exactly one of them. It is a random u32 in the
-- protocol, widened here because a primary key should not be the narrowest type that happens to fit
-- today.
--
-- `claimed_at` rather than deleting the row: a claimed transfer is evidence for the trace ("this
-- token was used, then"), and a replayed token must be refused by looking at it rather than by
-- noticing it is missing. Nothing depends on the row being gone.
--
-- The partial index covers the only query the world makes: pending transfers for my world.

CREATE TABLE IF NOT EXISTS transfer (
    player_token  bigint      PRIMARY KEY,
    world_id      int         NOT NULL REFERENCES world (id) ON DELETE CASCADE,
    map_id        int         NOT NULL,
    session_id    uuid        NOT NULL REFERENCES session (id) ON DELETE CASCADE,
    account_id    uuid        NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    character_id  uuid        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    claimed_at    timestamptz
);

CREATE INDEX IF NOT EXISTS transfer_pending
    ON transfer (world_id) WHERE claimed_at IS NULL;

-- The map a session is playing, beside the world it was sent to. `world_id` has been on the session
-- since 0001; a session that is in a map needs both, because "world 8001, map 146" and "world 8001,
-- map 15" are different places and only the lease knows which.
ALTER TABLE session ADD COLUMN IF NOT EXISTS map_id int;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (4) ON CONFLICT (version) DO NOTHING;
