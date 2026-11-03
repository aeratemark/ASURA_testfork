-- The service registry (S2): every service kind announces itself the same way a world server
-- already does (docs/16-implementation-plan.md, S2; docs/02-architecture.md).
--
-- A row says: this service exists, this is where it is (address, port), this is its build, whether
-- it is alive (`heartbeat_at`) and what state it is in. The supervisor writes a row for every
-- service it starts; consumers read them instead of hard-coding addresses in scripts.
--
-- Worlds keep their own `world` table as the load-bearing registry (placement and the front door
-- read it); this table is the general one. `kind` has no CHECK list on purpose: the set of service
-- kinds grows (a content server, a gateway), and adding a kind must not need a migration.
--
-- It is a table consumers read, not a network hop between services. See the migration comment above
-- in `crates/asura-db/src/lib.rs` for why that distinction matters.

CREATE TABLE IF NOT EXISTS service (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind         text NOT NULL,
    name         text NOT NULL,
    address      text NOT NULL,
    port         int  NOT NULL,
    build        text NOT NULL,
    state        text NOT NULL DEFAULT 'ready',
    started_at   timestamptz NOT NULL DEFAULT now(),
    heartbeat_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (kind, name)
);

CREATE INDEX IF NOT EXISTS service_by_kind ON service (kind);

INSERT INTO schema_version (version) VALUES (10) ON CONFLICT (version) DO NOTHING;
