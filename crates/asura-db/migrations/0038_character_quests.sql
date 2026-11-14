-- A character's quest attempts (docs/58-quests.md).
--
-- WHY A TABLE. The stage machine is `asura_rules::quest`; the row is what survives a reconnect. One row
-- per (character, quest): the attempt cycle and the stage, and nothing derived. The content - the log
-- text, markers, offer and reward values - is the pack's (`config/World/<pack>/quests.json`), not a row.
--
-- WHAT IS NOT HERE YET. Objective credits, a timed attempt and a per-map marker are the pieces the first
-- quest (80, no objectives) does not need. They join the table when the first quest that needs them is
-- ported, rather than as empty columns nobody reads.
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21), with D31's `t_` prefix.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS t_character_quests (
    character_id uuid        NOT NULL REFERENCES t_characters (id) ON DELETE CASCADE,
    -- The client's quest id (`GAME_SMSG_QUEST_ADD` field 1).
    quest_id     integer     NOT NULL CHECK (quest_id > 0),
    -- The attempt number: a reacceptance increments it, and a delayed packet from an old attempt is
    -- refused by naming the old cycle (`gw_quest_state.h`).
    cycle        integer     NOT NULL CHECK (cycle > 0),
    -- 1 active, 2 completed, 3 rewarded, 4 abandoned (`Stage` in `asura-rules/src/quest.rs`).
    stage        smallint    NOT NULL CHECK (stage BETWEEN 1 AND 4),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (character_id, quest_id)
);

INSERT INTO schema_version (version) VALUES (38) ON CONFLICT (version) DO NOTHING;
