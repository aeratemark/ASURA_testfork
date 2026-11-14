-- Whether a pending transfer is a login handoff or a world travel (docs/56-character-on-travel.md).
--
-- WHY. The auth service writes a transfer for the character a client selected, then the client may
-- present a different character in its game-version message; at login that presented character is the
-- choice the client just made, so it wins when it is on the same account. A travel is not a choice: the
-- world writes the transfer for the character it is moving, and on 2026-11-13 a doorway from Foibles
-- Fair to Wizard's Folly switched a Warrior to the account's Monk because the client's version message
-- still named the login selection. The column keeps the two cases apart.
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21). Idempotent, like every migration.

ALTER TABLE t_transfer ADD COLUMN IF NOT EXISTS is_login boolean NOT NULL DEFAULT false;

INSERT INTO schema_version (version) VALUES (37) ON CONFLICT (version) DO NOTHING;
