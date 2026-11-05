-- Prefix every data table with `t_`.
--
-- A table called `session`, `world`, `item` or `account` collides with the words this project uses for
-- the concepts those tables hold: the login session, the world server, an item on the wire. Reading a
-- query, a log line or a document, there is no way to tell which one is meant. The prefix makes the
-- namespace explicit: `t_session` is a row, `session` is the thing (docs/38-layer-decisions.md, D31).
--
-- The rename is conditional, so applying this file twice is a no-op: a rename cannot be written to be
-- repeatable any other way, because the old name is gone after the first run. 0001-0025 are not
-- rewritten, because they are the record of what a database that reaches this point has already had
-- done to it - they still name the old tables, and this file is where the names change.
DO $$
DECLARE
    pair  text[];
    pairs text[][] := ARRAY[
        ['account', 't_account'],
        ['bag', 't_bag'],
        ['characters', 't_characters'],
        ['instance_unlock', 't_instance_unlock'],
        ['item', 't_item'],
        ['lease', 't_lease'],
        ['map_config', 't_map_config'],
        ['map_portal', 't_map_portal'],
        ['map_spawn', 't_map_spawn'],
        ['service', 't_service'],
        ['session', 't_session'],
        ['transfer', 't_transfer'],
        ['world', 't_world'],
        ['world_pool', 't_world_pool']
    ];
BEGIN
    FOREACH pair SLICE 1 IN ARRAY pairs LOOP
        IF to_regclass('public.' || pair[1]) IS NOT NULL
           AND to_regclass('public.' || pair[2]) IS NULL THEN
            EXECUTE format('ALTER TABLE public.%I RENAME TO %I', pair[1], pair[2]);
        END IF;
    END LOOP;
END $$;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (26) ON CONFLICT (version) DO NOTHING;
