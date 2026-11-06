//! Parties: whether an invitation, an answer to one, or a departure may happen; who leads a merge; and
//! what a travel does to a party.
//!
//! Pure decisions over what the caller already knows about the two parties involved. The world's
//! party table (`asura_world::party`) asks, and logs the refusal; nothing here sends a message. The
//! travel rules are the owner's notes of 2026-11-06 (`docs/45-party-travel.md`).
//!
//! What a live server does on each of these is measured in `docs/44-party-invites.md`. What it does
//! when a request is refused is **not** captured, so a refused request is answered with nothing.

/// Why a party request is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Only a party's leader invites, cancels an invitation, or answers one for the party.
    NotLeader,
    /// The two players are already in the same party.
    SameParty,
    /// This party has already invited that one.
    AlreadyInvited,
    /// There is no such invitation to answer or cancel.
    NotInvited,
    /// A party of one has nobody to leave.
    Alone,
    /// The player to kick is not in the kicker's party.
    NotInParty,
    /// A leader leaves its party; it does not kick itself out of it.
    KickSelf,
    /// The party would hold more players than this map allows.
    Full,
    /// No party size is configured for this map, so no party grows here.
    NoSizeForMap,
    /// Parties are formed and left in outposts. An explorable is one party's own copy of the map
    /// (`docs/22-world-model.md`, "Explorables"), so its party does not change there.
    NotInOutpost,
}

/// May a party be changed here at all? Asked before every invitation, answer and leave.
pub fn may_change_party(in_outpost: bool) -> Result<(), Refused> {
    if !in_outpost {
        return Err(Refused::NotInOutpost);
    }
    Ok(())
}

/// What an invitation is decided on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invitation {
    /// Whether the player inviting leads its own party.
    pub inviter_leads: bool,
    /// Whether the invited player is in the inviter's party already.
    pub same_party: bool,
    /// Whether the inviter's party has an open invitation of the invited one.
    pub already_invited: bool,
}

/// May this invitation be made?
pub fn may_invite(invitation: Invitation) -> Result<(), Refused> {
    if !invitation.inviter_leads {
        return Err(Refused::NotLeader);
    }
    if invitation.same_party {
        return Err(Refused::SameParty);
    }
    if invitation.already_invited {
        return Err(Refused::AlreadyInvited);
    }
    Ok(())
}

/// May a party grow to `size_after` players on a map that allows `max_size`? `None` is a map with no
/// configured size: nothing grows there.
pub fn may_fill(size_after: usize, max_size: Option<u32>) -> Result<(), Refused> {
    let Some(max_size) = max_size else {
        return Err(Refused::NoSizeForMap);
    };
    if size_after > max_size as usize {
        return Err(Refused::Full);
    }
    Ok(())
}

/// May this player accept, decline or cancel an invitation? The same two questions for all three:
/// does it lead its party, and is the invitation open?
pub fn may_answer(leads: bool, invitation_open: bool) -> Result<(), Refused> {
    if !leads {
        return Err(Refused::NotLeader);
    }
    if !invitation_open {
        return Err(Refused::NotInvited);
    }
    Ok(())
}

/// May this leader kick that player? `target_in_party` is whether the target is a member of the kicker's
/// party, `target_is_self` whether it is the kicker.
pub fn may_kick(leads: bool, target_in_party: bool, target_is_self: bool) -> Result<(), Refused> {
    if !leads {
        return Err(Refused::NotLeader);
    }
    if target_is_self {
        return Err(Refused::KickSelf);
    }
    if !target_in_party {
        return Err(Refused::NotInParty);
    }
    Ok(())
}

/// May a member of a party of `members` players leave it?
pub fn may_leave(members: usize) -> Result<(), Refused> {
    if members <= 1 {
        return Err(Refused::Alone);
    }
    Ok(())
}

// ---- merges -------------------------------------------------------------------------------------

/// Whose leader leads the party two parties merge into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeLeader {
    /// The leader of the party that sent the invitation.
    Inviting,
    /// The leader of the party that accepted it.
    Invited,
}

/// Who leads after a merge. The owner's rule, 2026-11-06: "If two solo players or two parties of equal
/// size merge, the player who initiated the invitation becomes leader. If parties of different sizes
/// merge, the leader of the larger party takes leadership." Only a leader invites, so the player who
/// initiated it is the inviting party's leader.
pub fn merge_leader(inviting_size: usize, invited_size: usize) -> MergeLeader {
    if invited_size > inviting_size {
        MergeLeader::Invited
    } else {
        MergeLeader::Inviting
    }
}

// ---- travel -------------------------------------------------------------------------------------

/// What a world-map travel request (`GAME_CMSG_PARTY_TRAVEL`) does to the party of the player that sent
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelRequest {
    /// A party of one: it travels at once, as it always has.
    Alone,
    /// A leader in an outpost: the whole party follows after the countdown.
    PartyAfterCountdown,
    /// Anyone else - a member in an outpost, or anyone in an explorable: it leaves its party, which
    /// stays where it is, and travels alone. The client asks the player first; the server sees only the
    /// request.
    LeavesParty,
}

/// Decide what a world-map travel request does (the owner's notes, 2026-11-06, sections 3, 5 and 6).
pub fn travel_request(in_outpost: bool, leads: bool, party_size: usize) -> TravelRequest {
    if party_size <= 1 {
        TravelRequest::Alone
    } else if in_outpost && leads {
        TravelRequest::PartyAfterCountdown
    } else {
        TravelRequest::LeavesParty
    }
}

/// Whether a member follows its leader's world-map travel to an outpost: only when it has that outpost
/// unlocked itself (owner, 2026-11-06: "the outpost has to be already unlocked for all the members of
/// the party"). One that has not stays behind in a party of its own; how a live server offers the leader
/// that choice is not captured. A doorway is not asked this: crossing one unlocks its destination.
pub fn follows_to_outpost(destination_unlocked: bool) -> bool {
    destination_unlocked
}

/// How many of a moving party's players travel, counted leader first, on a destination that allows
/// `max_size` (ledger R26: a smaller cap truncates the party and the rest stay behind in parties of
/// one). `None` is a destination with no configured size: no party grows there, so only the player that
/// started the move goes, and the caller logs why.
pub fn travelling_count(present: usize, max_size: Option<u32>) -> Result<usize, Refused> {
    let Some(max_size) = max_size else {
        return Err(Refused::NoSizeForMap);
    };
    Ok(present.min(max_size as usize))
}

/// Whether a player whose connection ended keeps its place in its party: only after a **network**
/// disconnect (a clean logout is a leave), only in an explorable (outposts have no hold), and only when
/// the party has someone left to hold the place for (`docs/22-world-model.md`, "Explorable teardown"
/// and "Member disconnect"; ledger R11).
pub fn holds_on_disconnect(network_drop: bool, in_explorable: bool, others_present: usize) -> bool {
    network_drop && in_explorable && others_present > 0
}

/// A character's place in a party that its connection lost, as the caller read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    /// The map the party is in.
    pub map_id: i32,
    /// Whether the hold has not expired yet.
    pub live: bool,
}

/// Where a character that logs in again enters: the map its party holds its place in, when the hold
/// is live, and otherwise `None` - the ordinary placement rule answers.
pub fn rejoin_held(held: Option<Held>) -> Option<i32> {
    held.filter(|held| held.live).map(|held| held.map_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invitation_needs_a_leader_another_party_and_no_open_one() {
        let ok = Invitation {
            inviter_leads: true,
            same_party: false,
            already_invited: false,
        };
        assert_eq!(may_invite(ok), Ok(()));
        assert_eq!(
            may_invite(Invitation {
                inviter_leads: false,
                ..ok
            }),
            Err(Refused::NotLeader)
        );
        assert_eq!(
            may_invite(Invitation {
                same_party: true,
                ..ok
            }),
            Err(Refused::SameParty)
        );
        assert_eq!(
            may_invite(Invitation {
                already_invited: true,
                ..ok
            }),
            Err(Refused::AlreadyInvited)
        );
    }

    #[test]
    fn an_answer_needs_a_leader_and_an_open_invitation_and_a_leave_needs_company() {
        assert_eq!(may_answer(true, true), Ok(()));
        assert_eq!(may_answer(false, true), Err(Refused::NotLeader));
        assert_eq!(may_answer(true, false), Err(Refused::NotInvited));
        assert_eq!(may_leave(2), Ok(()));
        assert_eq!(may_leave(1), Err(Refused::Alone));
    }

    #[test]
    fn a_kick_is_a_leaders_and_names_another_member() {
        assert_eq!(may_kick(true, true, false), Ok(()));
        assert_eq!(may_kick(false, true, false), Err(Refused::NotLeader));
        assert_eq!(may_kick(true, true, true), Err(Refused::KickSelf));
        assert_eq!(may_kick(true, false, false), Err(Refused::NotInParty));
    }

    #[test]
    fn a_party_grows_only_to_its_maps_size() {
        assert_eq!(may_fill(2, Some(2)), Ok(()));
        assert_eq!(may_fill(3, Some(2)), Err(Refused::Full));
        assert_eq!(may_fill(2, None), Err(Refused::NoSizeForMap));
    }

    #[test]
    fn a_party_changes_only_in_an_outpost() {
        assert_eq!(may_change_party(true), Ok(()));
        assert_eq!(may_change_party(false), Err(Refused::NotInOutpost));
    }

    #[test]
    fn the_inviter_leads_a_merge_unless_the_invited_party_is_larger() {
        assert_eq!(merge_leader(1, 1), MergeLeader::Inviting);
        assert_eq!(merge_leader(2, 2), MergeLeader::Inviting);
        assert_eq!(merge_leader(3, 1), MergeLeader::Inviting);
        assert_eq!(merge_leader(1, 3), MergeLeader::Invited);
    }

    #[test]
    fn a_leader_in_an_outpost_moves_the_party_and_everyone_else_leaves_it() {
        assert_eq!(travel_request(true, true, 1), TravelRequest::Alone);
        assert_eq!(travel_request(false, true, 1), TravelRequest::Alone);
        assert_eq!(
            travel_request(true, true, 2),
            TravelRequest::PartyAfterCountdown
        );
        assert_eq!(travel_request(true, false, 2), TravelRequest::LeavesParty);
        assert_eq!(travel_request(false, true, 2), TravelRequest::LeavesParty);
    }

    #[test]
    fn only_a_member_with_the_outpost_unlocked_follows_its_leader() {
        assert!(follows_to_outpost(true));
        assert!(!follows_to_outpost(false));
    }

    #[test]
    fn a_smaller_destination_truncates_the_party_leader_first() {
        assert_eq!(travelling_count(2, Some(2)), Ok(2));
        assert_eq!(travelling_count(3, Some(2)), Ok(2));
        assert_eq!(travelling_count(1, Some(8)), Ok(1));
        assert_eq!(travelling_count(2, None), Err(Refused::NoSizeForMap));
    }

    #[test]
    fn only_a_network_drop_in_an_explorable_with_company_is_held() {
        assert!(holds_on_disconnect(true, true, 1));
        assert!(!holds_on_disconnect(false, true, 1), "a logout is a leave");
        assert!(!holds_on_disconnect(true, false, 1), "outposts hold nobody");
        assert!(!holds_on_disconnect(true, true, 0), "nobody to hold it for");
    }

    #[test]
    fn a_live_hold_places_the_character_back_in_its_partys_map() {
        let held = Held {
            map_id: 146,
            live: true,
        };
        assert_eq!(rejoin_held(Some(held)), Some(146));
        assert_eq!(
            rejoin_held(Some(Held {
                live: false,
                ..held
            })),
            None
        );
        assert_eq!(rejoin_held(None), None);
    }
}
