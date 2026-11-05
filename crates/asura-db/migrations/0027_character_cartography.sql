-- A character's explored cells: one row per (character, continent).
--
-- WHOSE PROGRESS THIS IS. The character's, not the account's. That is how the game itself does it -
-- the Cartographer titles live on the character, and some underground areas reset their exploration
-- when a character leaves them (Rurik `studies/minimap/FINDINGS.md`, the Cartographer section,
-- corroborated from the titles rather than from a save format). So the key starts with the
-- character and two characters on one account explore separately.
--
-- WHY CONTINENT AND NOT MAP. The client keeps one bitmap per continent and the grid is
-- continent-absolute: `GAME_SMSG_MAP_INIT_START` replaces the whole bitmap on every instance load,
-- and a map's own area is a rectangle inside it. Measured on the wire: 64 x 128 cells for continent
-- 1 on 8 of 8 captures, and 256 x 192 from a character-creation instance - a different continent, a
-- different grid. A per-map row would have to be reassembled into the continent's grid on every load,
-- and nothing would own the cells that two maps' rectangles share.
--
-- A BLOB AND NOT ROWS, which is the opposite of what migration 0023 decided for instance unlocks,
-- so the reason matters. An unlock is a fact with a time that something asks about by instance
-- ("has this character seen map 148"), and there are hundreds of them. A continent's exploration is
-- 8,192 bits that are only ever read as a whole and written as a whole, because the whole grid is
-- what one message carries; a row per cell would be 8,192 rows per character per continent to
-- answer a question nobody asks cell by cell.
--
-- PACKED BITS AND NOT THE WIRE STREAM. The column could have held the compressed stream the client
-- is sent, which would be smaller still. It does not, because that stream's polarity is a reading of
-- the client's expander rather than a certainty (`crates/asura-world/src/cartography.rs`,
-- `REVEALED_FIRST`): storing it would mean that changing that constant silently reinterprets every
-- row already saved, turning explored into unexplored for every character at once. Packed bits mean
-- one thing only - bit `y * dims_x + x`, LSB first within each byte - and the stream is built from
-- them on the way out.
--
-- DIMS ARE STORED WITH THE BITS, because they are the only thing that gives the bits a shape, and
-- only continent 1's are measured. A row whose dims no longer match what the server would send for
-- that continent is refused on load and the character starts the continent fresh, rather than having
-- its cells drawn against the wrong width. That is a loss of progress, and it is the smaller harm:
-- the alternative is a map revealed in the wrong places with no way to tell that is what happened.
--
-- `explored` is the count of set bits, kept beside them so a percentage - which is what a
-- Cartographer title is - does not need the blob unpacked, and so that a row can be sanity-checked
-- against itself.
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21): this row changes while the
-- server runs, so it stays in the database. The prefixed name is D31's namespace marker.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS t_character_cartography (
    character_id uuid        NOT NULL REFERENCES t_characters (id) ON DELETE CASCADE,
    -- The continent the grid belongs to. 1 is Tyria, which is the only one this server serves.
    continent    smallint    NOT NULL,
    -- The grid the bits were packed against, in cells.
    dims_x       smallint    NOT NULL,
    dims_y       smallint    NOT NULL,
    -- One bit per cell, row-major, LSB first within a byte: `dims_x * dims_y / 8` bytes, which is
    -- 1,024 for continent 1's 64 x 128.
    blocks       bytea       NOT NULL,
    -- How many of those bits are set.
    explored     int         NOT NULL DEFAULT 0,
    updated_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (character_id, continent)
);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (27) ON CONFLICT (version) DO NOTHING;
