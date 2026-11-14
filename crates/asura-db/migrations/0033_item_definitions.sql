-- The link from an owned item to its static definition (item catalogue, stage 1).
--
-- An item definition is content: its model, file id, item type, default name, inherent modifiers,
-- stack behaviour and equipment positions live in `config/World/<pack>/items.json` and never in
-- this database (`docs/38-layer-decisions.md` D20-D24). This column is only the reference an
-- owned row carries into that catalogue. It is nullable because every row written before the
-- catalogue existed is still a complete `CREATE_NAMED_ITEM`: the row keeps all the wire fields,
-- and a null link means "this pack has not been asked to name it", not "this item has no
-- definition".
--
-- No name, model, type, modifier or capacity column is added here. Copying those into `t_item`
-- would make the database a second catalogue and let the two disagree; the row's existing wire
-- columns remain because they are the evidence of what this instance was sent as.
ALTER TABLE t_item ADD COLUMN IF NOT EXISTS definition_id text;

-- Looking an item up by definition is how a later catalogue edit finds the rows it describes.
-- Partial because a legacy row has no definition to index.
CREATE INDEX IF NOT EXISTS item_by_definition
    ON t_item (definition_id) WHERE definition_id IS NOT NULL;

INSERT INTO schema_version (version) VALUES (33) ON CONFLICT (version) DO NOTHING;
