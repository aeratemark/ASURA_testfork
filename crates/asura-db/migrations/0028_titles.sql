-- Title progress: points per (character, title) and per (account, title), and the title a character shows.
--
-- WHAT IS STORED. Points, and nothing derived from them. Which rank a point count is, what the next rank
-- needs and what the records are called is content - `config/Server/titles.json` - and the same for every
-- character; it changes with the game data, not with play. Storing a rank beside the points would let the
-- two disagree the moment that file changes (docs/40-titles-system-research.md, section 10.2).
--
-- TWO TABLES, ONE PER SCOPE. The game splits titles into character progress (Cartographer, Drunkard,
-- LDoA, ...) and account progress (Lucky, Treasure Hunter, Wisdom, ...). One table with both keys and a
-- null in one of them (the shape OpenTyria leaves unused) makes the scope a reading of which column is
-- null; two tables make it the key. Which table a title lives in is the title file's `scope`.
--
-- POINTS ARE A BIGINT CHECKED TO THE WIRE'S u32. `0x00F6` field 3 is a dword, and Kurzick/Luxon reach
-- 10,000,000, which an `int` holds - but a dword's top half does not, and a column that can hold a value
-- the wire cannot carry is a bug waiting for its first row.
--
-- WHAT A CHARACTER SHOWS is one nullable column on the character: the title id it asked to show
-- (`GAME_CMSG_TITLE_DISPLAY`), or null for none. The rank it shows is derived from the points at the time,
-- like everything else here.
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21), with D31's `t_` prefix.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS t_character_titles (
    character_id uuid        NOT NULL REFERENCES t_characters (id) ON DELETE CASCADE,
    -- The client's title id: its own table has 48 rows.
    title_id     smallint    NOT NULL CHECK (title_id BETWEEN 0 AND 47),
    points       bigint      NOT NULL DEFAULT 0 CHECK (points BETWEEN 0 AND 4294967295),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (character_id, title_id)
);

CREATE TABLE IF NOT EXISTS t_account_titles (
    account_id   uuid        NOT NULL REFERENCES t_account (id) ON DELETE CASCADE,
    title_id     smallint    NOT NULL CHECK (title_id BETWEEN 0 AND 47),
    points       bigint      NOT NULL DEFAULT 0 CHECK (points BETWEEN 0 AND 4294967295),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, title_id)
);

ALTER TABLE t_characters
    ADD COLUMN IF NOT EXISTS display_title smallint CHECK (display_title BETWEEN 0 AND 47);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (28) ON CONFLICT (version) DO NOTHING;
