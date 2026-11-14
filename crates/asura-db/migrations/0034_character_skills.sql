-- A character's skills: the ids it has learned, the eight slots of its bar, and its attribute ranks.
--
-- WHAT IS STORED. Ids and ranks, and nothing a skill *is*. What skill 281 costs, what it does and which
-- profession it belongs to is content - `config/World/<pack>/skills.json` - and the same for every
-- character (`docs/38-layer-decisions.md`, D20-D24). A row that copied those would disagree with the pack
-- the moment the pack changes. The pack's rules decide whether a bar or a rank may stand
-- (`asura_rules::skills`); this table only remembers what the character chose (`docs/52-skills.md`).
--
-- ONE ROW PER CHARACTER, THREE ARRAYS. The bar is eight (skill, copy) pairs, kept as two parallel
-- eight-element arrays because that is how the client's `0x00DA` carries it. Ranks are one entry per
-- attribute index, 0..50, the order of `attributes.json`. A character with no row has learned nothing,
-- has an empty bar and no ranks - which is a new character, not an error.
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21), with D31's `t_` prefix.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS t_character_skills (
    character_id uuid        PRIMARY KEY REFERENCES t_characters (id) ON DELETE CASCADE,
    -- Skill ids the character has learned, each once.
    learned      integer[]   NOT NULL DEFAULT '{}',
    -- The bar's eight skill ids, slot 0 first; 0 is an empty slot.
    bar_skills   integer[]   NOT NULL DEFAULT '{0,0,0,0,0,0,0,0}'
                             CHECK (cardinality(bar_skills) = 8),
    -- Each slot's copy index; 0 for every skill a character holds once.
    bar_copies   integer[]   NOT NULL DEFAULT '{0,0,0,0,0,0,0,0}'
                             CHECK (cardinality(bar_copies) = 8),
    -- One rank per attribute index 0..50.
    ranks        smallint[]  NOT NULL DEFAULT '{}',
    updated_at   timestamptz NOT NULL DEFAULT now()
);

INSERT INTO schema_version (version) VALUES (34) ON CONFLICT (version) DO NOTHING;
