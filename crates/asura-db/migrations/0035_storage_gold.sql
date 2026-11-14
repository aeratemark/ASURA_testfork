-- The gold a character keeps in Xunlai storage (`docs/53-npc-services.md`).
--
-- WHY A COLUMN ON THE CHARACTER. Storage's bags are the character's own rows in this schema (`t_bag`,
-- migration 0006, bag models 5 and 7+), so the gold beside them is the character's too. Retail keeps
-- storage per account; moving bags and gold to the account is one later change for both, not this one.
--
-- WHAT IT IS. A balance, 0 to 1,000,000 (the client's own cap), changed only at a Xunlai window
-- (`GAME_CMSG_ITEM_CHANGE_GOLD`, `0x807C`) in the same transaction as the carried gold beside it. The
-- load sends it as `0x0141` after the carried gold's `0x0140`, when it is not 0
-- (preforged `packets-43292-168891734.jsonl` record 472026).
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21). Idempotent, like every migration.

ALTER TABLE t_characters ADD COLUMN IF NOT EXISTS gold_storage int NOT NULL DEFAULT 0
    CHECK (gold_storage >= 0);

INSERT INTO schema_version (version) VALUES (35) ON CONFLICT (version) DO NOTHING;
