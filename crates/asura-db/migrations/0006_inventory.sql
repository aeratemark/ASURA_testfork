-- The inventory (M5): items, the bags that hold them, and the position a character stood in.
--
-- Why this replaces a byte table: until now the world server answered the client's item request with
-- the reference's own messages, replayed verbatim (`game::default_inventory`). The client was happy and
-- the server knew nothing: nothing could be equipped, moved, dropped or saved, because there was no row,
-- bag or slot to name. Those messages are now *data*: the same nine items, with the same ids, are rows,
-- and the block is built from them (`crates/asura-db/src/inventory.rs`,
-- `tools/gen-default-inventory.py`).
--
-- The shape follows the reference's own tables (`PreSearing/db/seed.sql`: `bags` and `items`), with the
-- three substitutions this schema already makes everywhere:
--
--   * a character is a uuid, not an (account_id, char_id) pair, because the character row is the only
--     owner an item needs;
--   * `bag_model_id` picks the bag's *kind* (0 backpack, 5 material storage, 21 equipped items...,
--     `GmInventory.h`), and `bag_id` is the number the client is told. The reference uses one for both --
--     it keys its rows by bag model and hands out bag ids at runtime -- which cannot name two bags of
--     the same kind. This schema keeps them apart;
--   * the reference persists neither an item's name nor its modifiers, and rebuilds the backpack's own
--     item from a compiled-in struct (`g_Backpack`, `GmDefaultArmors.c`). Here they are columns, because
--     `CREATE_NAMED_ITEM` carries them and a row that cannot reproduce the message it describes is not
--     a model of anything.
--
-- The item id is the client-visible id, and it comes from a sequence rather than from the row's
-- position: the reference allocates item ids at runtime (`GameSrv_AllocateItem`), so they are arbitrary
-- to a client as long as they are consistent within one session (`GAME_CMSG_DROP_ITEM` names one, and
-- `UPDATE_AGENT_VISUAL_EQUIPMENT` carries nine). Zero is not an id: the reference refuses item id 0 in
-- its drop handler, so the sequence starts at one.

-- Where the character stands, saved when it leaves and used when it enters. `pos_x`/`pos_y` have been
-- there since 0001 and were never written; the plane is what the client calls a plane index rather than
-- a height (0 on every pre-Searing map measured so far).
--
-- NULL means "no position has ever been recorded for this character", and it is why the column is
-- nullable: a coordinate of (0, 0) is a legal coordinate (it is outside the walkable geometry on every
-- map measured so far, but it is not out of range), so it cannot double as the sentinel.
ALTER TABLE characters ADD COLUMN IF NOT EXISTS pos_plane smallint;

-- The client-visible item id. `CREATE_NAMED_ITEM` carries it, and every later message about that item
-- (moving it, dropping it, wearing it) names it.
CREATE SEQUENCE IF NOT EXISTS item_id_seq AS integer START WITH 1;

CREATE TABLE IF NOT EXISTS bag (
    character_id   uuid NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    -- The number the client is told (`INVENTORY_CREATE_BAG.bag_id`). Per character, not global: a bag id
    -- only ever has to be unambiguous inside one client's inventory model.
    bag_id         int  NOT NULL,
    -- `BagModelId` (`GmInventory.h`): the kind of bag, and what the wire orders its bags by.
    bag_model_id   int  NOT NULL,
    -- `BagType` (`GmInventory.h`): bag, equipped, not collected, storage, material storage.
    bag_type       int  NOT NULL,
    slot_count     int  NOT NULL CHECK (slot_count >= 0),
    -- The item that *is* the bag (the backpack's own item, `bag_item_id` in the reference). 0 for the
    -- bags that have none, which is every bag but the backpack. Not a foreign key on purpose: the
    -- reference's two tables point at each other (a bag names its item, an item names its bag), and a
    -- cycle here would make the seed order-dependent for no gain.
    assoc_item_id  int  NOT NULL DEFAULT 0,
    PRIMARY KEY (character_id, bag_id)
);

CREATE TABLE IF NOT EXISTS item (
    id            int    PRIMARY KEY DEFAULT nextval('item_id_seq'),
    character_id  uuid   NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    -- Where the item sits. Both are NULL for an item that is not in a slot at all -- a bag's own item --
    -- and the CHECK below makes "one of the two" impossible.
    bag_id        int,
    slot          int    CHECK (slot >= 0),
    -- Everything `CREATE_NAMED_ITEM` (0x0161) carries, in its wire order and its wire widths. Bigint for
    -- the two unsigned 32-bit fields: PostgreSQL has no unsigned integer, and a file id or a flag word
    -- with the top bit set would not fit an int.
    file_id       bigint NOT NULL,
    item_type     int    NOT NULL,
    dye_tint      int    NOT NULL DEFAULT 0,
    dye_colors    int    NOT NULL DEFAULT 0,
    materials     int    NOT NULL DEFAULT 0,
    unk1          int    NOT NULL DEFAULT 0,
    flags         bigint NOT NULL DEFAULT 0,
    value         int    NOT NULL DEFAULT 0,
    model_id      int    NOT NULL,
    quantity      int    NOT NULL DEFAULT 1 CHECK (quantity >= 0),
    -- `ItemSetProfession` (0x015A) is only sent when this is not zero (Profession_None).
    profession    int    NOT NULL DEFAULT 0,
    -- The item's name, as the UTF-16 units the wire carries. Not `text`: the reference's backpack name is
    -- a Gw.dat template id packed into code units (0x21A8 0xD157 0xB58F 0x166F), which is not valid
    -- Unicode and would be mangled by any encoder. Unit for unit, so the message can be rebuilt.
    name          int[]  NOT NULL DEFAULT '{}',
    -- The modifier list, `CREATE_NAMED_ITEM`'s trailing counted array. Its count is ONE byte on the wire
    -- (`TYPE_NESTED_STRUCT`, `msgpack.c`), not the two every other counted field uses -- measured, see
    -- docs/03-protocol.md section 11.4.
    modifiers     bigint[] NOT NULL DEFAULT '{}',
    CONSTRAINT item_is_placed_or_is_its_own_bag CHECK ((bag_id IS NULL) = (slot IS NULL))
);

CREATE INDEX IF NOT EXISTS bag_by_character ON bag (character_id);
CREATE INDEX IF NOT EXISTS item_by_character ON item (character_id);

-- One item per slot. Partial, because an item that is a bag's own item has no slot and there may be
-- several of those (the reference's `items` primary key names a slot and so cannot store one at all).
CREATE UNIQUE INDEX IF NOT EXISTS item_in_one_slot
    ON item (character_id, bag_id, slot) WHERE bag_id IS NOT NULL;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (6) ON CONFLICT (version) DO NOTHING;
