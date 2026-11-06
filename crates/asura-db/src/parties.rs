//! Parties that travel: the ledger a party crosses from one instance into another with (migration 0030).
//!
//! Rows only. What a travel does to a party is `asura_rules::party`; the party itself, while it stays in
//! one instance, is the world's (`asura_world::party`). A world writes here when a party moves
//! ([`depart`]) and when a member arrives ([`arrived`]) or drops ([`hold`]); a world reads it when a
//! member's connection arrives ([`membership`]); the auth service reads it when a member logs in again
//! and forgets it when it does not rejoin ([`forget`]).

use crate::{PgPool, Uuid};

/// A member told to go and not arrived yet.
pub const TRAVELLING: i16 = 0;
/// A member standing in the party's map.
pub const IN_MAP: i16 = 1;
/// A member whose connection dropped and whose place the party keeps until its hold ends.
pub const HELD: i16 = 2;

/// One row of [`membership`]'s join: the party's id, leader, world and map, then one member's character,
/// seat, state and whether its hold is live.
type MemberRow = (Uuid, Uuid, i32, i32, Uuid, i16, i16, bool);

/// One character's place in a travelling party.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    /// The character.
    pub character_id: Uuid,
    /// Its place in the party's order, the leader at 0.
    pub seat: i16,
    /// [`TRAVELLING`], [`IN_MAP`] or [`HELD`].
    pub state: i16,
    /// For a held member, whether its hold has not ended yet. False for every other state.
    pub hold_live: bool,
}

/// A character's party, as the ledger has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The party's id: one per move.
    pub party_id: Uuid,
    /// The character that leads it.
    pub leader: Uuid,
    /// The world it went to.
    pub world_id: i32,
    /// The map it went to.
    pub map_id: i32,
    /// Every member, in seat order.
    pub members: Vec<Member>,
}

impl Membership {
    /// The row of one character.
    pub fn member(&self, character_id: Uuid) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| member.character_id == character_id)
    }
}

/// Write a party that is about to move: `members` in the party's order, the leader first, all
/// travelling to `map_id` on `world_id`. Returns the new party's id.
///
/// Whatever party any of them was in before is deleted with every place in it, held places included:
/// a party that changes areas leaves a held member behind. One transaction, so a member is never in
/// two parties.
pub async fn depart(
    pool: &PgPool,
    leader: Uuid,
    world_id: i32,
    map_id: i32,
    members: &[Uuid],
) -> sqlx::Result<Uuid> {
    let party_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM t_parties
          WHERE id IN (SELECT party_id FROM t_party_members WHERE character_id = ANY($1))",
    )
    .bind(members)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO t_parties (id, leader, world_id, map_id) VALUES ($1, $2, $3, $4)")
        .bind(party_id)
        .bind(leader)
        .bind(world_id)
        .bind(map_id)
        .execute(&mut *tx)
        .await?;
    for (seat, character_id) in members.iter().enumerate() {
        sqlx::query(
            "INSERT INTO t_party_members (character_id, party_id, seat, state)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(character_id)
        .bind(party_id)
        .bind(i16::try_from(seat).unwrap_or(i16::MAX))
        .bind(TRAVELLING)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(party_id)
}

/// The party a character is in, with every member in seat order, or `None` when the ledger has none.
pub async fn membership(pool: &PgPool, character_id: Uuid) -> sqlx::Result<Option<Membership>> {
    let rows: Vec<MemberRow> = sqlx::query_as(
        "SELECT p.id, p.leader, p.world_id, p.map_id,
                m.character_id, m.seat, m.state,
                coalesce(m.state = 2 AND m.held_until > now(), false)
           FROM t_party_members me
           JOIN t_parties p ON p.id = me.party_id
           JOIN t_party_members m ON m.party_id = p.id
          WHERE me.character_id = $1
          ORDER BY m.seat",
    )
    .bind(character_id)
    .fetch_all(pool)
    .await?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let (party_id, leader, world_id, map_id) = (first.0, first.1, first.2, first.3);
    Ok(Some(Membership {
        party_id,
        leader,
        world_id,
        map_id,
        members: rows
            .into_iter()
            .map(
                |(_, _, _, _, character_id, seat, state, hold_live)| Member {
                    character_id,
                    seat,
                    state,
                    hold_live,
                },
            )
            .collect(),
    }))
}

/// A member stands in its party's map. Returns whether it had a place to mark.
pub async fn arrived(pool: &PgPool, character_id: Uuid) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE t_party_members SET state = $2, held_until = NULL WHERE character_id = $1",
    )
    .bind(character_id)
    .bind(IN_MAP)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// A member's connection dropped: its party keeps its place for `seconds`. Returns whether it had a
/// place to hold.
pub async fn hold(pool: &PgPool, character_id: Uuid, seconds: f64) -> sqlx::Result<bool> {
    let done = sqlx::query(
        "UPDATE t_party_members
            SET state = $2, held_until = now() + make_interval(secs => $3::float8)
          WHERE character_id = $1",
    )
    .bind(character_id)
    .bind(HELD)
    .bind(seconds)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// A character is in no travelling party any more: it left, was left behind, or logged in afresh. A
/// party left with nobody is deleted with it. Returns whether it had a place.
pub async fn forget(pool: &PgPool, character_id: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let party: Option<Uuid> = sqlx::query_scalar(
        "DELETE FROM t_party_members WHERE character_id = $1 RETURNING party_id",
    )
    .bind(character_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(party_id) = party {
        sqlx::query(
            "DELETE FROM t_parties p
              WHERE p.id = $1
                AND NOT EXISTS (SELECT 1 FROM t_party_members m WHERE m.party_id = p.id)",
        )
        .bind(party_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(party.is_some())
}
