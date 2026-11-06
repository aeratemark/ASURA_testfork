//! Guilds in the world: the guild block of a load, a guilded peer's introduction, and the four requests
//! this server answers - create (`0x8036`), invite (`0x80B5`), leave (`0x80BB`) and the announcement
//! (`0x80BE`).
//!
//! What a live server sends for each is in `docs/41-guild-system-research.md`, from
//! `recordings/client/feature/guild/guild created, people invited.jsonl` (creation, five invitations, two
//! announcements; called "the raw file" below) and `.../guild/entering, leaving guildhall and leaving
//! guild.jsonl` (a leave; "the leave file"). Each batch names the seq it copies, and what no capture shows
//! is marked **not captured** where it is sent.
//!
//! **Gold is not charged** (owner, 2026-11-06: free for now, gold a placeholder). The live creation and
//! every live invitation debit 100 gold (`0x014F`) and a refused invitation credits it back (`0x0140`);
//! none of those messages is sent here, and the two prices in `config/Server/guilds.json` are read and
//! logged only.
//!
//! Creation opens with `/guild create` (owner, 2026-11-06), which draws the Guild Registration window
//! without a registrar NPC ([`handle_guild_command`]).
//!
//! Not built: a Guild Registrar NPC and the vendor system behind it (`docs/42-vendor-system-research.md`);
//! the invited character's side of an invitation and its accept
//! (`0x0123` and the request that answers it are not captured); kicks, promotions, guild chat and guild
//! halls; the roster and history a load sends when the guild has them (`pre-searing-load-3.jsonl` seq
//! 720-741 carries `0x012E`, twenty `0x011F` and a `0x012F` after `0x0118`, and what triggers that block is
//! not established).

use std::collections::HashMap;
use std::sync::Mutex;

use asura_db::guilds as db;
use asura_protocol::game::guild as wire;
use asura_rules::guild as rules;

use crate::*;

/// This character's guild and its own member row, as read when the connection was built and as changed by
/// this connection since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The guild.
    pub guild: db::Guild,
    /// The character's row in it.
    pub member: db::Member,
}

impl Membership {
    /// Whether the character leads the guild: the only permission that is measured.
    pub fn leads(&self) -> bool {
        self.member.member_type == wire::GUILD_MEMBER_LEADER
    }
}

/// What another client in the instance is told about a player's guild: the guild, and the local id this
/// instance's messages name it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    /// The id `0x0121` and agent property 30 carry.
    pub local_id: u16,
    /// The guild.
    pub guild: db::Guild,
}

/// The local ids of the guilds an instance has named. The live ids are small server-chosen numbers that
/// change from load to load (180 at creation, 22 on a later load of the same character); the client needs
/// only that two guilds in view never share one, so this hands out the lowest free id and keeps it for the
/// instance's life.
#[derive(Default)]
pub struct LocalIds {
    ids: Mutex<HashMap<Uuid, u16>>,
}

impl LocalIds {
    /// The local id of a guild in this instance, allocated on first use.
    pub fn of(&self, guild: Uuid) -> u16 {
        let mut ids = self.ids.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(id) = ids.get(&guild) {
            return *id;
        }
        let id = (1..=u16::MAX)
            .find(|candidate| !ids.values().any(|taken| taken == candidate))
            .unwrap_or(u16::MAX);
        ids.insert(guild, id);
        id
    }
}

/// The guild settings, or `None` with the reason logged.
fn settings() -> Option<&'static asura_config::Guilds> {
    match asura_config::guilds() {
        Ok(settings) => Some(settings),
        Err(err) => {
            eprintln!("world: guild: {err}; guild requests are refused");
            None
        }
    }
}

/// Read a character's guild when its connection is built. A database error is logged and read as no guild:
/// a load must not fail because a side table could not be read.
pub async fn load(pool: &PgPool, character: Uuid) -> Option<Membership> {
    match db::membership(pool, character).await {
        Ok(found) => found.map(|(guild, member)| Membership { guild, member }),
        Err(err) => {
            eprintln!("world: guild: could not read the guild of {character}: {err}");
            None
        }
    }
}

/// The badge a player is introduced with, allocating its guild's local id in its instance.
pub fn badge(instance: &Instance, membership: Option<&Membership>) -> Option<Badge> {
    membership.map(|membership| Badge {
        local_id: instance.guild_ids.of(membership.guild.id),
        guild: membership.guild.clone(),
    })
}

// ---- the frames ---------------------------------------------------------------------------------------

fn key(guild: &db::Guild) -> [u8; 16] {
    *guild.id.as_bytes()
}

/// `0x0121` for one guild.
fn general_info(local_id: u16, guild: &db::Guild) -> Frame {
    let cape = guild.cape;
    wire::guild_general_info(&wire::GuildGeneral {
        local_id,
        key: key(guild),
        name: &guild.name,
        tag: &guild.tag,
        feature: guild.feature,
        territory: guild.territory,
        cape: wire::Cape {
            background_color: cape.background_color,
            detail_color: cape.detail_color,
            emblem_color: cape.emblem_color,
            shape: cape.shape,
            detail: cape.detail,
            emblem: cape.emblem,
            trim: cape.trim,
        },
        allegiance: guild.allegiance,
        faction: guild.faction,
        qualifier: guild.qualifier,
        rating: guild.rating,
        rank: guild.rank,
    })
}

/// Agent property 30: the agent's guild, by local id; 0 for none.
fn guild_property(agent_id: u32, local_id: u16) -> Frame {
    game::agent_property_update_int(wire::AGENT_PROPERTY_GUILD, agent_id, u32::from(local_id))
}

/// A guilded player as another client is told it, right after its player create: `0x0121`, then property
/// 30 on its agent (`pre-searing-load-3.jsonl` seq 299-301; `titles-live-20261106-025940.jsonl` seq
/// 1147-1148).
pub fn badge_frames(badge: &Badge, agent_id: u32) -> [Frame; 2] {
    [
        general_info(badge.local_id, &badge.guild),
        guild_property(agent_id, badge.local_id),
    ]
}

/// The guild block of a character's own load, just before its party block:
///
/// - in a guild: `0x0121`, property 30 on its agent, `0x0118` with its role, then `0x011C`
///   (`titles-live-20261106-034507.jsonl` seq 838-841, `titles-live-20261106-025940.jsonl` seq
///   1095-1098);
/// - in none: `0x011C` alone (`multiplay/party.jsonl` seq 2661).
pub fn load_frames(
    badge: Option<&Badge>,
    membership: Option<&Membership>,
    agent_id: u32,
) -> Vec<Frame> {
    let mut frames = Vec::new();
    if let (Some(badge), Some(membership)) = (badge, membership) {
        frames.extend(badge_frames(badge, agent_id));
        frames.push(wire::guild_player_role(
            badge.local_id,
            key(&badge.guild),
            membership.member.member_type,
        ));
    }
    frames.push(wire::guild_stream_end());
    frames
}

/// Put the guild block in front of the player block's party (`PARTY_CREATE`), where a live load carries it.
/// `false`, and the block untouched, when there is no party message to anchor on.
pub fn splice_into_player_block(block: &mut Vec<Frame>, guild: Vec<Frame>) -> bool {
    let Some(at) = block
        .iter()
        .position(|frame| frame.header() == game::PARTY_CREATE)
    else {
        return false;
    };
    block.splice(at..at, guild);
    true
}

// ---- the requests -------------------------------------------------------------------------------------

fn incomplete(needed: usize) -> GameError {
    GameError::Framing(FrameError::Incomplete { needed })
}

/// A price the client quoted or the file states, logged and not charged.
fn note_price(what: &str, configured: u32, quoted: Option<u32>) {
    if configured != 0 || quoted.is_some_and(|quoted| quoted != 0) {
        eprintln!(
            "world: guild: {what} is free; the configured price {configured} is a placeholder{}",
            quoted
                .map(|quoted| format!(" and the client quoted {quoted}"))
                .unwrap_or_default()
        );
    }
}

/// Answer a refused request with `0x0132 {request_id, 14}` and log why. The live answer to the four refused
/// invitations is that message after the gold debit and credit (raw file seq 719-722), which are not sent.
async fn refuse(
    stream: &mut TcpStream,
    session: &mut Session,
    name: &str,
    request_id: u32,
    what: &str,
    why: impl std::fmt::Debug,
    trace: bool,
) -> Result<bool, GameError> {
    eprintln!("world: guild: {name:?} {what}: refused ({why:?})");
    send(
        stream,
        session,
        vec![wire::guild_request_result(
            request_id,
            wire::GUILD_RESULT_REFUSED,
        )],
        trace,
    )
    .await?;
    Ok(true)
}

/// The next value of a guild's `0x0133` counter, stored. A guild that is gone keeps the value it had plus
/// one step, unstored.
async fn next_revision(world: &World, guild: &db::Guild, step: u32) -> Result<u32, GameError> {
    Ok(db::advance_revision(&world.pool, guild.id, step)
        .await
        .map_err(GameError::Database)?
        .unwrap_or(guild.revision.wrapping_add(step)))
}

/// `/guild <verb>`: the owner's chat stand-in for the Guild Registrar (2026-11-06). `/guild create` opens
/// the Guild Registration window, owned by the player's own agent because no registrar is spawned; the
/// form it draws sends `0x8036`, answered by [`handle_create`]. A character already in a guild is not shown
/// the window. Everything else prints the usage.
pub async fn handle_guild_command(
    stream: &mut TcpStream,
    session: &mut Session,
    player: &mut Player,
    argument: &str,
    trace: bool,
) -> Result<bool, GameError> {
    match argument.trim() {
        "create" => {
            if let Some(membership) = &player.guild {
                eprintln!(
                    "world: /guild create: {:?} is in {:?} already; no window",
                    player.name, membership.guild.name
                );
                return Ok(true);
            }
            if trace {
                eprintln!(
                    "world: /guild create: the registration window for {:?}",
                    player.name
                );
            }
            send(
                stream,
                session,
                wire::registration_window(player.agent_id),
                trace,
            )
            .await?;
            Ok(true)
        }
        _ => {
            eprintln!("world: /guild: Use: /guild create");
            Ok(true)
        }
    }
}

/// `0x8036`: create a guild with this character as its leader.
///
/// The live batch (raw file seq 313-326): the gold debit `0x014F` (not sent: free), a server chat line
/// `0x005D` + `0x005E` (**not sent**: its coded content is not decoded), `0x00CC {8}`, `0x0131`, `0x0121`,
/// property 30 on the founder's agent, `0x0118 {local, key, 5}`, `0x011F` with the founding line, `0x012F`
/// empty, `0x0127` for the founder, `0x0133 {0x0200}`, `0x011C`. Nothing follows a refusal: the live
/// answer to one is not captured, and creation carries no request id to answer.
///
/// The other players in the instance are sent the founder's badge (**not captured**).
pub async fn handle_create(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let request = wire::read_create_request(frame.body()).ok_or_else(|| incomplete(8))?;
    let Some(settings) = settings() else {
        return Ok(true);
    };
    note_price(
        "creation",
        settings.creation_price.value,
        Some(request.quoted),
    );
    if let Err(why) = rules::may_create(rules::Creation {
        in_guild: player.guild.is_some(),
        name_units: request.name.encode_utf16().count(),
        name_capacity: wire::GUILD_NAME_UNITS,
        tag_units: request.tag.encode_utf16().count(),
        tag_capacity: wire::GUILD_TAG_UNITS,
    }) {
        eprintln!(
            "world: guild: {:?} creates {:?} [{}]: refused ({why:?}); nothing sent",
            player.name, request.name, request.tag
        );
        return Ok(true);
    }
    let start = db::NewGuild {
        feature: settings.new_guild_feature.value,
        territory: settings.new_guild_territory.value,
        allegiance: settings.new_guild_allegiance.value,
        rating: settings.new_guild_rating.value,
        rank: settings.new_guild_rank.value,
        revision: settings.revision_on_create.value,
    };
    let created = db::create(
        &world.pool,
        Uuid::new_v4(),
        &request.name,
        &request.tag,
        &start,
        player.character,
        &player.name,
        wire::GUILD_MEMBER_LEADER,
    )
    .await
    .map_err(GameError::Database)?;
    let Some((guild, member)) = created else {
        eprintln!(
            "world: guild: {:?} creates {:?}: refused (the name is taken, or it is in a guild); nothing sent",
            player.name, request.name
        );
        return Ok(true);
    };
    let membership = Membership { guild, member };
    let badge = badge(&player.instance, Some(&membership)).expect("a membership has a badge");
    let date = wire::guild_date(membership.member.joined_at);

    let mut frames = vec![
        wire::trader_transaction_done(wire::GUILD_REGISTRAR_WINDOW),
        wire::guild_stream_start(),
    ];
    frames.extend(badge_frames(&badge, player.agent_id));
    frames.push(wire::guild_player_role(
        badge.local_id,
        key(&membership.guild),
        membership.member.member_type,
    ));
    frames.push(wire::guild_history_entry(
        date,
        &wire::founded_line(&player.name),
    ));
    frames.push(wire::guild_announcement(
        &membership.guild.announcement,
        &membership.guild.announcement_author,
    ));
    frames.push(wire::guild_player_info(&wire::GuildPlayer {
        invited_name: &membership.member.invited_name,
        current_name: &player.name,
        invited_by: &membership.member.invited_by,
        context: "",
        minutes_since_login: 0,
        join_date: date,
        status: wire::GUILD_STATUS_ONLINE,
        member_type: membership.member.member_type,
    }));
    frames.push(wire::guild_revision(membership.guild.revision));
    frames.push(wire::guild_stream_end());

    if trace {
        eprintln!(
            "world: guild: {:?} founds {:?} [{}], local id {}",
            player.name, membership.guild.name, membership.guild.tag, badge.local_id
        );
    }
    player
        .instance
        .broadcast(player.player_id, &badge_frames(&badge, player.agent_id));
    player.instance.wear_guild(player.player_id, Some(badge));
    player.guild = Some(membership);
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `0x80B5`: invite a character, by name, to this character's guild.
///
/// The live answer to a successful invitation (raw file seq 3344-3352): the gold debit (not sent: free),
/// `0x0132 {request, 0}`, `0x0127` for the invitee - its name, no current name, the inviter, the date, status
/// 0, type 1 - and `0x0133`. A refusal is `0x0132 {request, 14}`.
///
/// The invited character is told nothing (**not captured**: its `0x0123` and the request that accepts are
/// the next capture, doc 41 section 5 item 2). "Online" is this world's own instances: a character in another
/// world process reads as offline.
pub async fn handle_invite(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let request = wire::read_invite_request(frame.body()).ok_or_else(|| incomplete(8))?;
    let Some(settings) = settings() else {
        return refuse(
            stream,
            session,
            &player.name,
            request.request_id,
            "invites",
            "no guild settings",
            trace,
        )
        .await;
    };
    note_price("an invitation", settings.invite_fee.value, None);
    let leads = player.guild.as_ref().map(Membership::leads);
    let found = db::character_named(&world.pool, &request.name)
        .await
        .map_err(GameError::Database)?;
    let target = match &found {
        None => rules::Target::Unknown,
        Some((id, name)) => rules::Target::Found {
            is_self: *id == player.character,
            online: world.instances.is_online(name),
            in_guild: db::is_member(&world.pool, *id)
                .await
                .map_err(GameError::Database)?,
            already_invited: match &player.guild {
                Some(membership) => db::is_invited(&world.pool, membership.guild.id, *id)
                    .await
                    .map_err(GameError::Database)?,
                None => false,
            },
        },
    };
    if let Err(why) = rules::may_invite(leads, target) {
        return refuse(
            stream,
            session,
            &player.name,
            request.request_id,
            &format!("invites {:?}", request.name),
            why,
            trace,
        )
        .await;
    }
    let (Some(membership), Some((target_id, target_name))) = (player.guild.as_ref(), found) else {
        return refuse(
            stream,
            session,
            &player.name,
            request.request_id,
            "invites",
            "no guild or no target",
            trace,
        )
        .await;
    };
    let guild = membership.guild.clone();
    let invited_at = db::invite(&world.pool, guild.id, target_id, &target_name, &player.name)
        .await
        .map_err(GameError::Database)?;
    let Some(invited_at) = invited_at else {
        return refuse(
            stream,
            session,
            &player.name,
            request.request_id,
            "invites",
            rules::Refused::AlreadyInvited,
            trace,
        )
        .await;
    };
    let revision = next_revision(world, &guild, settings.revision_step.value).await?;
    if let Some(membership) = player.guild.as_mut() {
        membership.guild.revision = revision;
    }
    if trace {
        eprintln!(
            "world: guild: {:?} invites {target_name:?} to {:?}",
            player.name, guild.name
        );
    }
    let frames = vec![
        wire::guild_request_result(request.request_id, wire::GUILD_RESULT_OK),
        wire::guild_player_info(&wire::GuildPlayer {
            invited_name: &target_name,
            current_name: "",
            invited_by: &player.name,
            context: "",
            minutes_since_login: 0,
            join_date: wire::guild_date(invited_at),
            status: wire::GUILD_STATUS_OFFLINE,
            member_type: wire::GUILD_MEMBER_INVITED,
        }),
        wire::guild_revision(revision),
    ];
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `0x80BB`: leave the guild.
///
/// The live batch (leave file seq 728-733): `0x0128` with the leaver's roster name, `0x0133`, property 30 on
/// its agent set to 0, `0x0119`, `0x0132 {request, 0}`. A leader leaves only as the last member, and the
/// guild goes with it (this server's rule: no capture shows a leader leaving). The other players in the
/// instance are sent the property set to 0 (**not captured**).
pub async fn handle_leave(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let request_id = wire::read_leave_request(frame.body()).ok_or_else(|| incomplete(6))?;
    let Some(settings) = settings() else {
        return refuse(
            stream,
            session,
            &player.name,
            request_id,
            "leaves",
            "no guild settings",
            trace,
        )
        .await;
    };
    let leads = player.guild.as_ref().map(Membership::leads);
    let others = match &player.guild {
        Some(membership) => db::other_members(&world.pool, membership.guild.id, player.character)
            .await
            .map_err(GameError::Database)?,
        None => 0,
    };
    let departure = match rules::may_leave(leads, others) {
        Ok(departure) => departure,
        Err(why) => {
            return refuse(
                stream,
                session,
                &player.name,
                request_id,
                "leaves",
                why,
                trace,
            )
            .await;
        }
    };
    let Some(membership) = player.guild.take() else {
        return refuse(
            stream,
            session,
            &player.name,
            request_id,
            "leaves",
            "no guild",
            trace,
        )
        .await;
    };
    let step = settings.revision_step.value;
    let revision = match departure {
        rules::Departure::Leaves => {
            db::leave(&world.pool, player.character)
                .await
                .map_err(GameError::Database)?;
            next_revision(world, &membership.guild, step).await?
        }
        rules::Departure::Dissolves => {
            db::dissolve(&world.pool, membership.guild.id)
                .await
                .map_err(GameError::Database)?;
            membership.guild.revision.wrapping_add(step)
        }
    };
    if trace {
        eprintln!(
            "world: guild: {:?} leaves {:?} ({departure:?})",
            player.name, membership.guild.name
        );
    }
    let cleared = guild_property(player.agent_id, 0);
    player.instance.wear_guild(player.player_id, None);
    player
        .instance
        .broadcast(player.player_id, std::slice::from_ref(&cleared));
    let frames = vec![
        wire::guild_player_remove(&membership.member.invited_name),
        wire::guild_revision(revision),
        cleared,
        wire::guild_none(),
        wire::guild_request_result(request_id, wire::GUILD_RESULT_OK),
    ];
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `0x80BE`: set the guild's announcement.
///
/// The live batch (raw file seq 5585-5587, 6080-6082): `0x012F {text, author}`, `0x0133`, `0x0132 {request,
/// 0}`. Sent to the one who set it; the other members online are told nothing (**not captured**).
pub async fn handle_announcement(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let (request_id, text) =
        wire::read_announcement_request(frame.body()).ok_or_else(|| incomplete(6))?;
    let Some(settings) = settings() else {
        return refuse(
            stream,
            session,
            &player.name,
            request_id,
            "sets the announcement",
            "no guild settings",
            trace,
        )
        .await;
    };
    let leads = player.guild.as_ref().map(Membership::leads);
    if let Err(why) = rules::may_announce(
        leads,
        text.encode_utf16().count(),
        wire::GUILD_ANNOUNCEMENT_UNITS,
    ) {
        return refuse(
            stream,
            session,
            &player.name,
            request_id,
            "sets the announcement",
            why,
            trace,
        )
        .await;
    }
    let Some(guild) = player
        .guild
        .as_ref()
        .map(|membership| membership.guild.clone())
    else {
        return refuse(
            stream,
            session,
            &player.name,
            request_id,
            "sets the announcement",
            "no guild",
            trace,
        )
        .await;
    };
    db::set_announcement(&world.pool, guild.id, &text, &player.name)
        .await
        .map_err(GameError::Database)?;
    let revision = next_revision(world, &guild, settings.revision_step.value).await?;
    if let Some(membership) = player.guild.as_mut() {
        membership.guild.announcement = text.clone();
        membership.guild.announcement_author = player.name.clone();
        membership.guild.revision = revision;
    }
    if trace {
        eprintln!(
            "world: guild: {:?} sets the announcement of {:?}",
            player.name, guild.name
        );
    }
    let frames = vec![
        wire::guild_announcement(&text, &player.name),
        wire::guild_revision(revision),
        wire::guild_request_result(request_id, wire::GUILD_RESULT_OK),
    ];
    send(stream, session, frames, trace).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guild() -> db::Guild {
        db::Guild {
            id: Uuid::from_bytes([9; 16]),
            name: "Guild".into(),
            tag: "TAG".into(),
            feature: 0,
            territory: 2,
            cape: db::Cape::default(),
            allegiance: 4,
            faction: 0,
            qualifier: 0,
            rating: 1000,
            rank: 0,
            announcement: String::new(),
            announcement_author: String::new(),
            revision: 0x0200,
        }
    }

    fn membership() -> Membership {
        Membership {
            guild: guild(),
            member: db::Member {
                guild_id: Uuid::from_bytes([9; 16]),
                character_id: Uuid::from_bytes([1; 16]),
                member_type: wire::GUILD_MEMBER_LEADER,
                invited_name: "Founder".into(),
                invited_by: "Founder".into(),
                joined_at: 0,
            },
        }
    }

    #[test]
    fn a_guild_keeps_its_local_id_and_two_guilds_never_share_one() {
        let ids = LocalIds::default();
        let a = Uuid::from_bytes([1; 16]);
        let b = Uuid::from_bytes([2; 16]);
        assert_eq!(ids.of(a), 1);
        assert_eq!(ids.of(b), 2);
        assert_eq!(ids.of(a), 1);
    }

    #[test]
    fn an_unguilded_load_is_the_end_marker_alone() {
        let frames = load_frames(None, None, 7);
        let headers: Vec<u16> = frames.iter().map(Frame::header).collect();
        assert_eq!(headers, vec![wire::GUILD_STREAM_END]);
    }

    #[test]
    fn a_guilded_load_is_info_property_role_end() {
        let membership = membership();
        let badge = Badge {
            local_id: 22,
            guild: membership.guild.clone(),
        };
        let frames = load_frames(Some(&badge), Some(&membership), 7);
        let headers: Vec<u16> = frames.iter().map(Frame::header).collect();
        assert_eq!(
            headers,
            vec![
                wire::GUILD_GENERAL_INFO,
                game::AGENT_PROPERTY_UPDATE_INT,
                wire::GUILD_PLAYER_ROLE,
                wire::GUILD_STREAM_END,
            ]
        );
        // Property 30, agent 7, local id 22.
        let property = frames[1].body();
        assert_eq!(&property[0..4], &30u32.to_le_bytes());
        assert_eq!(&property[4..8], &7u32.to_le_bytes());
        assert_eq!(&property[8..12], &22u32.to_le_bytes());
        // The role names the same guild by the same key.
        assert_eq!(&frames[2].body()[2..18], &[9; 16]);
        assert_eq!(frames[2].body()[18], wire::GUILD_MEMBER_LEADER);
    }

    #[test]
    fn the_guild_block_goes_before_the_party() {
        let mut block = vec![
            game::world_update_controlled_agent(1, 3),
            game::party_create(1),
            game::party_member_stream_end(1),
        ];
        assert!(splice_into_player_block(
            &mut block,
            load_frames(None, None, 1)
        ));
        let headers: Vec<u16> = block.iter().map(Frame::header).collect();
        assert_eq!(
            headers,
            vec![
                game::WORLD_UPDATE_CONTROLLED_AGENT,
                wire::GUILD_STREAM_END,
                game::PARTY_CREATE,
                game::PARTY_MEMBER_STREAM_END,
            ]
        );
        let mut no_party = vec![game::world_update_controlled_agent(1, 3)];
        assert!(!splice_into_player_block(&mut no_party, Vec::new()));
    }
}
