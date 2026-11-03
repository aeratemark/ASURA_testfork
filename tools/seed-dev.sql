-- The development login identities the patched client presents.
--
-- The client does not ask a web service who it is: the rig's portal DLL hands it a fixed pair of
-- UUIDs (PreSearing/portal/portal.c). Login then has to resolve that pair, so these rows have to
-- exist. Applied by `asura-supervisor seed` (Windows and Linux) and by `tools/seed-dev.ps1` (the
-- old Windows-only wrapper).
--
-- Idempotent: re-running reopens the same sessions and leaves the characters alone, which is also
-- how a developer resets a pair that a test or a logout closed.
--
-- map_id 146 is a placeholder: the login batch reports where a character stands, and nothing here
-- moves it. The real spawn map arrives with the world server.

WITH acct AS (
    INSERT INTO account (id, username) VALUES (gen_random_uuid(), 'user1@example.com')
    ON CONFLICT (username) DO UPDATE SET username = EXCLUDED.username
    RETURNING id
), sess AS (
    INSERT INTO session (id, account_id, portal_user, started_at, ended_at)
    SELECT 'd8b9bf5d-90b1-4cbd-9b76-88da7be763b6'::uuid, acct.id,
           'fa520ee2-4419-4eb4-ae49-6e9abe6ef24f'::uuid, now(), NULL
    FROM acct
    ON CONFLICT (id) DO UPDATE SET
        account_id  = EXCLUDED.account_id,
        portal_user = EXCLUDED.portal_user,
        started_at  = now(),
        ended_at    = NULL
    RETURNING account_id
)
INSERT INTO characters (id, account_id, name, map_id)
SELECT gen_random_uuid(), sess.account_id, 'Asura One', 146 FROM sess
WHERE NOT EXISTS (
    SELECT 1 FROM characters c WHERE c.account_id = sess.account_id AND c.name = 'Asura One'
);

WITH acct AS (
    INSERT INTO account (id, username) VALUES (gen_random_uuid(), 'user2@example.com')
    ON CONFLICT (username) DO UPDATE SET username = EXCLUDED.username
    RETURNING id
), sess AS (
    INSERT INTO session (id, account_id, portal_user, started_at, ended_at)
    SELECT '224aee30-d8a5-4f18-ad8e-b5fcbf121c4e'::uuid, acct.id,
           '5fb222bc-9d19-4308-ac3e-3c62685bc6ae'::uuid, now(), NULL
    FROM acct
    ON CONFLICT (id) DO UPDATE SET
        account_id  = EXCLUDED.account_id,
        portal_user = EXCLUDED.portal_user,
        started_at  = now(),
        ended_at    = NULL
    RETURNING account_id
)
INSERT INTO characters (id, account_id, name, map_id)
SELECT gen_random_uuid(), sess.account_id, 'Asura Two', 146 FROM sess
WHERE NOT EXISTS (
    SELECT 1 FROM characters c WHERE c.account_id = sess.account_id AND c.name = 'Asura Two'
);

WITH acct AS (
    INSERT INTO account (id, username) VALUES (gen_random_uuid(), 'user3@example.com')
    ON CONFLICT (username) DO UPDATE SET username = EXCLUDED.username
    RETURNING id
), sess AS (
    INSERT INTO session (id, account_id, portal_user, started_at, ended_at)
    SELECT 'ccdb4b8d-1c88-48c0-bda0-0a5e8d87f4da'::uuid, acct.id,
           '9957c79c-b93c-4b60-9d84-9f38a1f62be3'::uuid, now(), NULL
    FROM acct
    ON CONFLICT (id) DO UPDATE SET
        account_id  = EXCLUDED.account_id,
        portal_user = EXCLUDED.portal_user,
        started_at  = now(),
        ended_at    = NULL
    RETURNING account_id
)
INSERT INTO characters (id, account_id, name, map_id)
SELECT gen_random_uuid(), sess.account_id, 'Asura Three', 146 FROM sess
WHERE NOT EXISTS (
    SELECT 1 FROM characters c WHERE c.account_id = sess.account_id AND c.name = 'Asura Three'
);
