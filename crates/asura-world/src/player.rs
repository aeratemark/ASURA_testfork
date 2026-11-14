//! The player: the character one connection drives, and the block that describes it.
//!
//! [`Player`] is the record and [`player_block`] is the character's own
//! `GAME_SMSG_INSTANCE_PLAYER_DATA_*` run; the constants below are the
//! reference's defaults for it (`PLAYER_ID`, `AGENT_ID`, `PARTY_ID`) and the profession it
//! reports when a character row carries none.

use crate::*;

/// The player: the character one connection drives, and the instance it is in.
///
/// The player as a record: the state this server owns, with the domain's verbs on it
/// (`player.GetAgentID()`).
///
/// Assembled after the token pair is believed and before the load block, so that everything the blocks
/// need - whose character this is, where it stands, what it carries - is decided in one place rather than
/// read again in the middle of a conversation.
pub struct Player {
    /// The character the transfer named. Every write on the way out is scoped to it.
    pub character: Uuid,
    /// This connection's player id, allocated when it joined. The reference allocates the same number
    /// from `srv->players` (`GmIdAllocate`) and puts it in `GAME_SMSG_PLAYER_CREATE`, every agent's
    /// `model_id`, the party messages and the chat speaker field.
    pub player_id: u32,
    /// The agent this character drives in its instance, as `GAME_SMSG_WORLD_CREATE_AGENT` names it.
    pub agent_id: u32,
    /// The instance this player is in: the map as a running object (its players, their agents and its
    /// clock), not the catalogue row that describes it. `player.instance.map` is that row.
    pub instance: Arc<Instance>,
    pub name: String,
    pub inventory: Inventory,
    /// Whether this connection has opened item stream 1. A second `0x8091` is answered without
    /// `0x0144`, because the client registers one inventory per stream id and asserts `!inventory`
    /// on a second create (`ItCliApi.cpp(2010)`, measured 2026-11-09).
    pub items_sent: bool,
    /// The login session and account the transfer named.
    ///
    /// The reference keeps both on its player (`GmPlayer.account_id`, `.char_id`) and uses them for every
    /// bag and item query and for its transfer request (`GameSrv.c:1450-1472`). This server scoped its
    /// writes by character alone, which is enough for everything it did until a *world* had to start a
    /// travel: the transfer row is keyed by session and needs the account as well (`docs/09` #9).
    pub session_id: Uuid,
    pub account_id: Uuid,
    /// Set when this connection has been redirected to another map. Its teardown must then leave the
    /// lease alone: the seat it holds now belongs to the connection the client is about to open.
    pub moved: bool,
    /// The last client-reported position checked for doorways. Simulation ticks never change it:
    /// a tick can cross a wall between reports, and that movement must remain in the next check.
    pub portal_position: (f32, f32),
    /// The segment between finite client reports or corrections, consumed by the doorway test.
    pub last_step: Option<((f32, f32), (f32, f32))>,
    /// What a character-creation connection has chosen so far: `Some` when this connection is in a
    /// creation instance (map type 11), which is the reference's `MapType_CharacterCreation`.
    pub creation: Option<Creation>,
    /// The character's stored traits - what the load block sends instead of constants.
    pub traits: db_game::Traits,
    /// The character's explored cells on the continent it is on. Read from
    /// `t_character_cartography` when the connection is built, saved as it changes and when the
    /// connection ends.
    pub explored: crate::cartography::Grid,
    /// The walking reveal: the last cell it marked and the paced queue its `0x008C` marks go
    /// through. Per connection, because the marker belongs to the socket that carries the marks.
    pub walker: crate::cartography::Walker,
    /// When the connection began, for the load time the reference reports.
    pub started: Instant,
    /// The character's title progress as read when the connection was built, the title it shows, and
    /// which tracks and rank records this connection has been sent (`crate::titles`).
    pub titles: crate::titles::TitleState,
    /// The character's guild and its own member row, as read when the connection was built and as this
    /// connection has changed it since (`crate::guild`). `None` for a character in no guild.
    pub guild: Option<crate::guild::Membership>,
    /// Set when the client said goodbye (`GAME_CMSG_DISCONNECT`). A connection that ends without it,
    /// and without a travel, is a network drop - the one ending an explorable party holds a place for
    /// (`crate::party::left_instance`).
    pub said_goodbye: bool,
    /// What the character has learned, its bar and its attribute ranks, as read when the connection was
    /// built and as this connection has changed them since (`crate::skill`).
    pub skills: crate::skill::Loadout,
    /// The gold in the character's storage, as read when the connection was built and as a Xunlai window
    /// has changed it since (`crate::vendor`). The carried gold is `traits.gold`.
    pub storage_gold: u32,
    /// The NPC window this connection has open, if any: a merchant's, a collector's or Xunlai storage
    /// (`crate::vendor`). What a trade request or a storage move is checked against.
    pub vendor: crate::vendor::Windows,
    /// The character's quest attempts, as read when the connection was built and as a dialog changes
    /// them (`crate::quest`). The row survives the session; this is the session's copy.
    pub quests: asura_rules::quest::Profile,
    /// Whether this connection is a login handoff rather than a map travel. Both rebuild the quest log
    /// inside the load (`crate::quest::LoadLog`).
    pub restore_quests: bool,
    /// The quests whose marker this connection has sent - in the load, with an acceptance, or in answer
    /// to the client's `0x8012` - so the client's request for one it already has is not answered again
    /// (`crate::quest::handle_info_request`).
    pub quest_markers_sent: std::collections::HashSet<u32>,
}

#[allow(non_snake_case)]
impl Player {
    /// Preserve the full reported movement even when simulation ticks advanced the agent meanwhile.
    pub fn record_portal_step(&mut self, position: (f32, f32)) {
        if crate::move_trace_on() && self.portal_position != position {
            eprintln!(
                "move[portal] map {} client segment ({:.1}, {:.1}) -> ({:.1}, {:.1})",
                self.instance.map.map_id,
                self.portal_position.0,
                self.portal_position.1,
                position.0,
                position.1
            );
        }
        self.last_step =
            (self.portal_position != position).then_some((self.portal_position, position));
        self.portal_position = position;
    }

    /// Where the agent is **now**, from the instance's own slot: what the reference answers the spawn
    /// and players requests with (`agent->position`). After the client has walked, that is no longer
    /// where it entered.
    pub fn standing(&self) -> Standing {
        self.instance
            .standing(self.player_id)
            .unwrap_or(Standing::from_map(&self.instance.map))
    }

    /// `Player.GetAgentID()`: the agent this character drives.
    pub fn GetAgentID(&self) -> u32 {
        self.agent_id
    }

    /// `Player.GetPlayerNumber()`: the player id the wire calls the player.
    pub fn GetPlayerNumber(&self) -> u32 {
        self.player_id
    }

    /// `Player.GetName()`: what `GAME_SMSG_PLAYER_CREATE` calls the character.
    pub fn GetName(&self) -> &str {
        &self.name
    }

    /// `Player.GetXY()`: where the character stands, as the server last believed it.
    pub fn GetXY(&self) -> (f32, f32) {
        self.standing().position
    }

    /// `Player.GetAgent()`: the agent record itself, as a copy (the instance owns the original).
    pub fn GetAgent(&self) -> Option<AgentState> {
        self.instance.agent(self.player_id)
    }

    /// `Player.GetPlayerUUID()`: the character's uuid, the one the login connection carried.
    pub fn GetPlayerUUID(&self) -> Uuid {
        self.character
    }

    /// `Player.GetGoldOnCharacter()`: the gold the load block reports.
    pub fn GetGoldOnCharacter(&self) -> u32 {
        self.traits.gold
    }
}
/// The one player and one agent this world server has.
///
/// The reference allocates both from per-server id tables (thousands of agents, hundreds of players).
/// ASURA has one character in a map, so the id table is a constant -'- and the party message carries
/// these as `u16`, which is the only real constraint. It becomes a real allocator when there is more
/// than one player to allocate for.
pub const PLAYER_ID: u32 = 1;

/// See [`PLAYER_ID`].
pub const AGENT_ID: u32 = 1;

/// The party a lone player is put in.
pub const PARTY_ID: u16 = 1;

/// `GameSrv_CreatePlayerAgent`'s arbitrary team token; the client keeps it for the session and the
/// reference never gives it a meaning.
pub const PLAYER_TEAM_TOKEN: u32 = 0xBAAD_F00D;

/// The primary profession the reference reports for its default character, read off its send log
/// (`GAME_SMSG_PLAYER_UPDATE_PROFESSION`: `01`). 0 is `Profession_None`, which is not a character a
/// client can render.
pub const DEFAULT_PRIMARY_PROFESSION: u8 = 1;

/// The unlocked-profession bitmap the creation screen is told, before a character exists
/// (`GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS`: `02 00 00 00`) - read from
/// `config/Server/defaults.json`. A real character's bitmap is written at the confirm and read from its
/// row on every load, so this is the screen's default and nothing else's.
pub fn creation_screen_unlocked_professions() -> u32 {
    asura_config::defaults()
        .map(|defaults| defaults.creation_screen_unlocked_professions())
        .unwrap_or(0)
}

/// How many UTF-16 units of a character name the confirm keeps, from
/// `config/Server/defaults.json` (`name_max_units`).
pub fn name_max_units() -> usize {
    asura_config::defaults()
        .map(|defaults| defaults.name_max_units())
        .unwrap_or(0)
}

/// Morale is a percentage around 100, and 100 is the value a live load carries for a character with no
/// morale modifier (`dump4-after`, `worldcontext.morale`). Sending 0 left the client's morale at zero.
const MORALE_FULL: u32 = 100;

/// What the player block is built from: everything a character's own messages carry.
///
/// A struct rather than eight positional arguments, for the reason `asura-rules` and
/// `asura-protocol` record (`docs/38-layer-decisions.md`, D16): the id, the agent id and the traits are
/// all small integers, so a swapped pair compiles and a client draws the wrong character.
pub struct PlayerBlock<'a> {
    /// The character's name, as the client shows it.
    pub name: &'a str,
    /// Where it stands: the position and plane the client draws it at.
    pub standing: Standing,
    /// Its bags and items, as rows were read.
    pub inventory: &'a Inventory,
    /// When this connection started, which the client's clock display uses.
    pub started: Instant,
    /// The player id this connection was given.
    pub player_id: u32,
    /// The agent id this connection was given.
    pub agent_id: u32,
    /// The party this player is in (`crate::party`): its own party of one at a first load.
    pub party_id: u16,
    /// The character's own row: appearance, campaign, level and the rest.
    pub traits: db_game::Traits,
    /// The unlocked-map bitmap, in the client's own encoding.
    pub unlocked: &'a [u32],
    /// The character's skills (`crate::skill`). `None` keeps the reference's own values: an empty bar,
    /// no learned skills, 50 free points of 50, no ranks, 25 energy, 400 health and no regeneration.
    pub loadout: Option<&'a crate::skill::Loadout>,
}

/// Everything the client is told about the players in the instance, in the reference's order
/// (`GameSrv_HandleInstanceLoadRequestPlayers`).
///
/// Two of the block's fields are there for the two things it no longer takes from a constant:
/// `standing`, which places the agent where the character actually is (`WORLD_CREATE_AGENT`), and
/// `inventory`, which says what the agent is wearing (`UPDATE_AGENT_VISUAL_EQUIPMENT`) - the nine ids
/// read out of the equipped-items bag, because they belong to *this* character's items.
pub fn player_block(block: &PlayerBlock<'_>) -> Vec<Frame> {
    let PlayerBlock {
        name,
        standing,
        inventory,
        started,
        player_id,
        agent_id,
        party_id,
        traits,
        unlocked,
        loadout,
    } = *block;
    // The skill pack decides the numbers a loadout's messages carry; without it, or without a loadout,
    // the block keeps the reference's own values.
    let skills = loadout.zip(crate::skill::catalog::current().ok());
    let mut frames = Vec::with_capacity(30);

    // The PvP unlocks. The reference sends its own fixed table of 387 unlockable items
    // (`GameSrv_SendPvpItems`), which this crate carries as content (`crate::pvp_items`, layer 9),
    // and the client builds its unlockable-item table from them, so they are copied rather than
    // skipped. The message builder stays in the wire crate; only the rows moved.
    frames.push(game::pvp_update_unlocked_skills(&[]));
    frames.push(game::pvp_update_unlocked_heroes(&[0xFFFF_FFFF; 8]));
    for item in crate::pvp_items::PVP_ITEMS {
        frames.push(game::pvp_item_stream_add(item.id, item.params));
    }
    frames.push(game::pvp_item_stream_end());

    // The account features, exactly the reference's list (`GameSrv_SendAccountFeatures`).
    for (feature, param1, param2) in [
        (1u16, 10u16, 0u16),
        (99, 3, 0),
        (100, 5, 0),
        (101, 5, 5),
        (102, 2, 2),
        (111, 1, 0),
        (124, 1, 0),
        (125, 1, 0),
        (131, 1, 0),
    ] {
        frames.push(game::account_feature(feature, param1, param2));
    }

    // The unlocked-map list. The two hard-mode bitmaps are sent at their **full length, zero-filled**,
    // because that is what the reference sends for a character with no mission progress: 31 words each.
    // An empty list is not the same message - it is an array the client can index past the end of.
    //
    // **The fifth list is the instance bitmap, and it is the one that has to be right.** The client's
    // world context holds five arrays in this order - missions completed, mission bonuses, missions
    // completed hard, bonuses hard, unlocked instances - and reads an instance's unlock as bit
    // `map_id % 32` of word `map_id / 32` in the fifth (`docs/22-world-model.md`). This server used to
    // send the fifth list empty, which is an unlocked array of length zero: every instance read as
    // locked, and the world map had nothing to offer. `unlocked` is the character's reached instances
    // (`map::unlock_bitmap`), and the instance it is standing in is always among them.
    frames.push(game::map_update_unlocked_list([
        &[][..],
        &[],
        crate::player::UNLOCKED_MAP_ZEROS,
        crate::player::UNLOCKED_MAP_ZEROS,
        unlocked,
    ]));
    frames.push(game::instance_loaded(PLAYER_TEAM_TOKEN));

    // Skills and attributes, then factions, then the attribute spread. The profession values are the
    // reference's own for its default character, read off its send log: primary 1 (Warrior) and one
    // unlocked-profession bit.
    match skills {
        Some((loadout, pack)) => {
            let (free, total) = crate::skill::character::attribute_points(
                loadout,
                u32::from(traits.level.max(1)),
                pack,
            );
            frames.push(game::agent_update_attribute_points(agent_id, free, total));
        }
        None => frames.push(game::agent_update_attribute_points(agent_id, 50, 50)),
    }
    frames.push(game::player_update_profession(
        agent_id,
        traits.profession,
        0,
        0,
    ));
    frames.push(game::player_update_unlocked_professions(
        agent_id,
        traits.unlocked_professions,
    ));
    // The bar, then the learned set: preforged's restoration order (`gw_character_skills.h`) puts the
    // learned set first, the reference sends the bar first; the reference's order is kept.
    match skills {
        Some((loadout, _)) => {
            frames.push(crate::skill::character::bar_frame(agent_id, loadout));
            frames.push(crate::skill::character::learned_frame(loadout));
        }
        None => {
            frames.push(game::skillbar_update(agent_id, &[0; 8], &[0; 8], 1));
            frames.push(game::update_unlocked_skills(&[]));
        }
    }
    // The character record's own numbers, in the reference's order (`GameSrv_UpdatePlayerFactions`).
    // This used to be thirteen zeros, and thirteen zeros is what the client showed: level 0,
    // experience 0, morale 0. A live load leaves level 13, experience 68113 and morale 100 in the
    // client's record for a level-13 character (`dump4-after`, worldcontext.level/experience/morale),
    // and `characters.level` and `characters.experience` are where our numbers come from.
    frames.push(game::character_update_factions(&game::CharacterProgress {
        experience: traits.experience,
        level: u32::from(traits.level),
        morale: MORALE_FULL,
        current_skill_points: traits.skill_points,
        total_earned_skill_points: traits.skill_points_total,
        ..game::CharacterProgress::default()
    }));
    // A live load sends these two immediately after the faction block, once per player: the visibility
    // flags and the player's own flags word (`recordings/asura/world/load-layers-20261031.jsonl`,
    // headers 00EF then 003C). Without them the client's player record carries no flags and its
    // inventory display-mode column has nothing for any slot - the structures exist but stay empty.
    frames.push(game::char_visibility_flags(0xFFFF_FFFF, 0xFFFF_FFFF));
    frames.push(game::player_update_flags(player_id as u16, 0, 7));
    match skills {
        Some((loadout, pack)) => frames.push(crate::skill::character::attributes_frame(
            agent_id, loadout, &traits, pack,
        )),
        None => frames.push(game::agent_update_attributes(agent_id, &[0; 42])),
    }
    frames.push(game::world_update_load_time(
        started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
    ));

    // The player, the agent, and what a client needs in order to draw them. The appearance carries
    // the profession the model is built from: all-zero bits say `Profession_None`, which is not a
    // character the client can draw, so the reference's own default is used.
    // The appearance the client chose, or the reference's default for a character with no bits recorded:
    // all-zero bits say `Profession_None`, which is not a character the client can draw.
    let appearance = if traits.appearance == 0 {
        // The look for a character with no bits of its own
        // (`config/World/Vanilla/character_defaults.json`, `appearance`): all-zero bits are
        // `Profession_None`, which the client cannot draw.
        crate::character_defaults::current()
            .map(|defaults| defaults.appearance().pack())
            .unwrap_or_default()
    } else {
        traits.appearance
    };
    frames.push(game::player_create(player_id, agent_id, appearance, name));
    // The profession goes out **twice** in the reference: once with the skills and attributes, and
    // again here, immediately after the player is created and before the agent exists
    // (`GameSrv_HandleInstanceLoadRequestPlayers`: `BroadcastPlayerCreate` then
    // `SendPlayerProfession`). It is the second one that gives the freshly created player the
    // profession its model is built from - without it the client draws an agent it has no model for.
    frames.push(game::player_update_profession(
        agent_id,
        traits.profession,
        0,
        0,
    ));
    frames.push(game::player_create_agent(
        agent_id,
        player_id,
        standing.position,
        standing.plane,
        standing.direction,
        PLAYER_TEAM_TOKEN,
    ));
    // The agent's own morale record, which the party panel's morale icon is drawn from. A live server
    // sends one per body at load with 100, the unmodified value; with the record left unset the client
    // paints a morale penalty the character does not have.
    frames.push(game::agent_update_morale(agent_id, MORALE_FULL));
    // Health and energy in the reference's order, then the regeneration rate and the public level.
    // The reference's own values, read off its send log: energy 25, health 400, regeneration 0.0 and
    // level 0 (it never sets `energy_per_sec` or `level` on a fresh agent).
    // With a loadout the three carry the pack's own player rules (`combat.json`): maximum energy, maximum
    // health for the character's level, and the energy regeneration the fight runs at.
    let (energy, health, regen) = match skills {
        Some((_, pack)) => crate::skill::character::vitals(&traits, pack),
        None => (game::PLAYER_ENERGY_MAX, game::PLAYER_HEALTH_MAX, 0.0),
    };
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_ENERGY,
        agent_id,
        energy,
    ));
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_HEALTH,
        agent_id,
        health,
    ));
    frames.push(game::agent_property_update_float(
        game::AGENT_PROPERTY_ENERGY_REGEN,
        agent_id,
        regen,
    ));
    // The agent's level, which the party window draws. It is not a field of the create message: it
    // arrives as agent int-property 36 (`PublicLevel`), which is what the client stores in
    // `AgentLiving.level` - measured on our own client as 0 while this sent 0, and Rurik's own reading
    // gives property 36 as the level on this exact opcode (`studies/character/FINDINGS.md`).
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_PUBLIC_LEVEL,
        agent_id,
        u32::from(traits.level),
    ));
    frames.push(game::agent_initial_effects(agent_id, 0));
    frames.push(game::update_agent_visual_equipment(
        agent_id,
        inventory.equipped_item_ids(),
    ));
    frames.push(game::agent_display_cape(agent_id, 1));
    frames.push(game::world_update_controlled_agent(agent_id, 3));

    // The party: one member, this player, and the client's own party pointer.
    frames.push(game::party_create(party_id));
    frames.push(game::party_player_add(party_id, player_id as u16, 1));
    frames.push(game::party_member_stream_end(party_id));
    frames.push(game::character_update_party(party_id, 1));

    frames.push(game::instance_load_finish());
    frames
}

// ---- experience and the level it implies ---------------------------------------------------------

/// `/.exp <total>`: the one place this world changes a character's experience.
///
/// The curve is the pack's (`crate::experience`); the level is what the total says, not a number the
/// caller chooses. Three client-visible values a level change owns are re-sent together: the fifteen-
/// dword progress block (`0x00E9`), the agent's public level (`0x009F` kind 36) and the attribute
/// points a level grants (`0x0037`). The health and energy pools, and the fight's own copy of them,
/// are refreshed on the next load: a level-up in the middle of a fight is not modelled yet.
pub async fn set_experience(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    total: u32,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(curve) = crate::experience::curve() else {
        let line = "Experience is unavailable: the pack has no experience curve.";
        send(stream, session, crate::chat::tell(player, line), trace).await?;
        return Ok(true);
    };
    let total = total.min(curve.max_total());
    let level = curve.level_for(total);
    let previous_level = u32::from(player.traits.level.max(1));
    let gained = curve
        .skill_points_for(level)
        .saturating_sub(curve.skill_points_for(previous_level));
    let before = player.traits;
    player.traits.experience = total;
    player.traits.level = u16::try_from(level).unwrap_or(u16::MAX);
    player.traits.skill_points = player.traits.skill_points.saturating_add(gained);
    player.traits.skill_points_total = player.traits.skill_points_total.saturating_add(gained);
    // A lower level gives back the points its levels granted. Quests do not grant points yet, so the
    // level's own grant is the whole ledger; a reset command that left level 1 holding level 4's
    // points would be lying to the next load.
    if level < previous_level {
        let granted = curve.skill_points_for(level);
        player.traits.skill_points = player.traits.skill_points.min(granted);
        player.traits.skill_points_total = player
            .traits
            .skill_points_total
            .min(granted)
            .max(player.traits.skill_points);
    }
    if let Err(err) = db_game::set_progress(
        &world.pool,
        player.character,
        player.traits.experience,
        player.traits.level,
        player.traits.skill_points,
        player.traits.skill_points_total,
    )
    .await
    {
        player.traits = before;
        eprintln!("world: /.exp: the database refused the change: {err}");
        let line = "Experience was not changed: the character row refused the write.";
        send(stream, session, crate::chat::tell(player, line), trace).await?;
        return Ok(true);
    }
    eprintln!(
        "world: {} experience {total}, level {level} (was {}), skill points {}/{}",
        player.name,
        before.level,
        player.traits.skill_points,
        player.traits.skill_points_total
    );
    // The bar moves from the delta (`0x00EE`), not from a re-sent load block: Rurik measured the client
    // applying `[0, xp]` with `+=` (toolkit/authsrv/authsrv.py:5911-5960), and a re-sent `0x00E9` left
    // this client's bar still. **Only the award is sent.** The public level (`0x009F` 36) and the
    // attribute points (`0x0037`) are not: on 2026-11-13 this client crashed on a change that sent
    // them, and the measured kill and quest paths send neither - the client works its own level out
    // from the award, and the server's level and points are read at the next load.
    //
    // An award that would cross more than one level is not a shape retail sends (a kill or a 500-point
    // quest crosses at most one), and a multi-level `0x00EE` is what the crash followed. Such a jump
    // restates the whole `0x00E9` block instead, which is the login's own safe path; a jump of one
    // level or less uses the measured award and moves the bar.
    let mut frames = Vec::new();
    if total > before.experience && level <= previous_level.saturating_add(1) {
        frames.push(asura_protocol::game::skill::agent_kill_reward(
            asura_protocol::game::skill::ATTRIBUTE_EXPERIENCE,
            total - before.experience,
        ));
    } else if total != before.experience {
        // A decrease has no negative delta, and a multi-level jump is not a retail award shape; both
        // restate the whole block, which the login already sends safely.
        frames.push(game::character_update_factions(
            &game::CharacterProgress {
                experience: player.traits.experience,
                level: u32::from(player.traits.level),
                morale: MORALE_FULL,
                current_skill_points: player.traits.skill_points,
                total_earned_skill_points: player.traits.skill_points_total,
                ..game::CharacterProgress::default()
            },
        ));
    }
    frames.extend(crate::chat::tell(
        player,
        &format!("Experience {total}; level {level}."),
    ));
    send(stream, session, frames, trace).await?;
    Ok(true)
}

// ---- the instances a character has reached, as the client addresses them ---------------------

/// A zero-filled bitmap of the length the reference uses for a character with no progress: 31 words.
///
/// Copied from its trace, not chosen: `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST` there carries two lists of
/// 31 zero dwords (`02 00 00 00` twice, then 31 words of zeros, twice more). Sending the lists empty
/// instead changes the *length* of a bitmap the client indexes by map id.
pub const UNLOCKED_MAP_ZEROS: &[u32] = &[0u32; 31];

/// The instance bitmap a character's reached instances become, in the client's own addressing.
///
/// **Read from the client's world context** (`docs/22-world-model.md`): the bitmap is an array of
/// `u32` where instance `n` is bit `n % 32` of word `n / 32`. 31 words covers instances 0 to 991,
/// which contains the reference's own `MapId_Count` sentinel of 877 (`MAP_ID_COUNT`), so a character
/// can never outgrow the list the reference sends.
///
/// An id outside that range, or a negative one, is dropped rather than growing the list: the width is
/// the message's, and an instance the client cannot address is not made addressable by a longer array.
pub fn unlock_bitmap(instances: &[i32]) -> Vec<u32> {
    let mut words = UNLOCKED_MAP_ZEROS.to_vec();
    for &map_id in instances {
        let index = usize::try_from(map_id).ok().map(|id| id / 32);
        if let Some(word) = index.filter(|index| *index < words.len()) {
            words[word] |= 1 << (map_id.rem_euclid(32) as u32);
        }
    }
    words
}

/// The player is somewhere new: record the cells the step reveals and save them now.
///
/// Two callers: the position reports and the connection's walk poll. The poll exists because a
/// server-side walk sends no reports until it ends, so without it a click-walk would reveal nothing
/// until the arrival. The save is immediate for the same reason the position save is best-effort:
/// a travel opens the next map's connection before this one has closed, and the next connection
/// reads the row as it stands.
pub(crate) async fn walked(world: &World, player: &mut Player, trace: bool) {
    if player.moved {
        player.walker.halt();
        return;
    }
    let map_id = player.instance.map.map_id;
    let position = player.GetXY();
    let before = player.explored.explored_count();
    let queued = player
        .walker
        .advance(&mut player.explored, map_id, position);
    let now = player.explored.explored_count();
    if trace && !queued.is_empty() {
        eprintln!(
            "world: cartography: at ({:.0}, {:.0}) on map {map_id}, {} mark(s) queued, last at {:?}; {now} cells explored",
            position.0,
            position.1,
            queued.len(),
            queued.last(),
        );
    }
    if now != before {
        save_explored(world, player).await;
        // A title earned by exploration follows the grid (`crate::titles`).
        crate::titles::exploration_changed(world, player).await;
    }
}

/// Write this character's grid to its row, now.
pub async fn save_explored(world: &World, player: &Player) {
    let (dims_x, dims_y) = player.explored.dims();
    let stored = db_cartography::Explored {
        dims_x: dims_x as i16,
        dims_y: dims_y as i16,
        blocks: player.explored.packed(),
        explored: player.explored.explored_count() as i32,
    };
    if let Err(err) = db_cartography::record_explored(
        &world.pool,
        player.character,
        crate::cartography::continent_for(player.instance.map.map_id),
        &stored,
    )
    .await
    {
        eprintln!("world: could not save the exploration as it was earned: {err}");
    }
}

// ---- the client asks who else is in the instance ------------------------------------------

/// The client asks who else is here. This is the big block, and its last message
/// (`INSTANCE_LOAD_FINISH`) is what takes the client off the loading screen.
///
/// Everyone already in the map goes after it, in the reference's own order for a joining client
/// (`GameSrv_SendWorldAgents`, `GmAgent.c:310-331`), and the same frames go out to *them* so they
/// learn about this one. The reference broadcasts the player create there and unicasts the rest;
/// the order is what matters here, and it is the same in both directions.
pub async fn handle_request_players(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let replay = world.replay.as_ref();
    // The load block's own cost, measured. The first instance a fresh process serves is the one that warms
    // every cache behind it, so these numbers are what separates a server-side delay from a client-side
    // one: a block built in single-digit milliseconds cannot be why a client's load drags.
    let load_started = Instant::now();
    // The instance the character is standing in is reached, so it is unlocked - that is the whole rule
    // (`docs/22-world-model.md`): the walk sets the bit, the bit never gates the walk. The creation
    // instance is not a place a character "reaches", so it sets nothing.
    let here = player.instance.map.map_id;
    // The last outpost this character stood in, kept for the base rule (docs/22-world-model.md): a travel
    // into an explorable must not overwrite it, which is why only an outpost is written here.
    if player.instance.map.map_type == map_kinds().main_town {
        if let Err(err) = db_game::remember_outpost(&world.pool, player.character, here).await {
            eprintln!("world: could not remember the outpost {here}: {err}");
        }
    }
    if player.instance.map.map_type != map_kinds().character_creation {
        match db_game::unlock_instance(&world.pool, player.character, here).await {
            Ok(true) => eprintln!(
                "world: {:?} unlocked instance {here}: first arrival",
                player.name
            ),
            Ok(false) => {}
            Err(err) => eprintln!("world: could not record the arrival in instance {here}: {err}"),
        }
    }
    // The bitmaps the client is given: the character's own reached instances, and the one it is in even
    // if the database could not be read - a client told it is nowhere can offer nowhere.
    let unlocked = match db_game::unlocked_instances(&world.pool, player.character).await {
        Ok(instances) => unlock_bitmap(&instances),
        Err(err) => {
            eprintln!("world: could not read the reached instances, sending this one only: {err}");
            unlock_bitmap(&[here])
        }
    };
    let unlocked_at = load_started.elapsed();
    let block_started = Instant::now();
    let mut block = match replay {
        Some(replay) if replay.pick.wants(crate::replay::BlockName::Players) => {
            replay.players.clone()
        }
        _ => {
            let mut block = player_block(&crate::player::PlayerBlock {
                name: &player.name,
                standing: player.standing(),
                inventory: &player.inventory,
                started: player.started,
                player_id: player.player_id,
                agent_id: player.agent_id,
                party_id: player
                    .instance
                    .parties
                    .party_of(player.player_id)
                    .map(|party| party.party_id)
                    .unwrap_or(PARTY_ID),
                traits: player.traits,
                unlocked: &unlocked,
                loadout: Some(&player.skills),
            });
            // The title block and this player's own binding, at the places a live load carries them
            // (`crate::titles`). Nothing when titles are off or the character has no progress.
            crate::titles::sync_exploration(world, player).await;
            crate::titles::splice_into_player_block(
                &mut block,
                player.player_id,
                &mut player.titles,
            );
            // The party state every live load ends with (`crate::party::load_tail`).
            crate::party::splice_load_tail(
                &mut block,
                player.instance.map.map_type == map_kinds().main_town,
            );
            // The guild block in front of the party block, where a live load carries it: `0x011C`
            // alone for a character in no guild (`crate::guild`). Not in a creation instance, which no
            // capture shows a guild message in.
            if player.instance.map.map_type != map_kinds().character_creation {
                let badge = crate::guild::badge(&player.instance, player.guild.as_ref());
                let guild = crate::guild::load_frames(
                    badge.as_ref(),
                    player.guild.as_ref(),
                    player.agent_id,
                );
                if !crate::guild::splice_into_player_block(&mut block, guild) {
                    eprintln!(
                        "world: guild: the player block has no PARTY_CREATE anchor; no guild block sent"
                    );
                }
            }
            block
        }
    };
    // The players already in the map, as this one enters it: the ones its block introduces and the only
    // ones it is introduced to (`Instance::enter`).
    let entered = player.instance.enter(player.player_id);
    let others: Vec<PeerView> = player
        .instance
        .others(player.player_id)
        .into_iter()
        .filter(|peer| entered.contains(&peer.player_id))
        .collect();
    // The player's party as the instance has it - a party that has just come through a doorway with its
    // members still on the way included - behind the guild block, where a live load carries it, each
    // member in the map only once this client has met it (`crate::party::load_section`). Not in a
    // replayed block, which is the reference's own bytes.
    let built = !replay.is_some_and(|replay| replay.pick.wants(crate::replay::BlockName::Players));
    if built {
        crate::party::splice_load_section(&mut block, &player.instance, player.player_id);
    }
    eprintln!(
        "world: [+{} ms] load block for instance {here}: {} frame(s); reached list {} ms, block {} ms, total {} ms",
        crate::stamp_ms(),
        block.len(),
        unlocked_at.as_millis(),
        block_started.elapsed().as_millis(),
        load_started.elapsed().as_millis()
    );
    // The agent array: every other player and every NPC within this client's range, in agent-id order.
    // A live load interleaves the two that way (`recordings/client/feature/maps/pre-searing-load-3.jsonl`
    // seq 210-298: player 0x28, NPCs 0x29-0x3C, player 0x3E, NPC 0x3F, ...), each NPC with its
    // definition in front of it the first time the client needs it (`crate::npc::burst`). Not in a
    // replayed block, whose NPCs the tick then creates as they come into range.
    let mut agents: Vec<(u32, Vec<Frame>)> = others
        .iter()
        .map(|peer| (peer.agent_id, introduction(peer)))
        .collect();
    if built {
        // The NPCs in range, each with the quest marker the live server puts above it: an offer (5) or
        // a turn-in (4), from the character's own profile (`crate::quest::marker_value`). The marker
        // follows the entry burst, where the capture sends it after `0x006D` (tick 169039203).
        for (agent_id, mut frames) in player
            .instance
            .npc_arrival(player.player_id, player.standing().position)
        {
            if let Some(target) = player.instance.npc_target(agent_id) {
                if let Some(name) = target.name_text() {
                    let marker = crate::quest::marker_for(
                        &crate::quest::Who::of(player),
                        &name,
                        player.instance.map.map_id,
                    );
                    if marker != asura_rules::quest::Marker::None {
                        frames.push(crate::quest::marker_frame(
                            agent_id,
                            crate::quest::marker_value(marker),
                        ));
                    }
                }
            }
            agents.push((agent_id, frames));
        }
    }
    agents.sort_by_key(|(agent_id, _)| *agent_id);
    for (_, frames) in agents {
        block.extend(frames);
    }
    // The party members it was just introduced to are in the map (`crate::party::in_map_notices`).
    if built {
        block.extend(crate::party::in_map_notices(
            &player.instance,
            player.player_id,
            &entered,
        ));
    }
    let me = PeerView {
        player_id: player.player_id,
        agent_id: player.agent_id,
        name: player.name.clone(),
        standing: player.standing(),
        equipped: player.inventory.equipped_item_ids(),
        shown_title: player.titles.shown_rank().cloned(),
        look: crate::instance::Look::from_traits(&player.traits),
        wearing: crate::instance::wearing(&player.inventory),
        party_leader: player.instance.parties.leader_and_size(player.player_id).0,
        party_size: player.instance.parties.leader_and_size(player.player_id).1,
        guild: crate::guild::badge(&player.instance, player.guild.as_ref()),
    };
    // The same introduction the newcomer was just given for each of them, now the other way: the
    // player, the agent, then its equipped items, visual equipment and cape
    // (`GameSrv_BroadcastPlayerEquippedItems` and the two after it, `GameSrv.c:1643-1650`). Only to the
    // players its block introduced: one still loading meets it in its own block.
    let hello = introduction(&me);
    for peer in &entered {
        player.instance.send_to(*peer, &hello);
    }
    // A party member that has just arrived: its party is told, now that its player exists on their
    // clients (`crate::party::announce_arrival`).
    crate::party::announce_arrival(&player.instance, player.player_id, &entered, trace);
    // Items already on the ground in this map, for a client that has just entered it: the record the
    // client draws (create for a whole drop, update for the part of a stack) and its agent. A live
    // server's ground items survive a relog; the instance owns these, so they do too.
    if built {
        for ground in player.instance.ground_items() {
            block.extend(crate::item::ground_frames(&ground));
        }
    }
    // The quest log, inside the load as the live server sends it (`crate::quest::LoadLog`): the
    // general info went out with the item answer; the markers go before the load finishes, the details
    // just after it. A replay block is the reference's own bytes.
    if built {
        let log = crate::quest::load_log(&player.quests, &player.name);
        if trace && !log.details.is_empty() {
            eprintln!(
                "world: {}'s quest log after a {}: {} marker(s), {} detail(s)",
                player.name,
                if player.restore_quests {
                    "login"
                } else {
                    "travel"
                },
                log.markers.len(),
                log.details.len()
            );
        }
        let finish = block
            .iter()
            .position(|frame| frame.header() == game::INSTANCE_LOAD_FINISH)
            .unwrap_or(block.len());
        block.splice(finish..finish, log.markers);
        player.quest_markers_sent.extend(log.marked);
        block.extend(log.details);
    }
    send(stream, session, block, trace).await?;
    // The load is over: the block's last message was `INSTANCE_LOAD_FINISH`. Only now may the instance's
    // clock tick this player - measured, not assumed: 36 live loads carry no `0x001E` between the client's
    // spawn request and this one, and this world used to send 49 into that window from the unasked block
    // onwards (see `run`'s comment in connection.rs).
    //
    // The stored exploration follows the load. `cartography_replay_delay_ms` keeps its first mark out
    // of the frame that finishes the load; a player who opens M without moving then sees the saved map
    // fill immediately rather than waiting for a position report.
    tokio::time::sleep(crate::cartography::replay_start_delay()).await;
    let replayed = player.walker.replay(&player.explored);
    if trace && !replayed.is_empty() {
        eprintln!(
            "world: cartography: replayed {} stored mark(s) after the load",
            replayed.len()
        );
    }
    player.instance.loaded(player.player_id);
    Ok(true)
}
