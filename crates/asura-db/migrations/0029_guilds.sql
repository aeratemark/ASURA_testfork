-- Guilds: the guild, its members, and its open invitations.
--
-- WHAT IS STORED. What a guild's own messages carry and play changes: the name and tag, the cape, the
-- standing numbers `0x0121` sends (allegiance, faction, qualifier, rating, rank and the two bytes whose
-- meaning is not established), the announcement and who set it, and the counter `0x0133` sends once per
-- change (docs/41-guild-system-research.md, sections 2 and 4). What a new guild starts with is
-- `config/Server/guilds.json`; the row is written from it once, at creation.
--
-- THE KEY IS THE ID. Every guild message names the guild by a sixteen-byte key (`0x0118`, `0x0121`); a
-- guid. The row's id is that key, so there is one identifier and not two that can disagree.
--
-- ONE GUILD PER CHARACTER is the primary key of the member table. An invitation is not a membership: the
-- invited character stays in no guild until it accepts, so invitations are their own table and a character
-- may hold several.
--
-- MEMBER TYPE is the client's number (`0x0127`'s last byte, `0x0118`'s role): 5 the leader is the only
-- member type measured. Kept as the client's number rather than a name so the wire and the row agree.
--
-- OPERATIONAL STATE, NOT CONTENT (docs/38-layer-decisions.md, D21), with D31's `t_` prefix.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS t_guilds (
    id                  uuid        PRIMARY KEY,
    name                text        NOT NULL,
    tag                 text        NOT NULL,
    feature             smallint    NOT NULL DEFAULT 0 CHECK (feature BETWEEN 0 AND 255),
    territory           smallint    NOT NULL DEFAULT 0 CHECK (territory BETWEEN 0 AND 255),
    cape_background     smallint    NOT NULL DEFAULT 0 CHECK (cape_background BETWEEN 0 AND 255),
    cape_detail_color   smallint    NOT NULL DEFAULT 0 CHECK (cape_detail_color BETWEEN 0 AND 255),
    cape_emblem_color   smallint    NOT NULL DEFAULT 0 CHECK (cape_emblem_color BETWEEN 0 AND 255),
    cape_shape          smallint    NOT NULL DEFAULT 0 CHECK (cape_shape BETWEEN 0 AND 255),
    cape_detail         smallint    NOT NULL DEFAULT 0 CHECK (cape_detail BETWEEN 0 AND 255),
    cape_emblem         integer     NOT NULL DEFAULT 0 CHECK (cape_emblem BETWEEN 0 AND 65535),
    cape_trim           smallint    NOT NULL DEFAULT 0 CHECK (cape_trim BETWEEN 0 AND 255),
    allegiance          smallint    NOT NULL DEFAULT 0 CHECK (allegiance BETWEEN 0 AND 255),
    faction             bigint      NOT NULL DEFAULT 0 CHECK (faction BETWEEN 0 AND 4294967295),
    qualifier           bigint      NOT NULL DEFAULT 0 CHECK (qualifier BETWEEN 0 AND 4294967295),
    rating              bigint      NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 4294967295),
    rank                bigint      NOT NULL DEFAULT 0 CHECK (rank BETWEEN 0 AND 4294967295),
    announcement        text        NOT NULL DEFAULT '',
    announcement_author text        NOT NULL DEFAULT '',
    revision            bigint      NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 4294967295),
    created_at          timestamptz NOT NULL DEFAULT now()
);

-- Names are unique whatever their case; tags are not unique (the game reuses them).
CREATE UNIQUE INDEX IF NOT EXISTS t_guilds_name_key ON t_guilds (lower(name));

CREATE TABLE IF NOT EXISTS t_guild_members (
    character_id uuid        PRIMARY KEY REFERENCES t_characters (id) ON DELETE CASCADE,
    guild_id     uuid        NOT NULL REFERENCES t_guilds (id) ON DELETE CASCADE,
    member_type  smallint    NOT NULL CHECK (member_type BETWEEN 0 AND 255),
    -- `0x0127`'s first and third names: the name it was invited under, and who invited it.
    invited_name text        NOT NULL,
    invited_by   text        NOT NULL,
    joined_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS t_guild_members_guild ON t_guild_members (guild_id);

CREATE TABLE IF NOT EXISTS t_guild_invites (
    guild_id     uuid        NOT NULL REFERENCES t_guilds (id) ON DELETE CASCADE,
    character_id uuid        NOT NULL REFERENCES t_characters (id) ON DELETE CASCADE,
    invited_name text        NOT NULL,
    invited_by   text        NOT NULL,
    invited_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, character_id)
);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (29) ON CONFLICT (version) DO NOTHING;
