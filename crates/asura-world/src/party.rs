//! Parties in one instance: who leads each one, who is in it, which invitations are open, and how a
//! party leaves for another instance and arrives in it.
//!
//! Every player is in a party from the moment it joins an instance - a party of one, which the load block
//! tells the client about (`player::player_block`). An invitation is between two **parties**, and the
//! messages that carry it name the *other* party from the receiving client's point of view.
//!
//! What a live server sends for each request is measured in `docs/44-party-invites.md`, from
//! `recordings/client/feature/titles/titles-live-20261106-025940.jsonl` (the invited side: leave,
//! invitation, accept), `titles-live-20261106-032434.jsonl` (the inviting side: invitation, cancel,
//! decline) and `multiplay/party.jsonl` (the inviting side of an accept, and a party's load after a
//! doorway). Each batch below names the seq it copies. The pieces no capture shows are marked **not
//! captured** where they are sent.
//!
//! **Travel** (`docs/45-party-travel.md`, experimental). A party is the travel unit. The table lives here,
//! in the instance; what crosses to the next instance is the database's party ledger
//! (`asura_db::parties`, owner 2026-11-06: "Database ledger"), because the destination may be another
//! world process and a member that logs in again is placed by the auth service. A move writes the ledger
//! and tells every travelling member's connection to go ([`begin_move`], `travel::move_party`); the first
//! member to arrive builds the party in the destination from the ledger, with a reserved player id for
//! every member still on its way, which the load shows as a member not in the map (`party.jsonl` seq
//! 2662-2669: `0x01CC {1, 2, 0}`) ([`arrive`]). A member that never arrives is removed after the pack's
//! grace time.
//!
//! **Merges** (experimental). An invitation may name a party of more than one, and an accept may bring
//! one. The party that survives is the one whose leader leads the merged party (`asura_rules::party::
//! merge_leader`, the owner's rule); the other party's members join it. No merge is captured.
//!
//! **The explorable disconnect hold** (experimental). A member whose connection drops in an explorable
//! stays in its party, drawn grey (`0x01CE`), until the hold ends, the party changes areas, or it logs in
//! again and the auth service places it back here ([`left_instance`], `asura_rules::party::rejoin_held`).
//!
//! Parties change only in outposts (`asura_rules::party::may_change_party`): an explorable is one
//! party's own copy of the map, so every request there is refused.
//!
//! A party's size is capped per map by the world pack (`config/World/<pack>/party_sizes.json`): an
//! invitation and an accept that would take a party past it are refused, and so is every invitation on
//! a map the file does not list.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use asura_rules::party as rules;

use crate::*;

/// The file inside a pack that holds the party size of each map.
pub const SIZES_FILE: &str = "party_sizes.json";

/// The file inside a pack that holds how a party travels.
pub const TRAVEL_FILE: &str = "party_travel.json";

/// How many players a party may hold, per map id.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct PartySizes {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// Map id, written as a string because JSON keys are strings, to its size.
    pub maps: HashMap<String, crate::character_defaults::Value<u32>>,
}

impl PartySizes {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<PartySizes, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The most players a party may hold on `map_id`, or `None` when the file has no answer for it.
    pub fn max_size(&self, map_id: i32) -> Option<u32> {
        self.maps.get(&map_id.to_string()).map(|size| size.value)
    }
}

/// Read `<pack>/party_sizes.json`.
pub fn load_sizes(pack: &std::path::Path) -> Result<PartySizes, String> {
    let path = pack.join(SIZES_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    PartySizes::from_json(&text)
        .map_err(|err| format!("{} is not a party sizes file: {err}", path.display()))
}

/// The party sizes in force, read once per process. A missing or broken file is logged once and answers
/// `None` for every map, so no party grows anywhere.
pub fn sizes() -> Option<&'static PartySizes> {
    static IN_FORCE: std::sync::OnceLock<Option<PartySizes>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load_sizes(&pack) {
                Ok(sizes) => {
                    eprintln!("asura: party sizes from {}", pack.display());
                    Some(sizes)
                }
                Err(err) => {
                    eprintln!("asura: {err}; no party may grow");
                    None
                }
            }
        })
        .as_ref()
}

/// The most players a party may hold on a map, from the pack.
pub fn max_size(map_id: i32) -> Option<u32> {
    sizes().and_then(|sizes| sizes.max_size(map_id))
}

/// The most players a party may hold in this player's map.
fn max_size_here(player: &Player) -> Option<u32> {
    max_size(player.instance.map.map_id)
}

/// How a party travels: the pack's `party_travel.json`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct TravelSettings {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// Seconds between a leader's world-map travel and the party's move.
    pub countdown_secs: crate::character_defaults::Value<u16>,
    /// The byte after the seconds in `0x01BC` (a guess: no capture).
    pub countdown_byte: crate::character_defaults::Value<u8>,
    /// The text `0x01BC` carries (a guess: no capture).
    pub countdown_text: crate::character_defaults::Value<String>,
    /// Seconds a member told to travel has to arrive before the party drops it.
    pub arrival_grace_secs: crate::character_defaults::Value<u32>,
    /// Seconds an explorable party keeps the place of a member whose connection dropped.
    pub disconnect_hold_secs: crate::character_defaults::Value<u32>,
}

impl TravelSettings {
    /// Parse the file's text.
    pub fn from_json(text: &str) -> Result<TravelSettings, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Read `<pack>/party_travel.json`.
pub fn load_travel(pack: &std::path::Path) -> Result<TravelSettings, String> {
    let path = pack.join(TRAVEL_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    TravelSettings::from_json(&text)
        .map_err(|err| format!("{} is not a party travel file: {err}", path.display()))
}

/// The travel settings in force, read once per process. A missing or broken file is logged once and
/// answers `None`: a leader's countdown, an arrival's grace and a disconnect hold are then refused with
/// a logged reason, and the party does not move as one.
pub fn travel_settings() -> Option<&'static TravelSettings> {
    static IN_FORCE: std::sync::OnceLock<Option<TravelSettings>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load_travel(&pack) {
                Ok(settings) => {
                    eprintln!("asura: party travel from {}", pack.display());
                    Some(settings)
                }
                Err(err) => {
                    eprintln!("asura: {err}; parties do not travel as one");
                    None
                }
            }
        })
        .as_ref()
}

/// The party state a load ends with, after the player's own party (`CHARACTER_UPDATE_PARTY`) and before
/// `INSTANCE_LOAD_FINISH`. Every live load carries it, every value 0
/// (`titles-live-20261106-034507.jsonl` seq 847-861 for an outpost, `doors-20261031-095754.jsonl` seq
/// 322-324 for an explorable); a client given none of it keeps its own defaults, which drew the
/// difficulty toggle and the ready check in ASURA's Ascalon (owner, 2026-11-06).
///
/// - an outpost: `0x01E0 {0}`, `0x01BF {0}`, `0x01BE {0}`, the party search adverts, `0x01E0 {0}`;
///   this server has no party search, so it sends no adverts;
/// - an explorable: `0x01BF {0}`, `0x01BE {0}`.
pub fn load_tail(in_outpost: bool) -> Vec<Frame> {
    if in_outpost {
        vec![
            game::party_search_seek(0),
            game::party_set_difficulty(0),
            game::party_global_state_flags(0),
            game::party_search_seek(0),
        ]
    } else {
        vec![
            game::party_set_difficulty(0),
            game::party_global_state_flags(0),
        ]
    }
}

/// Put [`load_tail`] into a built players block, right before its `INSTANCE_LOAD_FINISH`. A block that
/// does not end with one is left as it is.
pub fn splice_load_tail(block: &mut Vec<Frame>, in_outpost: bool) {
    if block.last().map(Frame::header) != Some(game::INSTANCE_LOAD_FINISH) {
        return;
    }
    let at = block.len() - 1;
    block.splice(at..at, load_tail(in_outpost));
}

/// One party: its id, its leader and its members, all player ids. The leader is a member.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Party {
    /// The id every party message carries.
    pub party_id: u16,
    /// The player who leads it.
    pub leader: u32,
    /// Its players, in the order they joined; the leader first. A member that is not in the map yet
    /// (travelling in) or any more (held) is a member too, under the player id reserved for it.
    pub members: Vec<u32>,
    /// The members that are not in the map, by player id.
    pub absent: HashMap<u32, Absence>,
    /// The ledger move this party arrived with (`asura_db::parties`), if it arrived as a party.
    pub ledger: Option<Uuid>,
    /// A move is under way: its members are leaving, quietly, and nothing starts a second one.
    pub moving: bool,
    /// The ticket of a leader's countdown waiting to fire; a newer one, or a change of leader, voids it.
    pub countdown: Option<u64>,
    /// Members that have just arrived and whose arrival the others are told once their player exists
    /// on those clients (after the introduction): `true` when the member was held.
    pub entering: HashMap<u32, bool>,
}

/// Why a member is not in the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Absence {
    /// Held after a dropped connection (`true`), or still travelling in (`false`).
    pub held: bool,
    /// Which expiry may remove it: a later absence of the same member carries a newer number.
    pub epoch: u64,
}

impl Party {
    /// The members standing in the map, in order.
    pub fn present(&self) -> Vec<u32> {
        self.members
            .iter()
            .copied()
            .filter(|member| !self.absent.contains_key(member))
            .collect()
    }

    /// Whether a member is in the map.
    pub fn is_present(&self, member: u32) -> bool {
        self.members.contains(&member) && !self.absent.contains_key(&member)
    }
}

#[derive(Default)]
struct Table {
    parties: HashMap<u16, Party>,
    of_player: HashMap<u32, u16>,
    /// Open invitations, `(inviting party, invited party)`.
    invites: HashSet<(u16, u16)>,
    /// Every player's character, by player id - the players in the instance and the ids kept for members
    /// not in it: what the ledger names a member by.
    characters: HashMap<u32, Uuid>,
    /// The next absence epoch and countdown ticket.
    next_number: u64,
}

impl Table {
    /// The lowest free id. Live ids are arbitrary small numbers; the client keys its table by them and
    /// needs only that two parties never share one.
    fn free_id(&self) -> u16 {
        (1..=u16::MAX)
            .find(|id| !self.parties.contains_key(id))
            .unwrap_or(u16::MAX)
    }

    fn found(&mut self, player_id: u32) -> u16 {
        let party_id = self.free_id();
        self.parties.insert(
            party_id,
            Party {
                party_id,
                leader: player_id,
                members: vec![player_id],
                ..Party::default()
            },
        );
        self.of_player.insert(player_id, party_id);
        party_id
    }

    fn number(&mut self) -> u64 {
        self.next_number += 1;
        self.next_number
    }

    fn party_of(&self, player_id: u32) -> Option<&Party> {
        self.of_player
            .get(&player_id)
            .and_then(|party_id| self.parties.get(party_id))
    }

    /// Close every open invitation that names `party_id`, either way round.
    fn close_invites_of(&mut self, party_id: u16) -> (Vec<u16>, Vec<u16>) {
        let invited: Vec<u16> = self
            .invites
            .iter()
            .filter(|(from, _)| *from == party_id)
            .map(|(_, to)| *to)
            .collect();
        let invited_by: Vec<u16> = self
            .invites
            .iter()
            .filter(|(_, to)| *to == party_id)
            .map(|(from, _)| *from)
            .collect();
        self.invites
            .retain(|(from, to)| *from != party_id && *to != party_id);
        (invited, invited_by)
    }

    fn members(&self, party_id: u16) -> Vec<u32> {
        self.parties
            .get(&party_id)
            .map(|party| party.members.clone())
            .unwrap_or_default()
    }

    /// Take a member out of its party without a word to anyone: what a member of a moving party is
    /// when its connection goes. The party goes with its last member.
    fn drop_quietly(&mut self, party_id: u16, member: u32) {
        let gone = match self.parties.get_mut(&party_id) {
            Some(party) => {
                party.members.retain(|m| *m != member);
                party.absent.remove(&member);
                party.entering.remove(&member);
                if party.leader == member {
                    if let Some(next) = party.members.first() {
                        party.leader = *next;
                    }
                }
                party.members.is_empty()
            }
            None => false,
        };
        if self.of_player.get(&member) == Some(&party_id) {
            self.of_player.remove(&member);
        }
        self.characters.remove(&member);
        if gone {
            self.parties.remove(&party_id);
            self.close_invites_of(party_id);
        }
    }
}

/// The parties of one instance.
#[derive(Default)]
pub struct Parties {
    table: Mutex<Table>,
}

impl Parties {
    fn locked(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Put a player that has just joined the instance in a party of its own, and return the party's id.
    pub fn found(&self, player_id: u32) -> u16 {
        self.locked().found(player_id)
    }

    /// The party a player is in, as a copy.
    pub fn party_of(&self, player_id: u32) -> Option<Party> {
        self.locked().party_of(player_id).cloned()
    }

    /// Who leads a player's party, and how many players are in it: the two numbers `0x00B0` and
    /// `0x00B1` carry. A player the table does not know is a party of one.
    pub fn leader_and_size(&self, player_id: u32) -> (u32, u32) {
        self.locked()
            .party_of(player_id)
            .map(|party| (party.leader, party.members.len() as u32))
            .unwrap_or((player_id, 1))
    }

    /// Whether a player's party is leaving for another instance.
    pub fn is_moving(&self, player_id: u32) -> bool {
        self.locked()
            .party_of(player_id)
            .is_some_and(|party| party.moving)
    }
}

// ---- the frames ---------------------------------------------------------------------------------

fn id(player_id: u32) -> u16 {
    player_id as u16
}

/// A party as a client is told it: create, one add per member, end. The add's flag is 1 for a member
/// standing in the map (`titles-live-20261106-025940.jsonl` seq 2539-2541) and 0 for one that is not
/// (`multiplay/party.jsonl` seq 2667, the second member of a party that has just come through a doorway).
fn member_stream(party: &Party) -> Vec<Frame> {
    member_stream_where(party, |member| !party.absent.contains_key(&member))
}

/// [`member_stream`] with the flag decided by `in_map`.
fn member_stream_where(party: &Party, in_map: impl Fn(u32) -> bool) -> Vec<Frame> {
    let mut frames = vec![game::party_create(party.party_id)];
    for member in &party.members {
        frames.push(game::party_player_add(
            party.party_id,
            id(*member),
            u8::from(in_map(*member)),
        ));
    }
    frames.push(game::party_member_stream_end(party.party_id));
    frames
}

/// The size and leader pair everyone in the map is told when a party changes: `0x00B0` names the leader
/// and the size, `0x00B1` names the player and its leader.
fn size_and_leader(leader: u32, size: u32, player_id: u32) -> [Frame; 2] {
    [
        game::player_update_party_size(leader, size),
        game::player_update_party(player_id, leader),
    ]
}

/// The party part of a player's own load: the leader and size, each member's leader, the member
/// stream, and `0x01B3` with 1 when the player leads it. A live load after a doorway
/// (`multiplay/party.jsonl` seq 2662-2669): `0x00B0 {1, 2}`, `0x00B1 {1, 1}`, `0x00B1 {2, 1}`, `0x01D3
/// {1}`, `0x01CC {1, 1, 1}`, `0x01CC {1, 2, 0}`, `0x01D4 {1}`, `0x01B3 {1, 1}`; a party of one is the
/// same shape (`multiplay/party interface check.jsonl` seq 588-591).
///
/// In its own load a client sees only itself in the map (flag 1). Every other member is sent with 0
/// and, once its introduction is in the same block, with `0x01CF` ([`in_map_notices`]): a member sent
/// with 1 before its `PLAYER_CREATE` is what dropped the second client of a party arriving in Barradin
/// Estate (`logs/world-8001.log`, 2026-11-06), while the first, sent its absent leader with 0, stayed.
pub fn load_section(party: &Party, me: u32) -> Vec<Frame> {
    let size = party.members.len() as u32;
    let mut frames = vec![game::player_update_party_size(party.leader, size)];
    for member in &party.members {
        frames.push(game::player_update_party(*member, party.leader));
    }
    frames.extend(member_stream_where(party, |member| member == me));
    frames.push(game::character_update_party(
        party.party_id,
        u8::from(party.leader == me),
    ));
    frames
}

/// Replace the player block's own one-member party (`PARTY_CREATE` through `CHARACTER_UPDATE_PARTY`)
/// with this player's party as the instance has it ([`load_section`]). Called after the guild block is
/// in, so the guild stays in front of the party. A block without that run is left as it is.
pub fn splice_load_section(block: &mut Vec<Frame>, instance: &Instance, me: u32) {
    let Some(party) = instance.parties.party_of(me) else {
        return;
    };
    let Some(start) = block
        .iter()
        .position(|frame| frame.header() == game::PARTY_CREATE)
    else {
        return;
    };
    let Some(end) = block[start..]
        .iter()
        .position(|frame| frame.header() == game::CHARACTER_UPDATE_PARTY)
        .map(|offset| start + offset)
    else {
        return;
    };
    block.splice(start..=end, load_section(&party, me));
}

/// What a loading client is told, after the introductions in its block, about the members of its party
/// already in the map: `0x01CF {party, member}` for each one it was introduced to (`entered`). **Not
/// captured**; a client in Barradin Estate took it without complaint (2026-11-06).
pub fn in_map_notices(instance: &Instance, me: u32, entered: &[u32]) -> Vec<Frame> {
    let Some(party) = instance.parties.party_of(me) else {
        return Vec::new();
    };
    party
        .present()
        .into_iter()
        .filter(|member| *member != me && entered.contains(member))
        .map(|member| game::party_member_enter_world(party.party_id, id(member)))
        .collect()
}

/// Frames for some players, delivered after the table's lock is released. The lock order is fixed: the
/// party table's lock may be held while the instance's slot lock is taken (`bystanders`, a reservation),
/// never the other way round - `Instance::others` releases the slots before it reads the table.
type Plan = Vec<(u32, Vec<Frame>)>;

fn deliver(instance: &Instance, plan: Plan) {
    for (player_id, frames) in plan {
        instance.send_to(player_id, &frames);
    }
}

fn to_all(players: &[u32], frames: &[Frame], plan: &mut Plan) {
    for player_id in players {
        plan.push((*player_id, frames.to_vec()));
    }
}

/// Everyone in the instance who is in none of `parties`.
fn bystanders(instance: &Instance, involved: &[u32]) -> Vec<u32> {
    instance
        .player_ids()
        .into_iter()
        .filter(|player_id| !involved.contains(player_id))
        .collect()
}

fn read_word(frame: &Frame) -> Result<u16, GameError> {
    game::read_party_word(frame.body())
        .ok_or(GameError::Framing(FrameError::Incomplete { needed: 4 }))
}

fn refused(player: &Player, what: &str, why: impl std::fmt::Debug) -> Result<bool, GameError> {
    eprintln!(
        "world: party: {:?} {what}: refused ({why:?}); nothing sent",
        player.name
    );
    Ok(true)
}

/// The outpost rule, asked first by every request.
fn in_outpost(player: &Player) -> Result<(), rules::Refused> {
    rules::may_change_party(player.instance.map.map_type == map_kinds().main_town)
}

// ---- the requests -------------------------------------------------------------------------------

/// `GAME_CMSG_PARTY_INVITE_PLAYER` (`0x80A0`, one word: the invited player's id).
///
/// The inviter is shown the invited party and then the open invitation (`032434` seq 380-384); the
/// invited is shown the inviting party and then the invitation (`025940` seq 2539-2542). The invited
/// party may hold more than one player: accepting is then a merge (experimental, not captured).
pub fn handle_invite(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    if let Err(why) = in_outpost(player) {
        return refused(player, "invites", why);
    }
    let target = u32::from(read_word(frame)?);
    let me = player.player_id;
    let instance = &player.instance;
    let plan = {
        let mut table = instance.parties.locked();
        let (Some(mine), Some(theirs)) =
            (table.party_of(me).cloned(), table.party_of(target).cloned())
        else {
            return refused(player, "invites a player not in this instance", target);
        };
        if let Err(why) = rules::may_invite(rules::Invitation {
            inviter_leads: mine.leader == me,
            same_party: mine.party_id == theirs.party_id,
            already_invited: table.invites.contains(&(mine.party_id, theirs.party_id)),
        }) {
            return refused(player, "invites", why);
        }
        if let Err(why) = rules::may_fill(
            mine.members.len() + theirs.members.len(),
            max_size_here(player),
        ) {
            return refused(player, "invites", why);
        }
        table.invites.insert((mine.party_id, theirs.party_id));
        let mut plan = Plan::new();
        let mut inviters = member_stream(&theirs);
        inviters.push(game::party_invite_add(theirs.party_id));
        to_all(&mine.members, &inviters, &mut plan);
        let mut invited = member_stream(&mine);
        invited.push(game::party_join_request(mine.party_id));
        to_all(&theirs.members, &invited, &mut plan);
        plan
    };
    if trace {
        eprintln!("world: party: player {me} invites player {target}");
    }
    deliver(instance, plan);
    Ok(true)
}

/// `GAME_CMSG_PARTY_ACCEPT_CANCEL` (`0x809D`, one word: the invited party). The inviter withdraws.
///
/// The inviter gets `0x01C7` alone (`032434` seq 1295-1300). The invited side's `0x01C8` is **not
/// captured**.
pub fn handle_cancel(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    if let Err(why) = in_outpost(player) {
        return refused(player, "cancels", why);
    }
    let invited = read_word(frame)?;
    let me = player.player_id;
    let instance = &player.instance;
    let plan = {
        let mut table = instance.parties.locked();
        let Some(mine) = table.party_of(me).cloned() else {
            return refused(player, "cancels", "no party");
        };
        let open = table.invites.contains(&(mine.party_id, invited));
        if let Err(why) = rules::may_answer(mine.leader == me, open) {
            return refused(player, "cancels", why);
        }
        table.invites.remove(&(mine.party_id, invited));
        let mut plan = Plan::new();
        to_all(
            &mine.members,
            &[game::party_invite_cancel(invited)],
            &mut plan,
        );
        to_all(
            &table.members(invited),
            &[game::party_request_cancel(mine.party_id)],
            &mut plan,
        );
        plan
    };
    if trace {
        eprintln!("world: party: player {me} withdraws the invitation of party {invited}");
    }
    deliver(instance, plan);
    Ok(true)
}

/// `GAME_CMSG_PARTY_ACCEPT_REFUSE` (`0x809E`, one word: the inviting party). The invited declines.
///
/// The inviter gets `0x01CA` alone (`032434` seq 2058). The invited side's `0x01C9` alone is **not
/// captured**; it is the first message of the accept (`025940` seq 2622).
pub fn handle_refuse(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    if let Err(why) = in_outpost(player) {
        return refused(player, "declines", why);
    }
    let inviting = read_word(frame)?;
    let me = player.player_id;
    let instance = &player.instance;
    let plan = {
        let mut table = instance.parties.locked();
        let Some(mine) = table.party_of(me).cloned() else {
            return refused(player, "declines", "no party");
        };
        let open = table.invites.contains(&(inviting, mine.party_id));
        if let Err(why) = rules::may_answer(mine.leader == me, open) {
            return refused(player, "declines", why);
        }
        table.invites.remove(&(inviting, mine.party_id));
        let mut plan = Plan::new();
        to_all(
            &mine.members,
            &[game::party_request_response(inviting)],
            &mut plan,
        );
        to_all(
            &table.members(inviting),
            &[game::party_invite_response(mine.party_id)],
            &mut plan,
        );
        plan
    };
    if trace {
        eprintln!("world: party: player {me} declines the invitation of party {inviting}");
    }
    deliver(instance, plan);
    Ok(true)
}

/// `GAME_CMSG_PARTY_ACCEPT_INVITE` (`0x809C`, one word: the inviting party). The invited party joins it.
///
/// A party of one joining (measured):
///
/// - the one joining: `0x01C9`, its add, the size and leader pair, its flags word, then its new party
///   (`0x01B3` with 0: not the leader), `0x01E0 {0}`, `0x01BF {0}` (`025940` seq 2622-2638);
/// - the party it joins: `0x01CA`, the add, the pair, the flags word (`multiplay/party.jsonl`
///   seq 160-165);
/// - everyone else in the map: the pair, which every capture carries for parties the client is not in.
///
/// Two parties of more than one merging (experimental, **not captured**): the party whose leader leads
/// the merge survives (`rules::merge_leader`) and every member of the other moves into it. Each mover
/// gets its answer (`0x01C9` or `0x01CA`), the surviving party's whole member stream, the pairs, its
/// flags word and `0x01B3 {party, 0}`, `0x01E0 {0}`, `0x01BF {0}`; the surviving members get the answer
/// once, then one add, pair and flags word per mover.
///
/// The live batches also carry `0x003C` for four or five other players with values that did not change;
/// which players those are is not established, so only the one joining is sent. Its value is the 0
/// this server's load sends for every player. Other invitations the joining party had open are closed
/// first, the way `titles-live-20261106-030218.jsonl` seq 203-204 closes two before an add.
pub fn handle_accept(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    if let Err(why) = in_outpost(player) {
        return refused(player, "accepts", why);
    }
    let inviting = read_word(frame)?;
    let me = player.player_id;
    let instance = &player.instance;
    let (plan, survivor) = {
        let mut table = instance.parties.locked();
        let Some(mine) = table.party_of(me).cloned() else {
            return refused(player, "accepts", "no party");
        };
        let open = table.invites.contains(&(inviting, mine.party_id));
        if let Err(why) = rules::may_answer(mine.leader == me, open) {
            return refused(player, "accepts", why);
        }
        let Some(theirs) = table.parties.get(&inviting).cloned() else {
            return refused(player, "accepts", "the inviting party is gone");
        };
        if theirs.moving || mine.moving {
            return refused(player, "accepts", "a party is leaving the map");
        }
        if let Err(why) = rules::may_fill(
            theirs.members.len() + mine.members.len(),
            max_size_here(player),
        ) {
            return refused(player, "accepts", why);
        }
        let mut plan = Plan::new();
        // Every other invitation either party had open, either way round, is closed with it. The
        // accepting party's are closed by its own answer; the inviting party's other invitations stand.
        let (invited, invited_by) = table.close_invites_of(mine.party_id);
        for other in invited.into_iter().filter(|other| *other != inviting) {
            to_all(
                &mine.members,
                &[game::party_invite_cancel(other)],
                &mut plan,
            );
            to_all(
                &table.members(other),
                &[game::party_request_cancel(mine.party_id)],
                &mut plan,
            );
        }
        for other in invited_by.into_iter().filter(|other| *other != inviting) {
            to_all(
                &mine.members,
                &[game::party_request_response(other)],
                &mut plan,
            );
            to_all(
                &table.members(other),
                &[game::party_invite_response(mine.party_id)],
                &mut plan,
            );
        }
        table.invites.remove(&(inviting, mine.party_id));

        // The merge: who leads, and so which party survives.
        let (keep, moving_in, movers_answer, keepers_answer) =
            match rules::merge_leader(theirs.members.len(), mine.members.len()) {
                rules::MergeLeader::Inviting => (
                    theirs,
                    mine.clone(),
                    game::party_request_response(inviting),
                    game::party_invite_response(mine.party_id),
                ),
                rules::MergeLeader::Invited => (
                    mine.clone(),
                    theirs,
                    game::party_invite_response(mine.party_id),
                    game::party_request_response(inviting),
                ),
            };
        table.parties.remove(&moving_in.party_id);
        table.close_invites_of(moving_in.party_id);
        let party = {
            let party = table
                .parties
                .get_mut(&keep.party_id)
                .expect("read above, under the same lock");
            for mover in &moving_in.members {
                party.members.push(*mover);
                if let Some(absence) = moving_in.absent.get(mover) {
                    party.absent.insert(*mover, *absence);
                }
            }
            party.countdown = None;
            party.clone()
        };
        for mover in &moving_in.members {
            table.of_player.insert(*mover, keep.party_id);
        }

        let size = party.members.len() as u32;
        let movers = &moving_in.members;
        // The movers: the answer, then - for one player - its own add as the capture has it, or for a
        // party the whole surviving stream, then each mover's pair and flags word, then their new party.
        let mut moving_batch = vec![movers_answer];
        if movers.len() == 1 && keep.members.len() == 1 {
            moving_batch.push(game::party_player_add(keep.party_id, id(movers[0]), 1));
        } else {
            moving_batch.extend(member_stream(&party));
        }
        let mut keeping_batch = vec![keepers_answer];
        for mover in movers {
            let pair = size_and_leader(party.leader, size, *mover);
            let flags = game::player_update_flags(id(*mover), 0, 7);
            moving_batch.extend(pair.clone());
            moving_batch.push(flags.clone());
            keeping_batch.push(game::party_player_add(
                keep.party_id,
                id(*mover),
                u8::from(!party.absent.contains_key(mover)),
            ));
            keeping_batch.extend(pair.clone());
            keeping_batch.push(flags);
        }
        moving_batch.push(game::character_update_party(keep.party_id, 0));
        moving_batch.push(game::party_search_seek(0));
        moving_batch.push(game::party_set_difficulty(0));
        to_all(movers, &moving_batch, &mut plan);
        to_all(&keep.members, &keeping_batch, &mut plan);

        let mut pairs = Vec::new();
        for mover in movers {
            pairs.extend(size_and_leader(party.leader, size, *mover));
        }
        to_all(&bystanders(instance, &party.members), &pairs, &mut plan);
        (plan, keep.party_id)
    };
    if trace {
        eprintln!(
            "world: party: player {me}'s party joins party {inviting}; party {survivor} goes on"
        );
    }
    deliver(instance, plan);
    Ok(true)
}

/// `GAME_CMSG_PARTY_LEAVE_GROUP` (`0x80A2`, no fields). A player leaves its party and is put in a party
/// of its own; a leader that leaves hands the party to the next member (`034507` seq 3304-3346: party
/// `0x29` goes on under player `0x0E`, who is invited by its id at seq 3427).
pub fn handle_leave(player: &Player, trace: bool) -> Result<bool, GameError> {
    if let Err(why) = in_outpost(player) {
        return refused(player, "leaves", why);
    }
    let me = player.player_id;
    let instance = &player.instance;
    let plan = {
        let mut table = instance.parties.locked();
        let Some(old) = table.party_of(me).cloned() else {
            return refused(player, "leaves", "no party");
        };
        if let Err(why) = rules::may_leave(old.members.len()) {
            return refused(player, "leaves", why);
        }
        departure(&mut table, instance, &old, me)
    };
    if trace {
        eprintln!("world: party: player {me} leaves its party");
    }
    deliver(instance, plan);
    Ok(true)
}

/// A player leaves its party in order to travel alone (`rules::TravelRequest::LeavesParty`): the same
/// batch as [`handle_leave`], in an outpost or an explorable. Returns whether it had company to leave.
pub fn leave_to_travel(player: &Player, trace: bool) -> bool {
    let me = player.player_id;
    let instance = &player.instance;
    let plan = {
        let mut table = instance.parties.locked();
        let Some(old) = table.party_of(me).cloned() else {
            return false;
        };
        if old.members.len() <= 1 {
            return false;
        }
        departure(&mut table, instance, &old, me)
    };
    if trace {
        eprintln!("world: party: player {me} leaves its party to travel alone");
    }
    deliver(instance, plan);
    true
}

/// `GAME_CMSG_PARTY_KICK_PLAYER` (`0x80A9`, one word: the player). The leader puts a member out.
///
/// The leader's batch is measured (`034507` seq 2990-3001). The kicked player's is **not captured**; it
/// is sent the batch of a member that leaves.
pub fn handle_kick(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    if let Err(why) = in_outpost(player) {
        return refused(player, "kicks", why);
    }
    let target = u32::from(read_word(frame)?);
    let me = player.player_id;
    let instance = &player.instance;
    let plan = {
        let mut table = instance.parties.locked();
        let Some(mine) = table.party_of(me).cloned() else {
            return refused(player, "kicks", "no party");
        };
        if let Err(why) = rules::may_kick(
            mine.leader == me,
            mine.members.contains(&target),
            target == me,
        ) {
            return refused(player, "kicks", why);
        }
        departure(&mut table, instance, &mine, target)
    };
    if trace {
        eprintln!("world: party: player {me} kicks player {target}");
    }
    deliver(instance, plan);
    Ok(true)
}

/// One player goes from `old`: a leave, a kick, or a departure from the map. Returns every client's
/// frames; the one going is put in a new party of its own.
///
/// - the one going (`025940` seq 2437-2458 for a member, `034507` seq 3309-3346 for the leader): the
///   remove, then - if it led the party - the handover to the next member (`0x00B0 {new, n}`,
///   `0x00B1 {it, new}`, `0x00B1 {new, new}`), the party's new size, its own pair twice around its
///   flags word, the stream of its new party, `0x01B3` with 1, `0x01E0 {0}`, `0x01BF {0}`;
/// - the members staying (`034507` seq 2994-3001, the leader's side of a kick): the remove, the
///   handover if there is one, the new size, then the one going's pair twice around its flags word;
///   a new leader then gets `0x01CB {1}` (`025940` seq 627-631);
/// - everyone else in the map: the pairs.
///
/// The flags word goes out with this server's value 0; the live batches carry the player's own value
/// (`0x003C {0x0E, 4, 7}` at seq 2999) and, for the one going, four or five other players' as well,
/// which are not sent (see `handle_accept`).
///
/// A leader's countdown is voided when its leader goes. A member that is not in the map (travelling in,
/// or held) can be the leader; the next member in the map takes over then, the way it does when the
/// leader leaves.
fn departure(table: &mut Table, instance: &Instance, old: &Party, going: u32) -> Plan {
    let remaining: Vec<u32> = old
        .members
        .iter()
        .copied()
        .filter(|m| *m != going)
        .collect();
    let size_before = old.members.len() as u32;
    let size = remaining.len() as u32;
    let new_leader = if old.leader == going {
        remaining
            .iter()
            .copied()
            .find(|m| !old.absent.contains_key(m))
            .or_else(|| remaining.first().copied())
    } else {
        Some(old.leader)
    };
    if let Some(party) = table.parties.get_mut(&old.party_id) {
        party.members.retain(|member| *member != going);
        party.absent.remove(&going);
        party.entering.remove(&going);
        if let Some(leader) = new_leader {
            if party.leader != leader {
                party.countdown = None;
            }
            party.leader = leader;
            // The leader stands first, the order every handover reads.
            if let Some(at) = party.members.iter().position(|m| *m == leader) {
                let leader = party.members.remove(at);
                party.members.insert(0, leader);
            }
        }
    }
    if remaining.is_empty() {
        table.parties.remove(&old.party_id);
    }

    let fresh = table.found(going);
    let new_party = Party {
        party_id: fresh,
        leader: going,
        members: vec![going],
        ..Party::default()
    };

    let remove = game::party_player_remove(old.party_id, id(going));
    let mut handover = Vec::new();
    if let (true, Some(leader)) = (old.leader == going, new_leader) {
        handover.push(game::player_update_party_size(leader, size_before));
        handover.push(game::player_update_party(going, leader));
        handover.push(game::player_update_party(leader, leader));
    }
    let new_size = game::player_update_party_size(new_leader.unwrap_or(going), size);
    let own = size_and_leader(going, 1, going);
    let flags = game::player_update_flags(id(going), 0, 7);

    let mut leaving = vec![remove.clone()];
    leaving.extend(handover.iter().cloned());
    leaving.push(new_size.clone());
    leaving.extend(own.clone());
    leaving.push(flags.clone());
    leaving.extend(own.clone());
    leaving.extend(member_stream(&new_party));
    leaving.push(game::character_update_party(fresh, 1));
    leaving.push(game::party_search_seek(0));
    leaving.push(game::party_set_difficulty(0));

    let mut staying = vec![remove];
    staying.extend(handover.iter().cloned());
    staying.push(new_size.clone());
    staying.extend(own.clone());
    staying.push(flags);
    staying.extend(own.clone());

    let mut plan = Plan::new();
    plan.push((going, leaving));
    to_all(&remaining, &staying, &mut plan);
    if let (true, Some(leader)) = (old.leader == going, new_leader) {
        plan.push((leader, vec![game::party_you_are_leader(1)]));
    }
    let mut pairs = handover;
    pairs.push(new_size);
    pairs.extend(own);
    to_all(&bystanders(instance, &old.members), &pairs, &mut plan);
    plan
}

// ---- travel: arriving ---------------------------------------------------------------------------

/// A member whose arrival the party waits for, and how long it waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Awaited {
    /// The party it was reserved in.
    pub party_id: u16,
    /// The player id reserved for it.
    pub player_id: u32,
    /// Its character.
    pub character: Uuid,
    /// The absence's number, which [`expire`] checks.
    pub epoch: u64,
    /// Held after a drop (`true`), or travelling in.
    pub held: bool,
}

/// Put a player that has just joined the instance in its party.
///
/// `ledger` is the character's travelling party when this join is that party's arrival here (the
/// caller checked the map, the world and the member's state). Without one the player is a party of
/// one, as every join was before parties travelled.
///
/// With one: the first member to arrive builds the party from the ledger - every other member still
/// travelling, or held, gets a reserved player id and stands in the party as not in the map - and every
/// later member takes its reserved place. The members already here are told once its player exists on
/// their clients ([`announce_arrival`]). Returns the members the party now waits for, so the caller can
/// give each its expiry ([`expire`]).
pub fn arrive(
    instance: &Instance,
    player_id: u32,
    character: Uuid,
    ledger: Option<&asura_db::parties::Membership>,
    trace: bool,
) -> Vec<Awaited> {
    let mut table = instance.parties.locked();
    table.characters.insert(player_id, character);
    let Some(ledger) = ledger else {
        table.found(player_id);
        return Vec::new();
    };
    let existing = table
        .parties
        .values()
        .find(|party| party.ledger == Some(ledger.party_id))
        .map(|party| party.party_id);
    if let Some(party_id) = existing {
        // Its place: the id kept for it, or - when it joined before the first member kept one - the id
        // it joined with, put where the kept one stood.
        let kept = table.parties.get(&party_id).and_then(|party| {
            party.members.iter().copied().find(|member| {
                *member != player_id && table.characters.get(member) == Some(&character)
            })
        });
        if let Some(old) = kept {
            table.characters.remove(&old);
            table.of_player.remove(&old);
        }
        let party = table
            .parties
            .get_mut(&party_id)
            .expect("found above, under the same lock");
        let held = match kept {
            Some(old) => {
                if let Some(at) = party.members.iter().position(|m| *m == old) {
                    party.members[at] = player_id;
                }
                if party.leader == old {
                    party.leader = player_id;
                }
                party.entering.remove(&old);
                let held = party.absent.remove(&old).is_some_and(|a| a.held);
                instance.release(character);
                held
            }
            None if party.members.contains(&player_id) => party
                .absent
                .remove(&player_id)
                .is_some_and(|absence| absence.held),
            None => {
                party.members.push(player_id);
                false
            }
        };
        party.entering.insert(player_id, held);
        table.of_player.insert(player_id, party_id);
        if trace {
            eprintln!(
                "world: party: player {player_id} arrives in party {party_id}{}",
                if held { " again after a drop" } else { "" }
            );
        }
        return Vec::new();
    }

    // The first to arrive: the party as the ledger has it. A member already in the map in a party of its
    // own (it arrived alone) is not taken; one still travelling, or held, waits under a kept id.
    let party_id = table.free_id();
    let mut party = Party {
        party_id,
        leader: player_id,
        ledger: Some(ledger.party_id),
        ..Party::default()
    };
    let mut awaited = Vec::new();
    for member in &ledger.members {
        if member.character_id == character {
            party.members.push(player_id);
            continue;
        }
        let travelling = member.state == asura_db::parties::TRAVELLING;
        let held = member.state == asura_db::parties::HELD && member.hold_live;
        if !travelling && !held {
            continue;
        }
        let kept = instance.reserve(member.character_id);
        let epoch = table.number();
        table.characters.insert(kept, member.character_id);
        table.of_player.insert(kept, party_id);
        party.members.push(kept);
        party.absent.insert(kept, Absence { held, epoch });
        if member.character_id == ledger.leader {
            party.leader = kept;
        }
        awaited.push(Awaited {
            party_id,
            player_id: kept,
            character: member.character_id,
            epoch,
            held,
        });
    }
    if !party.members.contains(&player_id) {
        party.members.push(player_id);
    }
    // The leader stands first, the order every handover reads.
    if let Some(at) = party.members.iter().position(|m| *m == party.leader) {
        let leader = party.members.remove(at);
        party.members.insert(0, leader);
    }
    if trace {
        eprintln!(
            "world: party: player {player_id} is the first of its party to arrive; party {party_id} \
             holds {} member(s), {} still on the way",
            party.members.len(),
            awaited.len()
        );
    }
    table.parties.insert(party_id, party);
    table.of_player.insert(player_id, party_id);
    awaited
}

/// Tell the members already in the map that a member has entered, once its player exists on their
/// clients (after its introduction): `0x01CF {party, player}` (ledger E38 `MemberEnterWorld`), and for a
/// member that was held, `0x01CD {party, player}` first (`MemberConnectSoft`). **Not captured**. Only
/// members in `introduced` - the players that entered before it and were shown it as not in the map
/// (`load_section`) - are told; one that enters after it sees it in the map in its own load.
pub fn announce_arrival(instance: &Instance, me: u32, introduced: &[u32], trace: bool) {
    let plan = {
        let mut table = instance.parties.locked();
        let Some(party_id) = table.of_player.get(&me).copied() else {
            return;
        };
        let Some(party) = table.parties.get_mut(&party_id) else {
            return;
        };
        let held = party.entering.remove(&me).unwrap_or(false);
        if party.members.len() <= 1 {
            return;
        }
        let mut frames = Vec::new();
        if held {
            frames.push(game::party_member_connect_soft(party_id, id(me)));
        }
        frames.push(game::party_member_enter_world(party_id, id(me)));
        let others: Vec<u32> = party
            .present()
            .into_iter()
            .filter(|m| *m != me && introduced.contains(m))
            .collect();
        let mut plan = Plan::new();
        to_all(&others, &frames, &mut plan);
        plan
    };
    if trace && !plan.is_empty() {
        eprintln!("world: party: player {me}'s party is told it is in the map");
    }
    deliver(instance, plan);
}

/// A member the party waited for has not come: remove it, the way a leave removes a member, unless it
/// arrived (or was held again) since. Returns whether it was removed, so the caller forgets it in the
/// ledger.
pub fn expire(instance: &Instance, awaited: Awaited, trace: bool) -> bool {
    let plan = {
        let mut table = instance.parties.locked();
        let Some(party) = table.parties.get(&awaited.party_id).cloned() else {
            return false;
        };
        if party.absent.get(&awaited.player_id).map(|a| a.epoch) != Some(awaited.epoch) {
            return false;
        }
        let gone = awaited.player_id;
        let remaining: Vec<u32> = party
            .members
            .iter()
            .copied()
            .filter(|m| *m != gone)
            .collect();
        let new_leader = if party.leader == gone {
            remaining
                .iter()
                .copied()
                .find(|m| !party.absent.contains_key(m))
                .or_else(|| remaining.first().copied())
        } else {
            Some(party.leader)
        };
        table.drop_quietly(awaited.party_id, gone);
        let mut plan = Plan::new();
        if let Some(leader) = new_leader {
            if let Some(kept) = table.parties.get_mut(&awaited.party_id) {
                if kept.leader != leader {
                    kept.countdown = None;
                }
                kept.leader = leader;
                if let Some(at) = kept.members.iter().position(|m| *m == leader) {
                    let first = kept.members.remove(at);
                    kept.members.insert(0, first);
                }
            }
            // The members in the map: the remove and the new size (the leader's side of a kick, `034507`
            // seq 2994-2995); a new leader is told so (`025940` seq 627-631). The member never stood in
            // the map, so it has no pair of its own to send.
            let mut frames = vec![game::party_player_remove(awaited.party_id, id(gone))];
            if party.leader == gone {
                frames.push(game::player_update_party(leader, leader));
            }
            frames.push(game::player_update_party_size(
                leader,
                remaining.len() as u32,
            ));
            let present: Vec<u32> = remaining
                .iter()
                .copied()
                .filter(|m| !party.absent.contains_key(m))
                .collect();
            to_all(&present, &frames, &mut plan);
            if party.leader == gone && present.contains(&leader) {
                plan.push((leader, vec![game::party_you_are_leader(1)]));
            }
            to_all(
                &bystanders(instance, &party.members),
                &[game::player_update_party_size(
                    leader,
                    remaining.len() as u32,
                )],
                &mut plan,
            );
        }
        plan
    };
    instance.release(awaited.character);
    if trace {
        eprintln!(
            "world: party: player {} of party {} did not {}; it is out of the party",
            awaited.player_id,
            awaited.party_id,
            if awaited.held {
                "come back before its hold ended"
            } else {
                "arrive in time"
            }
        );
    }
    deliver(instance, plan);
    true
}

/// Give a member the party waits for its expiry: the pack's arrival grace for one travelling in, its
/// disconnect hold for one held. When it runs out and the member has not come, it is removed
/// ([`expire`]) and forgotten in the ledger. A pack with no travel file gives no expiry, and says so.
pub fn expire_later(
    instance: std::sync::Arc<Instance>,
    pool: PgPool,
    awaited: Awaited,
    trace: bool,
) {
    let Some(settings) = travel_settings() else {
        eprintln!(
            "world: party: no {TRAVEL_FILE} in the pack; player {} of party {} is waited for until the \
             instance ends",
            awaited.player_id, awaited.party_id
        );
        return;
    };
    let seconds = if awaited.held {
        settings.disconnect_hold_secs.value
    } else {
        settings.arrival_grace_secs.value
    };
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(u64::from(seconds))).await;
        if expire(&instance, awaited, trace) {
            if let Err(err) = asura_db::parties::forget(&pool, awaited.character).await {
                eprintln!("world: party: could not forget an expired member in the ledger: {err}");
            }
        }
    });
}

/// How long a dropped member's place is held, from the pack, for the ledger's `held_until`.
pub fn hold_secs() -> Option<u32> {
    travel_settings().map(|settings| settings.disconnect_hold_secs.value)
}

// ---- travel: leaving ----------------------------------------------------------------------------

/// A party about to move: who goes, in order, the leader first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    /// The party in this instance.
    pub party_id: u16,
    /// The travellers: player id, agent id and character, the leader first.
    pub travellers: Vec<(u32, u32, Uuid)>,
    /// The character that leads the party where it goes.
    pub leader: Uuid,
}

/// Why a party does not move as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoMove {
    /// The player is in no party with company: it travels alone.
    Alone,
    /// The party is already leaving.
    AlreadyMoving,
    /// The destination has no party size, so nobody but the player that started it goes.
    NoSizeForDestination,
}

/// Start a party's move (any member's crossing, or a leader's countdown ending): mark the party as
/// leaving and name its travellers, leader first, then the player that started it, then the rest in
/// order, up to the destination's size (`rules::travelling_count`). Members over the size stay behind
/// in parties of their own; members not in the map are left out (a held member is left behind when its
/// party changes areas); members named in `left_out` (a leader's travel to an outpost they have not
/// unlocked) stay behind the same way. The ledger write and the orders are the caller's
/// (`travel::move_party`).
pub fn begin_move(
    instance: &Instance,
    trigger: u32,
    destination_size: Option<u32>,
    left_out: &[u32],
    trace: bool,
) -> Result<Move, NoMove> {
    let (mv, plan) = {
        let mut table = instance.parties.locked();
        let Some(party) = table.party_of(trigger).cloned() else {
            return Err(NoMove::Alone);
        };
        if party.moving {
            return Err(NoMove::AlreadyMoving);
        }
        let present = party.present();
        // A party of one moves its player alone. A party whose other members are not in the map still
        // moves as a party: it changes areas, which leaves those members behind.
        if party.members.len() <= 1 || present.is_empty() {
            return Err(NoMove::Alone);
        }
        let mut order: Vec<u32> = Vec::with_capacity(present.len());
        if present.contains(&party.leader) {
            order.push(party.leader);
        }
        if !order.contains(&trigger) {
            order.push(trigger);
        }
        let rest: Vec<u32> = present
            .iter()
            .copied()
            .filter(|m| !order.contains(m))
            .collect();
        order.extend(rest);
        // The player that started the move always goes; anyone the caller left out stays.
        let (order, left): (Vec<u32>, Vec<u32>) = order
            .into_iter()
            .partition(|m| *m == trigger || !left_out.contains(m));
        let count = rules::travelling_count(order.len(), destination_size)
            .map_err(|_| NoMove::NoSizeForDestination)?;
        let (going, staying) = order.split_at(count.max(1));
        let (going, mut staying) = (going.to_vec(), staying.to_vec());
        staying.extend(left);

        // Those over the destination's size stay, each in a party of its own (ledger R26).
        let mut plan = Plan::new();
        for stayer in &staying {
            if let Some(current) = table.party_of(*stayer).cloned() {
                plan.extend(departure(&mut table, instance, &current, *stayer));
            }
        }
        // Members not in the map are not taken along: their places are freed.
        let mut released = Vec::new();
        let mut left: Vec<u32> = Vec::new();
        if let Some(moving) = table.parties.get_mut(&party.party_id) {
            for (absent, _) in moving.absent.drain() {
                moving.members.retain(|m| *m != absent);
                moving.entering.remove(&absent);
                left.push(absent);
            }
            moving.moving = true;
            moving.countdown = None;
        }
        for absent in left {
            table.of_player.remove(&absent);
            if let Some(character) = table.characters.remove(&absent) {
                released.push(character);
            }
        }
        for character in released {
            instance.release(character);
        }
        table.close_invites_of(party.party_id);
        let leader_character = table.characters.get(&going[0]).copied().unwrap_or_default();
        let travellers = going
            .iter()
            .map(|member| {
                (
                    *member,
                    instance.agent(*member).map(|a| a.agent_id).unwrap_or(0),
                    table.characters.get(member).copied().unwrap_or_default(),
                )
            })
            .collect();
        (
            Move {
                party_id: party.party_id,
                travellers,
                leader: leader_character,
            },
            plan,
        )
    };
    if trace {
        eprintln!(
            "world: party: party {} leaves the map with {} traveller(s)",
            mv.party_id,
            mv.travellers.len()
        );
    }
    deliver(instance, plan);
    Ok(mv)
}

/// The characters of the members of a player's party that stand in the map, with their player ids: what
/// a leader's travel asks the unlocks of.
pub fn present_characters(instance: &Instance, player_id: u32) -> Vec<(u32, Uuid)> {
    let table = instance.parties.locked();
    let Some(party) = table.party_of(player_id) else {
        return Vec::new();
    };
    party
        .present()
        .into_iter()
        .filter_map(|member| table.characters.get(&member).map(|c| (member, *c)))
        .collect()
}

/// A traveller's own transfer was refused (no seat): it stays, in a party of its own, and the party's
/// move goes on without it. **Not captured**; the batch is a leave's.
pub fn stays_behind(instance: &Instance, me: u32, trace: bool) {
    let plan = {
        let mut table = instance.parties.locked();
        let Some(party) = table.party_of(me).cloned() else {
            return;
        };
        if party.members.len() <= 1 {
            if let Some(alone) = table.parties.get_mut(&party.party_id) {
                alone.moving = false;
            }
            return;
        }
        let mut plan = departure(&mut table, instance, &party, me);
        // The others are leaving and do not need to hear it.
        plan.retain(|(to, _)| *to == me || !party.members.contains(to));
        plan
    };
    if trace {
        eprintln!(
            "world: party: player {me} could not follow its party and stays in a party of its own"
        );
    }
    deliver(instance, plan);
}

/// A leader's world-map travel in an outpost: the countdown starts. Every member in the map is told
/// with `0x01BC` (Headquarter's layout, seconds first; experimental, **not captured**). Returns the
/// ticket the countdown fires with, or `None` when the player leads no party with company, or the
/// pack states no countdown.
pub fn start_countdown(player: &Player, trace: bool) -> Option<(u64, u16)> {
    let Some(settings) = travel_settings() else {
        eprintln!(
            "world: party: {:?}'s party travel is refused: the pack states no {TRAVEL_FILE}",
            player.name
        );
        return None;
    };
    let me = player.player_id;
    let instance = &player.instance;
    let (ticket, plan) = {
        let mut table = instance.parties.locked();
        let ticket = table.number();
        let party_id = table.of_player.get(&me).copied()?;
        let party = table.parties.get_mut(&party_id)?;
        if party.leader != me || party.moving {
            return None;
        }
        party.countdown = Some(ticket);
        let timer = game::instance_travel_timer(
            settings.countdown_secs.value,
            settings.countdown_byte.value,
            &settings.countdown_text.value,
        );
        let mut plan = Plan::new();
        to_all(&party.present(), &[timer], &mut plan);
        (ticket, plan)
    };
    if trace {
        eprintln!(
            "world: party: player {me} leads its party out in {} s (ticket {ticket})",
            settings.countdown_secs.value
        );
    }
    deliver(instance, plan);
    Some((ticket, settings.countdown_secs.value))
}

/// The countdown has run out: whether it still stands - the same leader leads the same party and no
/// newer countdown or change voided it. The ticket is used up either way.
pub fn countdown_stands(instance: &Instance, leader: u32, ticket: u64) -> bool {
    let mut table = instance.parties.locked();
    let Some(party_id) = table.of_player.get(&leader).copied() else {
        return false;
    };
    let Some(party) = table.parties.get_mut(&party_id) else {
        return false;
    };
    let stands = party.leader == leader && party.countdown == Some(ticket);
    if party.countdown == Some(ticket) {
        party.countdown = None;
    }
    stands
}

// ---- leaving the instance -----------------------------------------------------------------------

/// How a player's connection to the instance ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaving {
    /// It was redirected: a travel, alone or with its party.
    Travelled,
    /// The client said goodbye (`GAME_CMSG_DISCONNECT`): a logout, which is a leave.
    LoggedOut,
    /// The socket ended without a goodbye: a network drop.
    Dropped,
}

/// A member whose place its party keeps after a drop, for the caller to write in the ledger and give
/// an expiry.
pub type Hold = Awaited;

/// A player leaves the instance (logout, disconnect or travel). Called before the instance drops it.
///
/// - a member of a moving party goes quietly: its redirect batch and the farewell already said it;
/// - a member whose connection dropped in an explorable with company stays in the party, its player id
///   reserved, and the members in the map are told `0x01CE {party, player}` (ledger E38
///   `MemberDisconnectSoft`, experimental, **not captured**); the hold is returned for the caller;
/// - otherwise, parties that invited it, or that it invited, are told it left the map, then that it is
///   out of its party, then that the invitation is closed: `0x01D0`, `0x01D1`, `0x01CA` (`034507` seq
///   2645-2650, an invited player walking out). The live batch has the player's farewell between the
///   first two; here the farewell follows the party messages (`connection.rs`). For the party it had
///   invited the closing message is `0x01C8` (**not captured**); its own party, if it has company, sees
///   the departure above (a leader's hands the party on).
pub fn left_instance(
    instance: &Instance,
    me: u32,
    character: Uuid,
    leaving: Leaving,
    trace: bool,
) -> Option<Hold> {
    let mut hold = None;
    let plan = {
        let mut table = instance.parties.locked();
        let Some(mine) = table.party_of(me).cloned() else {
            return None;
        };
        if mine.moving && leaving == Leaving::Travelled {
            table.drop_quietly(mine.party_id, me);
            return None;
        }
        let in_explorable = instance.map.map_type == map_kinds().main_explorable;
        let others_present = mine.present().iter().filter(|m| **m != me).count();
        if rules::holds_on_disconnect(leaving == Leaving::Dropped, in_explorable, others_present)
            && mine.ledger.is_some()
            && travel_settings().is_some()
        {
            let epoch = table.number();
            if let Some(party) = table.parties.get_mut(&mine.party_id) {
                party.absent.insert(me, Absence { held: true, epoch });
                party.entering.remove(&me);
            }
            instance.reserve_as(character, me);
            hold = Some(Hold {
                party_id: mine.party_id,
                player_id: me,
                character,
                epoch,
                held: true,
            });
            let mut plan = Plan::new();
            let others: Vec<u32> = mine.present().into_iter().filter(|m| *m != me).collect();
            to_all(
                &others,
                &[game::party_member_disconnect_soft(mine.party_id, id(me))],
                &mut plan,
            );
            plan
        } else {
            let mut plan = Plan::new();
            let (invited, invited_by) = table.close_invites_of(mine.party_id);
            let gone = [
                game::party_member_leave_world(mine.party_id, id(me)),
                game::party_player_remove(mine.party_id, id(me)),
            ];
            for other in invited_by {
                let mut frames = gone.to_vec();
                frames.push(game::party_invite_response(mine.party_id));
                to_all(&table.members(other), &frames, &mut plan);
            }
            for other in invited {
                to_all(
                    &table.members(other),
                    &[game::party_request_cancel(mine.party_id)],
                    &mut plan,
                );
            }
            if mine.members.len() > 1 {
                plan.extend(departure(&mut table, instance, &mine, me));
            }
            // Whatever party it is in now goes with it.
            if let Some(last) = table.of_player.remove(&me) {
                table.parties.remove(&last);
                table.close_invites_of(last);
            }
            table.characters.remove(&me);
            plan
        }
    };
    if trace && !plan.is_empty() {
        eprintln!(
            "world: party: player {me} left the instance ({leaving:?}); its parties are told{}",
            if hold.is_some() {
                ", and its place is held"
            } else {
                ""
            }
        );
    }
    // The leaving player's own connection is closing; anything still addressed to it is dropped.
    deliver(
        instance,
        plan.into_iter().filter(|(to, _)| *to != me).collect(),
    );
    hold
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed pack, read the way the world reads it.
    const VANILLA_SIZES: &str = include_str!("../../../config/World/Vanilla/party_sizes.json");
    const VANILLA_TRAVEL: &str = include_str!("../../../config/World/Vanilla/party_travel.json");

    #[test]
    fn the_vanilla_pack_caps_every_pre_searing_map_at_two() {
        let sizes = PartySizes::from_json(VANILLA_SIZES).expect("the committed file parses");
        for map_id in [145, 146, 147, 148, 160, 161, 162, 163, 164, 165, 166] {
            assert_eq!(sizes.max_size(map_id), Some(2), "map {map_id}");
        }
        assert_eq!(
            sizes.max_size(1),
            None,
            "a map the file does not list has no answer"
        );
    }

    #[test]
    fn the_vanilla_pack_states_how_a_party_travels() {
        let travel = TravelSettings::from_json(VANILLA_TRAVEL).expect("the committed file parses");
        assert_eq!(travel.countdown_secs.value, 10);
        assert_eq!(
            travel.disconnect_hold_secs.value, 900,
            "ledger R11: 15 minutes"
        );
        assert!(travel.arrival_grace_secs.value > 0);
    }

    #[test]
    fn the_load_ends_with_the_party_state_before_its_finish() {
        let mut block = vec![
            game::character_update_party(1, 1),
            game::instance_load_finish(),
        ];
        splice_load_tail(&mut block, true);
        let headers: Vec<u16> = block.iter().map(Frame::header).collect();
        assert_eq!(
            headers,
            vec![
                game::CHARACTER_UPDATE_PARTY,
                asura_protocol::packets::game::GAME_SMSG_PARTY_SEARCH_SEEK,
                asura_protocol::packets::game::GAME_SMSG_PARTY_SET_DIFFICULTY,
                game::PARTY_GLOBAL_STATE_FLAGS,
                asura_protocol::packets::game::GAME_SMSG_PARTY_SEARCH_SEEK,
                game::INSTANCE_LOAD_FINISH,
            ]
        );
    }

    #[test]
    fn a_party_of_one_per_player_with_the_lowest_free_id() {
        let parties = Parties::default();
        assert_eq!(parties.found(1), 1);
        assert_eq!(parties.found(2), 2);
        assert_eq!(parties.leader_and_size(2), (2, 1));
        assert_eq!(
            parties.leader_and_size(9),
            (9, 1),
            "an unknown player is a party of one"
        );
    }

    #[test]
    fn closing_a_partys_invitations_closes_both_directions() {
        let mut table = Table::default();
        let a = table.found(1);
        let b = table.found(2);
        let c = table.found(3);
        table.invites.insert((a, b));
        table.invites.insert((c, a));
        table.invites.insert((b, c));
        let (invited, invited_by) = table.close_invites_of(a);
        assert_eq!(invited, vec![b]);
        assert_eq!(invited_by, vec![c]);
        assert_eq!(
            table.invites.len(),
            1,
            "an invitation between two other parties stays"
        );
    }

    /// `multiplay/party.jsonl` seq 2662-2669, the load of a leader whose party of two has just come
    /// through a doorway, the second member not in the map yet.
    #[test]
    fn a_party_arriving_loads_as_the_live_server_sends_it() {
        let mut party = Party {
            party_id: 1,
            leader: 1,
            members: vec![1, 2],
            ..Party::default()
        };
        party.absent.insert(
            2,
            Absence {
                held: false,
                epoch: 1,
            },
        );
        let section = load_section(&party, 1);
        let bodies: Vec<(u16, Vec<u8>)> = section
            .iter()
            .map(|frame| (frame.header(), frame.body().to_vec()))
            .collect();
        assert_eq!(
            bodies,
            vec![
                (game::PLAYER_UPDATE_PARTY_SIZE, vec![1, 0, 2]),
                (game::PLAYER_UPDATE_PARTY, vec![1, 0, 1, 0]),
                (game::PLAYER_UPDATE_PARTY, vec![2, 0, 1, 0]),
                (game::PARTY_CREATE, vec![1, 0]),
                (game::PARTY_PLAYER_ADD, vec![1, 0, 1, 0, 1]),
                (game::PARTY_PLAYER_ADD, vec![1, 0, 2, 0, 0]),
                (game::PARTY_MEMBER_STREAM_END, vec![1, 0]),
                (game::CHARACTER_UPDATE_PARTY, vec![1, 0, 1]),
            ]
        );
        assert_eq!(
            load_section(&party, 2)
                .last()
                .map(|frame| frame.body().to_vec()),
            Some(vec![1, 0, 0]),
            "a member that does not lead is told 0"
        );
    }

    #[test]
    fn the_section_replaces_the_blocks_own_party_and_keeps_what_is_in_front() {
        let mut block = vec![
            game::instance_load_finish(),
            game::party_create(1),
            game::party_player_add(1, 1, 1),
            game::party_member_stream_end(1),
            game::character_update_party(1, 1),
            game::instance_load_finish(),
        ];
        let party = Party {
            party_id: 4,
            leader: 1,
            members: vec![1],
            ..Party::default()
        };
        let start = block
            .iter()
            .position(|frame| frame.header() == game::PARTY_CREATE)
            .expect("the run");
        block.splice(start..=start + 3, load_section(&party, 1));
        let headers: Vec<u16> = block.iter().map(Frame::header).collect();
        assert_eq!(
            headers,
            vec![
                game::INSTANCE_LOAD_FINISH,
                game::PLAYER_UPDATE_PARTY_SIZE,
                game::PLAYER_UPDATE_PARTY,
                game::PARTY_CREATE,
                game::PARTY_PLAYER_ADD,
                game::PARTY_MEMBER_STREAM_END,
                game::CHARACTER_UPDATE_PARTY,
                game::INSTANCE_LOAD_FINISH,
            ]
        );
    }

    /// In its own load a client is in the map and every other member is not, whoever entered first:
    /// the others follow as `0x01CF` after their introductions.
    #[test]
    fn a_load_shows_only_its_own_player_in_the_map() {
        let party = Party {
            party_id: 1,
            leader: 1,
            members: vec![1, 2],
            ..Party::default()
        };
        let flags = |me: u32| -> Vec<u8> {
            load_section(&party, me)
                .iter()
                .filter(|frame| frame.header() == game::PARTY_PLAYER_ADD)
                .map(|frame| frame.body()[4])
                .collect()
        };
        assert_eq!(flags(1), vec![1, 0]);
        assert_eq!(flags(2), vec![0, 1]);
    }

    #[test]
    fn a_party_knows_who_is_in_the_map() {
        let mut party = Party {
            party_id: 1,
            leader: 1,
            members: vec![1, 2, 3],
            ..Party::default()
        };
        party.absent.insert(
            3,
            Absence {
                held: true,
                epoch: 1,
            },
        );
        assert_eq!(party.present(), vec![1, 2]);
        assert!(party.is_present(2));
        assert!(!party.is_present(3));
        assert!(!party.is_present(9));
    }

    #[test]
    fn a_quiet_drop_hands_the_party_on_and_ends_it_with_its_last_member() {
        let mut table = Table::default();
        let party_id = table.found(1);
        {
            let party = table.parties.get_mut(&party_id).expect("founded");
            party.members.push(2);
            party.moving = true;
        }
        table.of_player.insert(2, party_id);
        table.drop_quietly(party_id, 1);
        assert_eq!(
            table.parties.get(&party_id).map(|party| party.leader),
            Some(2)
        );
        table.drop_quietly(party_id, 2);
        assert!(table.parties.is_empty());
        assert!(table.of_player.is_empty());
    }
}
