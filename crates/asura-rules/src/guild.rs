//! Guilds: whether a guild may be created, a character invited, a guild left or its announcement set.
//!
//! Pure decisions over what the caller has already read. The world (`asura_world::guild`) asks, logs a
//! refusal, and answers it on the wire; nothing here sends a message or knows a result code.
//!
//! What a live server sends on each of these is in `docs/41-guild-system-research.md`. Which reason the
//! live server gives for a refusal is **not** captured: four refused invitations all came back with the
//! same code (doc 41 section 3.2), so the reasons below are this server's, for its log.
//!
//! Only two member types are measured: the leader (the founder) and an invited character that has not
//! accepted. Officer and member types are not, so every permission here is the leader's.

/// Why a guild request is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The character is already in a guild, so it may not create one.
    InGuild,
    /// The character is in no guild, so there is nothing to invite to, leave or announce in.
    NotInGuild,
    /// Only the leader invites and sets the announcement (the only permitted type that is measured).
    NotLeader,
    /// A guild name or tag must have at least one unit.
    Empty,
    /// A name, tag or announcement that fills its field. Refused rather than cut: a string that fills
    /// a field exactly is a hangup in the title family (`asura_protocol::game::title`).
    TooLong,
    /// No character has that name.
    UnknownCharacter,
    /// The character named is not online in this world.
    Offline,
    /// The character named is in a guild already.
    TargetInGuild,
    /// The character named has an open invitation from this guild already.
    AlreadyInvited,
    /// A character does not invite itself.
    InvitesSelf,
    /// A leader leaves only when no other member is left; it does not leave members leaderless.
    LeaderNotAlone,
}

/// May a field of `units` units be stored in a field that holds `capacity`? Empty is refused only where
/// the caller says so: an announcement may be cleared.
pub fn fits(units: usize, capacity: usize, may_be_empty: bool) -> Result<(), Refused> {
    if units == 0 && !may_be_empty {
        return Err(Refused::Empty);
    }
    if units >= capacity {
        return Err(Refused::TooLong);
    }
    Ok(())
}

/// What a creation is decided on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Creation {
    /// Whether the founder is in a guild now.
    pub in_guild: bool,
    /// The name's length in units, and the field's capacity.
    pub name_units: usize,
    /// See [`Creation::name_units`].
    pub name_capacity: usize,
    /// The tag's length in units, and the field's capacity.
    pub tag_units: usize,
    /// See [`Creation::tag_units`].
    pub tag_capacity: usize,
}

/// May this guild be created? A name already taken is the database's to refuse: it is the only place
/// that sees every guild.
pub fn may_create(creation: Creation) -> Result<(), Refused> {
    if creation.in_guild {
        return Err(Refused::InGuild);
    }
    fits(creation.name_units, creation.name_capacity, false)?;
    fits(creation.tag_units, creation.tag_capacity, false)?;
    Ok(())
}

/// What the invited character is, as the caller found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// No character has the name.
    Unknown,
    /// A character with the name.
    Found {
        /// It is the inviter.
        is_self: bool,
        /// It is connected to this world.
        online: bool,
        /// It is in a guild.
        in_guild: bool,
        /// The inviter's guild has invited it already.
        already_invited: bool,
    },
}

/// May this invitation be made? `inviter_leads` is `None` when the inviter is in no guild.
pub fn may_invite(inviter_leads: Option<bool>, target: Target) -> Result<(), Refused> {
    match inviter_leads {
        None => return Err(Refused::NotInGuild),
        Some(false) => return Err(Refused::NotLeader),
        Some(true) => {}
    }
    match target {
        Target::Unknown => Err(Refused::UnknownCharacter),
        Target::Found { is_self: true, .. } => Err(Refused::InvitesSelf),
        Target::Found { online: false, .. } => Err(Refused::Offline),
        Target::Found { in_guild: true, .. } => Err(Refused::TargetInGuild),
        Target::Found {
            already_invited: true,
            ..
        } => Err(Refused::AlreadyInvited),
        Target::Found { .. } => Ok(()),
    }
}

/// What a departure does to the guild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Departure {
    /// The member goes; the guild stays.
    Leaves,
    /// The leader was the last member: the guild goes with it, open invitations included.
    Dissolves,
}

/// May this character leave its guild, and what happens to the guild? `leads` is `None` when it is in no
/// guild; `others` counts the other members (open invitations are not members).
pub fn may_leave(leads: Option<bool>, others: usize) -> Result<Departure, Refused> {
    match leads {
        None => Err(Refused::NotInGuild),
        Some(false) => Ok(Departure::Leaves),
        Some(true) if others == 0 => Ok(Departure::Dissolves),
        Some(true) => Err(Refused::LeaderNotAlone),
    }
}

/// May this character set its guild's announcement to a text of `units` units?
pub fn may_announce(leads: Option<bool>, units: usize, capacity: usize) -> Result<(), Refused> {
    match leads {
        None => return Err(Refused::NotInGuild),
        Some(false) => return Err(Refused::NotLeader),
        Some(true) => {}
    }
    fits(units, capacity, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUND: Target = Target::Found {
        is_self: false,
        online: true,
        in_guild: false,
        already_invited: false,
    };

    #[test]
    fn a_creation_needs_no_guild_and_a_name_and_tag_that_fit() {
        let ok = Creation {
            in_guild: false,
            name_units: 10,
            name_capacity: 32,
            tag_units: 4,
            tag_capacity: 6,
        };
        assert_eq!(may_create(ok), Ok(()));
        assert_eq!(
            may_create(Creation {
                in_guild: true,
                ..ok
            }),
            Err(Refused::InGuild)
        );
        assert_eq!(
            may_create(Creation { tag_units: 0, ..ok }),
            Err(Refused::Empty)
        );
        assert_eq!(
            may_create(Creation {
                name_units: 32,
                ..ok
            }),
            Err(Refused::TooLong)
        );
    }

    #[test]
    fn an_invitation_is_a_leaders_to_an_online_unguilded_other() {
        assert_eq!(may_invite(Some(true), FOUND), Ok(()));
        assert_eq!(may_invite(None, FOUND), Err(Refused::NotInGuild));
        assert_eq!(may_invite(Some(false), FOUND), Err(Refused::NotLeader));
        assert_eq!(
            may_invite(Some(true), Target::Unknown),
            Err(Refused::UnknownCharacter)
        );
        let with = |is_self, online, in_guild, already_invited| Target::Found {
            is_self,
            online,
            in_guild,
            already_invited,
        };
        assert_eq!(
            may_invite(Some(true), with(true, true, true, false)),
            Err(Refused::InvitesSelf)
        );
        assert_eq!(
            may_invite(Some(true), with(false, false, false, false)),
            Err(Refused::Offline)
        );
        assert_eq!(
            may_invite(Some(true), with(false, true, true, false)),
            Err(Refused::TargetInGuild)
        );
        assert_eq!(
            may_invite(Some(true), with(false, true, false, true)),
            Err(Refused::AlreadyInvited)
        );
    }

    #[test]
    fn a_leader_leaves_only_alone_and_takes_the_guild_with_it() {
        assert_eq!(may_leave(None, 0), Err(Refused::NotInGuild));
        assert_eq!(may_leave(Some(false), 3), Ok(Departure::Leaves));
        assert_eq!(may_leave(Some(true), 0), Ok(Departure::Dissolves));
        assert_eq!(may_leave(Some(true), 1), Err(Refused::LeaderNotAlone));
    }

    #[test]
    fn an_announcement_is_a_leaders_and_may_be_empty() {
        assert_eq!(may_announce(Some(true), 0, 256), Ok(()));
        assert_eq!(may_announce(Some(true), 256, 256), Err(Refused::TooLong));
        assert_eq!(may_announce(Some(false), 5, 256), Err(Refused::NotLeader));
        assert_eq!(may_announce(None, 5, 256), Err(Refused::NotInGuild));
    }
}
