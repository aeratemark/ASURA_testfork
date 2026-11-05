//! Titles on the game connection: the load's title block, the rank a player shows, the two client
//! requests, and the hook point changes go through.
//!
//! The definitions are `config/Server/titles.json` (`asura_config::titles`), the decisions are
//! `asura_rules::titles`, the bytes are `asura_protocol::game::title`, and the points are
//! `t_character_titles` / `t_account_titles` (migration 0028). This module puts them on the wire in the
//! order build 39112's live server does (`docs/40-titles-system-research.md`, section 5):
//!
//! * **Load.** Right after the player's `0x003C`: for each track with points, ascending by title id, its
//!   current and next rank records (`0x00F3`) and then the track (`0x00F6`)
//!   (`recordings/client/feature/travel/portal-battery-20261031-013856.jsonl` seq 706-749).
//! * **What a player shows.** `0x00F4` sits after `WORLD_UPDATE_CONTROLLED_AGENT` for the player's own
//!   load (same capture, seq 205-207), and right before `WORLD_CREATE_AGENT` (after the `0x003C` retail
//!   sends there) when another player is introduced, preceded by that rank's `0x00F3` when the record is
//!   new (`recordings/client/feature/maps/pre-searing-load-3.jsonl` seq 216-221 and 672-676).
//!
//! Experimental: nothing earns points yet. [`add_points`] is the one entry an earning rule calls, and a
//! track appears once its row has points.

use crate::*;
use asura_rules::titles::{self as rules, PointChange, Rank, Scope, Title};
use std::collections::{HashMap, HashSet};

/// The title definitions in force: empty when the feature is off or the file was refused.
pub fn definitions() -> &'static [Title] {
    asura_config::titles().definitions()
}

/// What one connection holds about titles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleState {
    /// Points per defined title, from the row its scope names. A title with no row has no entry.
    pub points: HashMap<u32, u32>,
    /// The title the character asked to show, as stored.
    pub display: Option<u32>,
    /// The tracks this connection was sent a `0x00F6` for: a `0x00F5` to any other is inert.
    pub sent_tracks: HashSet<u32>,
    /// The rank records this connection was sent: a record is seeded once per connection.
    pub seeded_ranks: HashSet<u32>,
}

impl TitleState {
    /// The points on one title, 0 when it has no row.
    pub fn points(&self, title_id: u32) -> u32 {
        self.points.get(&title_id).copied().unwrap_or(0)
    }

    /// The rank this character shows now: the stored title's current rung, if the rules still allow it.
    pub fn shown_rank(&self) -> Option<&'static Rank> {
        let title_id = self.display?;
        rules::display_rank(definitions(), title_id, self.points(title_id)).ok()
    }
}

/// Read a character's title progress. A failed read is logged and leaves the character with no titles for
/// this connection: titles never stand between a player and the map.
pub async fn load(pool: &PgPool, character: Uuid, account: Uuid) -> TitleState {
    let titles = definitions();
    if titles.is_empty() {
        return TitleState::default();
    }
    let progress = match db_titles::progress(pool, character, account).await {
        Ok(progress) => progress,
        Err(err) => {
            eprintln!("world: titles: could not read the progress, sending none: {err}");
            return TitleState::default();
        }
    };
    let display = match db_titles::display_title(pool, character).await {
        Ok(display) => display,
        Err(err) => {
            eprintln!("world: titles: could not read the shown title: {err}");
            None
        }
    };
    let mut points = HashMap::new();
    for title in titles {
        let rows = match title.scope {
            Scope::Character => &progress.character,
            Scope::Account => &progress.account,
        };
        if let Some((_, value)) = rows.iter().find(|(id, _)| *id == title.title_id) {
            points.insert(title.title_id, *value);
        }
    }
    TitleState {
        points,
        display,
        ..TitleState::default()
    }
}

/// `0x00F3` for one rank. The definitions were validated when the file was read, so a refusal here is a
/// bug; it is logged and the record skipped rather than sent cut.
fn rank_record(rank: &Rank) -> Option<Frame> {
    match game::title::title_rank_data(rank.rank_id, rank.flags, rank.value, &rank.name) {
        Ok(frame) => Some(frame),
        Err(err) => {
            eprintln!("world: titles: rank {} not sent: {err}", rank.rank_id);
            None
        }
    }
}

/// One track's group - its two records, then the track - and the bookkeeping that it went out. `None`
/// when the rules place no track at these points, or a frame could not be built; then nothing of the
/// group is returned, because a track without its records is the crash rule (doc 40, section 6).
fn group_frames(title: &Title, points: u32, state: &mut TitleState) -> Option<Vec<Frame>> {
    let group = rules::track_group(title, points)?;
    let mut frames = Vec::with_capacity(3);
    for rank in group.ranks {
        frames.push(rank_record(rank)?);
    }
    match game::title::title_track_info(&group.track) {
        Ok(frame) => frames.push(frame),
        Err(err) => {
            eprintln!("world: titles: title {} not sent: {err}", title.title_id);
            return None;
        }
    }
    state
        .seeded_ranks
        .extend(group.ranks.iter().map(|rank| rank.rank_id));
    state.sent_tracks.insert(title.title_id);
    Some(frames)
}

/// The load's title block: every track with points, ascending by title id, each as its group.
///
/// A defined title whose points the rules cannot place - a maxed track with no sentinel - is logged and
/// left out, rather than sent naming a record that does not exist.
pub fn load_block(state: &mut TitleState) -> Vec<Frame> {
    let mut titles: Vec<&Title> = definitions().iter().collect();
    titles.sort_by_key(|title| title.title_id);
    let mut frames = Vec::new();
    for title in titles {
        let points = state.points(title.title_id);
        if points == 0 {
            continue;
        }
        match group_frames(title, points, state) {
            Some(group) => frames.extend(group),
            None => eprintln!(
                "world: titles: title {} at {points} point(s) has no track to send (a maxed track with \
                 no `maxed` record in config/Server/titles.json?)",
                title.title_id
            ),
        }
    }
    frames
}

/// The binding and what follows it on the wire: `0x00F4`, then that player's `0x003C` flags word.
///
/// Retail sends the pair back to back: 142 of the 144 `0x00F4` in `recordings/client/feature/` are
/// followed at once by a `0x003C` for the same player, mask 7 (values 0, 4, 5 and 7 seen). The value
/// here is the 0 this server's own load sends for the player (`player::player_block`). Without it the
/// client took the binding but drew no title under the name (owner's live client, 2026-11-05).
fn display_frames(player_id: u32, rank_id: u32) -> [Frame; 2] {
    [
        game::title::title_rank_display(player_id as u16, rank_id),
        game::player_update_flags(player_id as u16, 0, 7),
    ]
}

/// What another client is told about a player's shown rank: the record, then the binding. Always both,
/// because this server does not track which records each peer has.
pub fn binding_for_peer(player_id: u32, rank: &Rank) -> Vec<Frame> {
    rank_record(rank)
        .into_iter()
        .chain(display_frames(player_id, rank.rank_id))
        .collect()
}

/// What this connection is told about its own shown rank: the record only if this connection has not been
/// sent it, then the binding.
fn binding_for_self(player_id: u32, rank: &Rank, state: &mut TitleState) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(2);
    if !state.seeded_ranks.contains(&rank.rank_id) {
        if let Some(record) = rank_record(rank) {
            frames.push(record);
            state.seeded_ranks.insert(rank.rank_id);
        }
    }
    frames.extend(display_frames(player_id, rank.rank_id));
    frames
}

/// Put the title frames into a built player block at retail's places: the track groups right after the
/// player's own `0x003C`, the player's binding right after `WORLD_UPDATE_CONTROLLED_AGENT`.
///
/// A block that has neither anchor is left as it is and the frames are not sent: a load built some other
/// way (a replayed block, say) is not this server's to rearrange.
pub fn splice_into_player_block(block: &mut Vec<Frame>, player_id: u32, state: &mut TitleState) {
    if definitions().is_empty() {
        return;
    }
    let groups = load_block(state);
    let binding = match state.shown_rank() {
        Some(rank) => binding_for_self(player_id, rank, state),
        None => Vec::new(),
    };
    if !splice_at_anchors(block, groups, binding) {
        eprintln!("world: titles: the player block has no 0x003C / 0x0022 anchor; no titles sent");
    }
}

/// The placement itself, apart from where the frames come from: `groups` after the first `0x003C`,
/// `binding` after the first `0x0022`. `false`, and the block untouched, when either anchor is missing.
fn splice_at_anchors(block: &mut Vec<Frame>, groups: Vec<Frame>, binding: Vec<Frame>) -> bool {
    let flags_at = block
        .iter()
        .position(|frame| frame.header() == game::PLAYER_UPDATE_FLAGS);
    let controlled_at = block
        .iter()
        .position(|frame| frame.header() == game::WORLD_UPDATE_CONTROLLED_AGENT);
    let (Some(flags_at), Some(controlled_at)) = (flags_at, controlled_at) else {
        return false;
    };
    // The later anchor first, so the earlier index still points where it did.
    let (first, second) = if controlled_at > flags_at {
        ((flags_at, groups), (controlled_at, binding))
    } else {
        ((controlled_at, binding), (flags_at, groups))
    };
    block.splice(second.0 + 1..second.0 + 1, second.1);
    block.splice(first.0 + 1..first.0 + 1, first.1);
    true
}

/// `GAME_CMSG_TITLE_DISPLAY` (`0x8058`): the client asks to show a title.
///
/// Answered with the binding (`0x00F4`, the record first when it is new) to this client and to everyone in
/// the instance, which is what the ledger records the server doing (`docs/36-game-rules-ledger.md` E160);
/// no capture of this exchange exists yet. A refusal sends nothing and logs why.
pub async fn handle_display(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(title_id) = game::title::read_title_display_request(frame.body()) else {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 6 }));
    };
    if definitions().is_empty() {
        eprintln!("world: titles: 0x8058 asks to show title {title_id}; titles are off");
        return Ok(true);
    }
    let points = player.titles.points(title_id);
    let rank = match rules::display_rank(definitions(), title_id, points) {
        Ok(rank) => rank,
        Err(why) => {
            eprintln!(
                "world: titles: {:?} may not show title {title_id} at {points} point(s): {why:?}",
                player.name
            );
            return Ok(true);
        }
    };
    player.titles.display = Some(title_id);
    if let Err(err) =
        db_titles::record_display_title(&world.pool, player.character, Some(title_id)).await
    {
        eprintln!("world: titles: could not store the shown title: {err}");
    }
    if trace {
        eprintln!(
            "world: titles: {:?} shows title {title_id}, rank {}",
            player.name, rank.rank_id
        );
    }
    player
        .instance
        .show_title(player.player_id, Some(rank.clone()));
    player
        .instance
        .broadcast(player.player_id, &binding_for_peer(player.player_id, rank));
    let mine = binding_for_self(player.player_id, rank, &mut player.titles);
    send(stream, session, mine, trace).await?;
    Ok(true)
}

/// `GAME_CMSG_TITLE_HIDE` (`0x8059`): the client asks to show none.
///
/// Stored, and the instance stops introducing this player with a title. **Nothing is sent back**: no
/// capture shows what the live server answers (doc 40, section 11), and the guess in section 10.3 - a
/// binding to rank 0 - names a record that may not exist, which is the crash rule. The live client
/// decides; this line is what to capture.
pub async fn handle_hide(world: &World, player: &mut Player) -> Result<bool, GameError> {
    if player.titles.display.take().is_some() {
        if let Err(err) = db_titles::record_display_title(&world.pool, player.character, None).await
        {
            eprintln!("world: titles: could not clear the shown title: {err}");
        }
    }
    player.instance.show_title(player.player_id, None);
    eprintln!(
        "world: titles: {:?} hides its title (0x8059); no reply sent - the live server's is not captured",
        player.name
    );
    Ok(true)
}

/// Add points to one title: [`set_points`] with the sum, saturating at the wire's `u32`.
///
/// The one entry an earning rule calls (Cartographer, Drunkard, ...).
pub async fn add_points(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    title_id: u32,
    delta: u32,
    trace: bool,
) -> Result<(), GameError> {
    let points = player.titles.points(title_id).saturating_add(delta);
    set_points(stream, session, world, player, title_id, points, trace).await
}

/// Give one title exactly `points`, store them, and tell the client in the way the rules decide: nothing,
/// a `0x00F5` patch, or the whole group again when the track is new to this connection or changed rung.
///
/// A title with no definition, or titles switched off, is refused with a log line and no write.
pub async fn set_points(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    title_id: u32,
    points: u32,
    trace: bool,
) -> Result<(), GameError> {
    let frames = change_points(world, player, title_id, points).await;
    send(stream, session, frames, trace).await
}

/// The change itself: the stored points and this connection's state move to `points`, and the frames
/// that tell the client are returned for the caller to deliver.
async fn change_points(
    world: &World,
    player: &mut Player,
    title_id: u32,
    points: u32,
) -> Vec<Frame> {
    let Some(title) = rules::find(definitions(), title_id) else {
        eprintln!("world: titles: title {title_id} is not defined; {points} point(s) ignored");
        return Vec::new();
    };
    let before = player.titles.points(title_id);
    if points == before {
        return Vec::new();
    }
    player.titles.points.insert(title_id, points);
    let owner = match title.scope {
        Scope::Character => db_titles::Owner::Character(player.character),
        Scope::Account => db_titles::Owner::Account(player.account_id),
    };
    if let Err(err) = db_titles::record_points(&world.pool, owner, title_id, points).await {
        eprintln!("world: titles: could not store title {title_id} at {points}: {err}");
    }
    let sent = player.titles.sent_tracks.contains(&title_id);
    let frames = match rules::point_change(title, before, points, sent) {
        PointChange::Nothing => Vec::new(),
        PointChange::Patch => vec![game::title::title_update(title_id, points)],
        PointChange::Resend => {
            group_frames(title, points, &mut player.titles).unwrap_or_else(|| {
                eprintln!(
                    "world: titles: title {title_id} at {points} has no track to send (a maxed \
                     track with no `maxed` record in config/Server/titles.json?)"
                );
                Vec::new()
            })
        }
    };
    eprintln!(
        "world: titles: {:?} title {title_id} {before} -> {points}, {} frame(s)",
        player.name,
        frames.len()
    );
    frames
}

/// How much of pre-Searing this grid has explored, in per-mille: its explored blocks inside
/// `cartography::pre_searing_union` over every block of that union. `None` for a grid of another
/// continent's shape, which the union's rectangles do not fit.
///
/// The union is the maps' whole world-map rectangles, ground nobody can walk on included, so a full walk
/// lands below 1000; the denominator is the cartography module's own definition of "all of pre-Searing".
pub fn pre_searing_exploration(explored: &crate::cartography::Grid) -> Option<u32> {
    let dims = explored.dims();
    if dims != crate::cartography::TYRIA {
        return None;
    }
    let union = crate::cartography::pre_searing_union(dims);
    let (mut inside, mut seen) = (0u64, 0u64);
    for y in 0..dims.1 {
        for x in 0..dims.0 {
            if union.is_explored(x, y) {
                inside += 1;
                if explored.is_explored(x, y) {
                    seen += 1;
                }
            }
        }
    }
    Some(rules::per_mille(seen, inside))
}

/// The titles earned by exploration, and the points the player's grid gives them now.
fn exploration_targets(player: &Player) -> Vec<(u32, u32)> {
    let Some(points) = pre_searing_exploration(&player.explored) else {
        return Vec::new();
    };
    definitions()
        .iter()
        .filter(|title| title.earned_by == Some(rules::Earning::PreSearingExploration))
        .map(|title| (title.title_id, points))
        .collect()
}

/// Before the load block: bring exploration-earned titles in line with the grid the connection read, so
/// the block carries the right points. Nothing is sent here; the block is what tells the client.
pub async fn sync_exploration(world: &World, player: &mut Player) {
    for (title_id, points) in exploration_targets(player) {
        if player.titles.points(title_id) != points {
            // Not sent yet on this connection, so the frames this returns are the block's to carry.
            let _ = change_points(world, player, title_id, points).await;
            player.titles.sent_tracks.remove(&title_id);
        }
    }
}

/// After a walk explored new blocks (`player::walked`): move the exploration-earned titles and deliver
/// the change through the player's own outbox, as the walk marks are.
pub async fn exploration_changed(world: &World, player: &mut Player) {
    for (title_id, points) in exploration_targets(player) {
        if player.titles.points(title_id) != points {
            let frames = change_points(world, player, title_id, points).await;
            if !frames.is_empty() {
                player.instance.send_to(player.player_id, &frames);
            }
        }
    }
}

/// The title a person means: a title id, or a `name` / `alias` from `config/Server/titles.json`,
/// compared without case, spaces or punctuation.
pub fn find_by_name<'a>(titles: &'a [Title], text: &str) -> Option<&'a Title> {
    if let Ok(title_id) = text.trim().parse::<u32>() {
        return rules::find(titles, title_id);
    }
    let key = |text: &str| -> String {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let wanted = key(text);
    titles
        .iter()
        .find(|title| title.names.iter().any(|name| key(name) == wanted))
}

/// `/title <title> <points>`: an ASURA test command, not the reference's (owner, 2026-11-05). Sets the
/// title's points through [`set_points`], so the client sees exactly what an earned change sends. The
/// answer is a log line, as every command answer here is.
pub async fn handle_title_command(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    argument: &str,
    trace: bool,
) -> Result<bool, GameError> {
    let usage = "Use: /title <name or id> <points>, e.g. /title ldoa 13";
    let Some((name, points)) = argument.trim().rsplit_once(' ') else {
        eprintln!("world: /title: {usage}");
        return Ok(true);
    };
    let Ok(points) = points.trim().parse::<u32>() else {
        eprintln!("world: /title: {points:?} is not a point count. {usage}");
        return Ok(true);
    };
    let Some(title) = find_by_name(definitions(), name) else {
        let known: Vec<String> = definitions()
            .iter()
            .map(|title| format!("{} ({})", title.names.join("/"), title.title_id))
            .collect();
        eprintln!(
            "world: /title: no title called {name:?}; defined: {}",
            if known.is_empty() {
                "none (titles are off)".to_string()
            } else {
                known.join(", ")
            }
        );
        return Ok(true);
    };
    set_points(
        stream,
        session,
        world,
        player,
        title.title_id,
        points,
        trace,
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(header: u16) -> Frame {
        Frame::new(header, Vec::new())
    }

    #[test]
    fn the_title_frames_land_after_their_anchors() {
        // The player block's own order: 0x003C, the attributes, ..., 0x0022, the party.
        let mut block = vec![
            frame(game::PLAYER_UPDATE_FLAGS),
            frame(game::AGENT_UPDATE_ATTRIBUTES),
            frame(game::WORLD_UPDATE_CONTROLLED_AGENT),
            frame(game::PARTY_CREATE),
        ];
        let groups = vec![frame(0x00F3), frame(0x00F3), frame(0x00F6)];
        assert!(splice_at_anchors(&mut block, groups, vec![frame(0x00F4)]));
        let headers: Vec<u16> = block.iter().map(Frame::header).collect();
        assert_eq!(
            headers,
            vec![
                game::PLAYER_UPDATE_FLAGS,
                0x00F3,
                0x00F3,
                0x00F6,
                game::AGENT_UPDATE_ATTRIBUTES,
                game::WORLD_UPDATE_CONTROLLED_AGENT,
                0x00F4,
                game::PARTY_CREATE,
            ]
        );
    }

    #[test]
    fn a_block_without_its_anchors_is_left_alone() {
        let mut block = vec![frame(game::PARTY_CREATE)];
        assert!(!splice_at_anchors(
            &mut block,
            vec![frame(0x00F6)],
            Vec::new()
        ));
        assert_eq!(block.len(), 1);
    }

    #[test]
    fn a_peer_binding_is_the_record_then_the_display() {
        let rank = Rank {
            rank_id: 0x98,
            flags: 0,
            value: 2,
            minimum: 0,
            name: vec![0x8101, 0x49CC, 0xE300, 0xE59E, 0x02AC],
        };
        let frames = binding_for_peer(0x35, &rank);
        let headers: Vec<u16> = frames.iter().map(Frame::header).collect();
        assert_eq!(headers, vec![0x00F3, 0x00F4, game::PLAYER_UPDATE_FLAGS]);
        assert_eq!(
            frames[1].encode(),
            vec![0xF4, 0x00, 0x35, 0x00, 0x98, 0x00, 0x00, 0x00],
            "pre-searing-load-3 seq 218: player 0x35 shows rank 0x98"
        );
    }

    #[test]
    fn a_title_is_found_by_id_name_or_alias() {
        let titles = vec![Title {
            title_id: 21,
            names: vec![
                "Legendary Defender of Ascalon".to_string(),
                "ldoa".to_string(),
            ],
            scope: Scope::Character,
            flags: 2,
            mode_flags: 0,
            base_rank_id: 177,
            progress_template: Vec::new(),
            description_template: Vec::new(),
            ranks: Vec::new(),
            maxed: None,
            display_from_rank: None,
            earned_by: None,
        }];
        for text in ["21", "ldoa", "LDoA", "legendary defender of ascalon"] {
            assert_eq!(
                find_by_name(&titles, text).map(|title| title.title_id),
                Some(21),
                "{text}"
            );
        }
        assert!(find_by_name(&titles, "drunkard").is_none());
        assert!(find_by_name(&titles, "7").is_none());
    }

    #[test]
    fn a_record_this_connection_has_is_not_sent_again() {
        let rank = Rank {
            rank_id: 177,
            flags: 0,
            value: 1,
            minimum: 20,
            name: vec![0x8101, 0x5A05, 0xA50B, 0x8B97, 0x2B8E],
        };
        let mut state = TitleState::default();
        assert_eq!(binding_for_self(1, &rank, &mut state).len(), 3);
        assert_eq!(binding_for_self(1, &rank, &mut state).len(), 2);
    }
}
