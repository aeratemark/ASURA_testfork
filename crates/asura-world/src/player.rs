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
    /// The segment the last accepted movement moved the agent along: where the server believed it stood
    /// before the report or correction, and where it believes it stands now. The doorway test reads it
    /// after the movement handler returns (`travel::portal_crossed`) - a doorway is a wall, and the step
    /// that passes through it is the crossing.
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
}

#[allow(non_snake_case)]
impl Player {
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

/// The experience this world reports for a character. There is no column for it yet, and a character
/// that has never earned any reads 0 - but 0 and "the field was never written" look the same to the
/// owner's eye, so this stays 1 until the column exists: a non-zero number is the proof that the
/// message is carrying the field at all.
const EXPERIENCE_TEST_VALUE: u32 = 500;

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
    /// The character's own row: appearance, campaign, level and the rest.
    pub traits: db_game::Traits,
    /// The unlocked-map bitmap, in the client's own encoding.
    pub unlocked: &'a [u32],
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
        traits,
        unlocked,
    } = *block;
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
    frames.push(game::agent_update_attribute_points(agent_id, 50, 50));
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
    frames.push(game::skillbar_update(agent_id, &[0; 8], &[0; 8], 1));
    frames.push(game::update_unlocked_skills(&[]));
    // The character record's own numbers, in the reference's order (`GameSrv_UpdatePlayerFactions`).
    // This used to be thirteen zeros, and thirteen zeros is what the client showed: level 0,
    // experience 0, morale 0. A live load leaves level 13, experience 68113 and morale 100 in the
    // client's record for a level-13 character (`dump4-after`, worldcontext.level/experience/morale),
    // and `characters.level` is where our level comes from. Experience and skill points have no column
    // yet - a freshly created character has neither - so they go out as zero rather than as absent.
    frames.push(game::character_update_factions(&game::CharacterProgress {
        // 500 until a column holds experience, so the sheet shows it working.
        experience: EXPERIENCE_TEST_VALUE,
        level: u32::from(traits.level),
        morale: MORALE_FULL,
        // Faction, skill points and their earned totals have no column yet.
        ..game::CharacterProgress::default()
    }));
    // A live load sends these two immediately after the faction block, once per player: the visibility
    // flags and the player's own flags word (`recordings/asura/world/load-layers-20261031.jsonl`,
    // headers 00EF then 003C). Without them the client's player record carries no flags and its
    // inventory display-mode column has nothing for any slot - the structures exist but stay empty.
    frames.push(game::char_visibility_flags(0xFFFF_FFFF, 0xFFFF_FFFF));
    frames.push(game::player_update_flags(player_id as u16, 0, 7));
    frames.push(game::agent_update_attributes(agent_id, &[0; 42]));
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
    frames.push(game::world_create_agent(&game::CreateAgent {
        agent_id,
        model_id: game::CHAR_CLASS_PLAYER_BASE | player_id,
        agent_type: game::AGENT_TYPE_LIVING,
        position: standing.position,
        plane: standing.plane,
        direction: standing.direction,
        speed_base: game::PLAYER_SPEED_BASE,
        player_team_token: PLAYER_TEAM_TOKEN,
    }));
    // The agent's own morale record, which the party panel's morale icon is drawn from. A live server
    // sends one per body at load with 100, the unmodified value; with the record left unset the client
    // paints a morale penalty the character does not have.
    frames.push(game::agent_update_morale(agent_id, MORALE_FULL));
    // Health and energy in the reference's order, then the regeneration rate and the public level.
    // The reference's own values, read off its send log: energy 25, health 400, regeneration 0.0 and
    // level 0 (it never sets `energy_per_sec` or `level` on a fresh agent).
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_ENERGY,
        agent_id,
        game::PLAYER_ENERGY_MAX,
    ));
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_HEALTH,
        agent_id,
        game::PLAYER_HEALTH_MAX,
    ));
    frames.push(game::agent_property_update_float(
        game::AGENT_PROPERTY_ENERGY_REGEN,
        agent_id,
        0.0,
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
    frames.push(game::party_create(PARTY_ID));
    frames.push(game::party_player_add(PARTY_ID, player_id as u16, 1));
    frames.push(game::party_member_stream_end(PARTY_ID));
    frames.push(game::character_update_party(PARTY_ID, 1));

    frames.push(game::instance_load_finish());
    frames
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
                traits: player.traits,
                unlocked: &unlocked,
            });
            // The title block and this player's own binding, at the places a live load carries them
            // (`crate::titles`). Nothing when titles are off or the character has no progress.
            crate::titles::sync_exploration(world, player).await;
            crate::titles::splice_into_player_block(
                &mut block,
                player.player_id,
                &mut player.titles,
            );
            block
        }
    };
    let others = player.instance.others(player.player_id);
    eprintln!(
        "world: [+{} ms] load block for instance {here}: {} frame(s); reached list {} ms, block {} ms, total {} ms",
        crate::stamp_ms(),
        block.len(),
        unlocked_at.as_millis(),
        block_started.elapsed().as_millis(),
        load_started.elapsed().as_millis()
    );
    for peer in &others {
        block.extend(introduction(peer));
    }
    let me = PeerView {
        player_id: player.player_id,
        agent_id: player.agent_id,
        name: player.name.clone(),
        standing: player.standing(),
        equipped: player.inventory.equipped_item_ids(),
        shown_title: player.titles.shown_rank().cloned(),
    };
    if !others.is_empty() {
        // The introduction, then what the others need in order to draw this player:
        // `GameSrv_BroadcastPlayerEquippedItems` (`GmItem.c:129-160`, the *unnamed* item form),
        // then `GameSrv_BroadcastUpdateAgentVisualEquipment` and `GameSrv_BroadcastAgentDisplayCape`
        // - the reference's own order in the same handler (`GameSrv.c:1643-1650`).
        let mut mine = introduction(&me);
        let equipped_bag = player
            .inventory
            .bags
            .iter()
            .find(|bag| bag.bag_type == 2)
            .map(|bag| bag.bag_id);
        for item in player
            .inventory
            .items
            .iter()
            .filter(|item| item.bag_id == equipped_bag)
        {
            mine.push(game::create_unnamed_item(&named_item(item)));
        }
        mine.push(equipment_frame(player));
        mine.push(game::agent_display_cape(player.agent_id, 1));
        player.instance.broadcast(player.player_id, &mine);
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
