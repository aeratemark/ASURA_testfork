//! The quest content a pack carries: the log entry, its details, and the variants a capture shows.
//!
//! The file is `config/World/<pack>/quests.json` (`docs/58-quests.md`). Every text field is **encoded
//! units**, the client's own string ids and control codes; this module never composes, resolves or
//! normalises one. A marker is stored as the exact IEEE-754 bits the capture carried, because
//! `+infinity, +infinity` is the client's own "no marker" sentinel and a float written as text would
//! lose that distinction.
//!
//! The rules that read this content live in `asura_rules::quest`; the wire builders are
//! `asura_protocol::game::quest`. This module is the file reader and the once-per-process copy, the
//! same shape `dialog.rs` and `experience.rs` use, plus what a quest does today: the floating marker an
//! NPC carries, the log restore after a login, and the answer to an NPC's offer or turn-in
//! (`docs/58-quests.md`). A missing or broken file is logged once and no quest is available, rather than
//! a quest being invented.

use asura_protocol::game::quest as wire;
use asura_rules::quest::{Marker as QuestMarker, Profile, Progress, Stage};

use crate::{db_quest, send, Frame, GameError, Player, Session, TcpStream, World};

/// The file inside a pack that holds quests.
pub const QUEST_FILE: &str = "quests.json";

/// The pack's `quests.json`, as it is written.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct QuestFile {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// What each key means.
    #[serde(default)]
    pub fields: Option<std::collections::BTreeMap<String, String>>,
    /// The quests, one per client id.
    pub quests: Vec<Quest>,
}

/// One quest's content: the entry a load restores, and the details a stage shows.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Quest {
    /// The client's quest id.
    pub id: u32,
    /// The captured log state. Bit 1 is the details-loaded flag and bit 2 the completed flag; a restore
    /// masks both and owns them (`gw_quest_restore.h`). Bit 32 is the primary-quest flag and survives.
    pub state: u32,
    /// The marker the active capture carried.
    pub marker: Marker,
    /// The map the marker points at.
    pub map_to: u16,
    /// The map the quest was taken on.
    pub map_from: u16,
    /// The log category, encoded units.
    pub location: Vec<u16>,
    /// The quest's name, encoded units.
    pub name: Vec<u16>,
    /// The quest giver's name, encoded units.
    pub npc: Vec<u16>,
    /// The 0x004C description, encoded units. Absent when the capture never loaded the details.
    #[serde(default)]
    pub description: Option<Vec<u16>>,
    /// The 0x004C objectives, encoded units.
    #[serde(default)]
    pub objectives: Option<Vec<u16>>,
    /// The captured character name inside the description, replaced at send time.
    #[serde(default)]
    pub character_argument: Option<Vec<u16>>,
    /// The details the capture shows once the quest is completed.
    #[serde(default)]
    pub completed: Option<Variant>,
    /// The details the capture shows after this many objective credits.
    #[serde(default)]
    pub counted: Vec<Variant>,
    /// Who offers the quest, and the ids that must not be complete before the offer opens.
    #[serde(default)]
    pub offer: Option<Offer>,
    /// Who takes the turn-in, and what it pays.
    #[serde(default)]
    pub reward: Option<Reward>,
}

/// One quest's offer: the NPC and map the quest starts at, who may take it, and what accepting teaches.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Offer {
    /// The placement's literal name, with or without its bracketed title.
    pub npc: String,
    /// The map the NPC stands on.
    pub map: i32,
    /// Quest ids that must be neither completed nor rewarded before this offer is open.
    #[serde(default)]
    pub requires_none_completed: Vec<u32>,
    /// Quest ids that must all be rewarded before this offer is open.
    #[serde(default)]
    pub requires_rewarded: Vec<u32>,
    /// Quest ids of which at least one must be rewarded (the six profession tests, for instance).
    #[serde(default)]
    pub requires_any_rewarded: Vec<u32>,
    /// The professions the quest is for; empty is every profession. See [`Offer::eligible`].
    #[serde(default)]
    pub professions: Vec<u8>,
    /// Whether only a primary profession in `professions` qualifies; otherwise a matching secondary,
    /// or no secondary yet, qualifies too (preforged `gw_presearing_quests.h`, `eligible`).
    #[serde(default)]
    pub primary_only: bool,
    /// The lowest character level the offer is open to.
    #[serde(default)]
    pub min_level: u16,
    /// Skills the character learns when it accepts.
    #[serde(default)]
    pub skills: Vec<u32>,
}

/// One quest's turn-in: the NPC and map, and the reward amounts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Reward {
    /// The placement's literal name, with or without its bracketed title.
    pub npc: String,
    /// The map the NPC stands on.
    pub map: i32,
    /// Experience the turn-in grants.
    pub experience: u32,
    /// Gold the turn-in grants.
    pub gold: u32,
    /// Skill points the turn-in grants, over what the experience's levels give.
    #[serde(default)]
    pub skill_points: u32,
    /// Skills the character learns at the turn-in.
    #[serde(default)]
    pub skills: Vec<u32>,
    /// Whether the turn-in waits for the objective to be complete. A quest whose objective this server
    /// does not model yet leaves it false, so it can be turned in once accepted.
    #[serde(default)]
    pub requires_completed: bool,
}

/// One stage's details: the text, and the marker and destination when they move with the stage.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Variant {
    /// The marker this stage carries; absent means the quest's own.
    #[serde(default)]
    pub marker: Option<Marker>,
    /// The destination this stage carries; absent means the quest's own.
    #[serde(default)]
    pub map_to: Option<u16>,
    /// The description, encoded units.
    pub description: Vec<u16>,
    /// The objectives, encoded units.
    pub objectives: Vec<u16>,
    /// The captured character name inside the description, replaced at send time.
    #[serde(default)]
    pub character_argument: Option<Vec<u16>>,
    /// How many objective credits this variant was captured at; only a counted variant has one.
    #[serde(default)]
    pub credits: Option<u8>,
}

/// A marker as exact IEEE-754 bits and a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct Marker {
    /// The x coordinate's bits.
    pub x_bits: u32,
    /// The y coordinate's bits.
    pub y_bits: u32,
    /// The plane the marker sits on.
    pub plane: u16,
}

impl Marker {
    /// The x coordinate.
    pub fn x(&self) -> f32 {
        f32::from_bits(self.x_bits)
    }

    /// The y coordinate.
    pub fn y(&self) -> f32 {
        f32::from_bits(self.y_bits)
    }

    /// Whether this is the client's "no marker" pair.
    pub fn is_none(&self) -> bool {
        self.x() == asura_protocol::game::quest::QUEST_NO_MARKER
            && self.y() == asura_protocol::game::quest::QUEST_NO_MARKER
    }
}

impl QuestFile {
    /// Parse and check the file's text.
    pub fn from_json(text: &str) -> Result<QuestFile, String> {
        let file: QuestFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
        file.check()?;
        Ok(file)
    }

    /// Check every quest: a unique nonzero id, the client's text bounds, a valid marker, a complete
    /// details pair, a contiguous counted sequence and every variant's own details.
    fn check(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for quest in &self.quests {
            if quest.id == 0 {
                return Err("a quest has id 0".to_string());
            }
            if !ids.insert(quest.id) {
                return Err(format!("quest {} is named twice", quest.id));
            }
            check_marker(&quest.marker).map_err(|err| format!("quest {}: {err}", quest.id))?;
            check_name(&quest.location)
                .map_err(|err| format!("quest {} location: {err}", quest.id))?;
            check_name(&quest.name).map_err(|err| format!("quest {} name: {err}", quest.id))?;
            check_name(&quest.npc).map_err(|err| format!("quest {} NPC: {err}", quest.id))?;
            if quest.description.is_some() != quest.objectives.is_some() {
                return Err(format!("quest {}: only half of the details pair", quest.id));
            }
            if let Some(description) = &quest.description {
                check_detail(description)
                    .map_err(|err| format!("quest {} description: {err}", quest.id))?;
                let objectives = quest.objectives.as_ref().expect("the pair was checked");
                check_detail(objectives)
                    .map_err(|err| format!("quest {} objectives: {err}", quest.id))?;
            }
            if let Some(argument) = &quest.character_argument {
                check_argument(argument)
                    .map_err(|err| format!("quest {} character argument: {err}", quest.id))?;
            }
            if let Some(completed) = &quest.completed {
                check_variant(quest.id, completed)?;
                if completed.credits.is_some() {
                    return Err(format!(
                        "quest {}: a completed variant names credits",
                        quest.id
                    ));
                }
            }
            if quest.offer.is_some() != quest.reward.is_some() {
                return Err(format!(
                    "quest {}: only half of the offer/reward pair",
                    quest.id
                ));
            }
            if let Some(offer) = &quest.offer {
                if offer.npc.trim().is_empty() || offer.map <= 0 {
                    return Err(format!(
                        "quest {}: an offer needs an NPC and a map",
                        quest.id
                    ));
                }
                if offer
                    .requires_none_completed
                    .iter()
                    .chain(&offer.requires_rewarded)
                    .chain(&offer.requires_any_rewarded)
                    .any(|id| *id == 0)
                {
                    return Err(format!("quest {}: a prerequisite id is 0", quest.id));
                }
                if offer.professions.iter().any(|p| *p == 0 || *p > 10) {
                    return Err(format!("quest {}: a profession is not 1 to 10", quest.id));
                }
                if offer.skills.iter().any(|id| *id == 0) {
                    return Err(format!("quest {}: an offer skill is 0", quest.id));
                }
            }
            if let Some(reward) = &quest.reward {
                if reward.npc.trim().is_empty() || reward.map <= 0 {
                    return Err(format!(
                        "quest {}: a reward needs an NPC and a map",
                        quest.id
                    ));
                }
                if reward.skills.iter().any(|id| *id == 0) {
                    return Err(format!("quest {}: a reward skill is 0", quest.id));
                }
            }
            for (index, variant) in quest.counted.iter().enumerate() {
                check_variant(quest.id, variant)?;
                if variant.credits != Some((index + 1) as u8) {
                    return Err(format!(
                        "quest {}: counted variant {} does not name {}",
                        quest.id,
                        index + 1,
                        index + 1
                    ));
                }
            }
        }
        Ok(())
    }

    /// The quest with this client id, if the pack carries it.
    pub fn find(&self, id: u32) -> Option<&Quest> {
        self.quests.iter().find(|quest| quest.id == id)
    }
}

fn check_name(units: &[u16]) -> Result<(), String> {
    if units.is_empty() {
        return Err("empty".to_string());
    }
    if units.len() > asura_protocol::game::quest::QUEST_NAME_LIMIT {
        return Err(format!(
            "{} units, the client takes {} plus a terminator",
            units.len(),
            asura_protocol::game::quest::QUEST_NAME_LIMIT
        ));
    }
    if units.contains(&0) {
        return Err("embeds a terminator".to_string());
    }
    Ok(())
}

fn check_detail(units: &[u16]) -> Result<(), String> {
    if units.len() > asura_protocol::game::quest::QUEST_DETAIL_LIMIT {
        return Err(format!(
            "{} units, the client takes {} plus a terminator",
            units.len(),
            asura_protocol::game::quest::QUEST_DETAIL_LIMIT
        ));
    }
    if units.contains(&0) {
        return Err("embeds a terminator".to_string());
    }
    Ok(())
}

fn check_argument(units: &[u16]) -> Result<(), String> {
    if units.is_empty() || units.len() > 19 {
        return Err(format!("{} units, the client takes 1 to 19", units.len()));
    }
    if units.contains(&0) {
        return Err("embeds a terminator".to_string());
    }
    Ok(())
}

fn check_marker(marker: &Marker) -> Result<(), String> {
    if !asura_protocol::game::quest::marker_is_valid(marker.x(), marker.y()) {
        return Err(format!(
            "marker is neither finite nor the positive-infinity pair: {:08x} {:08x}",
            marker.x_bits, marker.y_bits
        ));
    }
    Ok(())
}

fn check_variant(quest: u32, variant: &Variant) -> Result<(), String> {
    if let Some(marker) = &variant.marker {
        check_marker(marker).map_err(|err| format!("quest {quest} variant: {err}"))?;
    }
    check_detail(&variant.description)
        .map_err(|err| format!("quest {quest} variant description: {err}"))?;
    check_detail(&variant.objectives)
        .map_err(|err| format!("quest {quest} variant objectives: {err}"))?;
    if let Some(argument) = &variant.character_argument {
        check_argument(argument).map_err(|err| format!("quest {quest} variant argument: {err}"))?;
    }
    Ok(())
}

/// Read `<pack>/quests.json`.
pub fn load(pack: &std::path::Path) -> Result<QuestFile, String> {
    let path = pack.join(QUEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    QuestFile::from_json(&text)
        .map_err(|err| format!("{} is not a quest file: {err}", path.display()))
}

/// The quests in force, read once per process. A missing or broken file is logged once and answers
/// `None`; a quest command is then refused with a logged reason.
pub fn current() -> Option<&'static QuestFile> {
    static IN_FORCE: std::sync::OnceLock<Option<QuestFile>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load(&pack) {
                Ok(quests) => {
                    eprintln!(
                        "asura: {} quest(s) from {}",
                        quests.quests.len(),
                        pack.display()
                    );
                    Some(quests)
                }
                Err(err) => {
                    eprintln!("asura: {err}; quests are off");
                    None
                }
            }
        })
        .as_ref()
}

/// The storage bounds the stage rule is checked against, from `config/Server/quests.json`.
pub fn limits() -> asura_rules::quest::Limits {
    let settings = asura_config::quest_settings();
    asura_rules::quest::Limits {
        max_records: settings.max_records(),
        max_credits: settings.max_credits(),
    }
}

/// The character a quest decision is made for: its quest profile, its professions and its level.
#[derive(Debug, Clone, Copy)]
pub struct Who<'a> {
    /// The character's attempts.
    pub quests: &'a Profile,
    /// The primary profession, 0 for none.
    pub primary: u8,
    /// The secondary profession, 0 for none yet.
    pub secondary: u8,
    /// The character level.
    pub level: u16,
}

impl<'a> Who<'a> {
    /// The playing character.
    pub fn of(player: &'a Player) -> Who<'a> {
        Who {
            quests: &player.quests,
            primary: player.traits.profession,
            secondary: player.traits.secondary_profession,
            level: player.traits.level,
        }
    }
}

fn stage_of(quests: &Profile, id: u32) -> Option<Stage> {
    quests.records.get(&id).map(|progress| progress.stage)
}

impl Offer {
    /// Whether this character may be offered the quest, apart from the attempt itself: its level, its
    /// professions and the prerequisite quests.
    ///
    /// The profession rule is preforged's (`gw_presearing_quests.h`, `eligible`): a primary in
    /// `professions` always qualifies; unless `primary_only`, a matching secondary does too, and so does
    /// a character with no secondary yet, which is how a pre-Searing character finds its second
    /// profession.
    pub fn eligible(&self, who: &Who) -> bool {
        if who.level < self.min_level {
            return false;
        }
        if !self.professions.is_empty() {
            let primary = self.professions.contains(&who.primary);
            let secondary = !self.primary_only
                && (who.secondary == 0 || self.professions.contains(&who.secondary));
            if !primary && !secondary {
                return false;
            }
        }
        let rewarded = |id: &u32| stage_of(who.quests, *id) == Some(Stage::Rewarded);
        if !self.requires_rewarded.iter().all(rewarded) {
            return false;
        }
        if !self.requires_any_rewarded.is_empty()
            && !self.requires_any_rewarded.iter().any(rewarded)
        {
            return false;
        }
        !self.requires_none_completed.iter().any(|id| {
            matches!(
                stage_of(who.quests, *id),
                Some(Stage::Completed | Stage::Rewarded)
            )
        })
    }
}

/// Whether the quest's offer is open to this character: the attempt rule (no attempt, or an abandoned
/// one) and the offer block's own conditions. A quest the pack gives no offer is never offered.
pub fn offer_open(who: &Who, id: u32) -> bool {
    let Some(offer) = current()
        .and_then(|file| file.find(id))
        .and_then(|quest| quest.offer.as_ref())
    else {
        return false;
    };
    asura_rules::quest::offer_open(who.quests, id) && offer.eligible(who)
}

/// Whether the quest's turn-in is open: an active or completed attempt, or only a completed one when the
/// reward block waits for the objective.
pub fn reward_open(who: &Who, id: u32) -> bool {
    let Some(reward) = current()
        .and_then(|file| file.find(id))
        .and_then(|quest| quest.reward.as_ref())
    else {
        return false;
    };
    if reward.requires_completed {
        stage_of(who.quests, id) == Some(Stage::Completed)
    } else {
        asura_rules::quest::reward_open(who.quests, id)
    }
}

/// Whether the character is working on the quest: accepted, objective not yet complete.
pub fn quest_active(who: &Who, id: u32) -> bool {
    stage_of(who.quests, id) == Some(Stage::Active)
}

/// The marker an NPC carries for this character, from the pack's own offer and reward blocks. A turn-in
/// (`QuestMarker::TurnIn`) wins over an offer (`QuestMarker::Offer`), the order the live capture shows
/// when one NPC has both. The value sent to the client is [`marker_value`].
pub fn marker_for(who: &Who, npc_name: &str, map_id: i32) -> QuestMarker {
    let Some(file) = current() else {
        return QuestMarker::None;
    };
    let name = crate::vendor::content::base_name(npc_name);
    let mut strongest = QuestMarker::None;
    for quest in &file.quests {
        if let Some(block) = &quest.offer {
            if block.map == map_id
                && crate::vendor::content::base_name(&block.npc) == name
                && offer_open(who, quest.id)
            {
                strongest = strongest.max(QuestMarker::Offer);
            }
        }
        if let Some(block) = &quest.reward {
            if block.map == map_id
                && crate::vendor::content::base_name(&block.npc) == name
                && reward_open(who, quest.id)
            {
                strongest = strongest.max(QuestMarker::TurnIn);
            }
        }
    }
    strongest
}

/// The client value for a marker state, from `config/Server/quests.json`: `marker_offer`,
/// `marker_turn_in` and `marker_none`. The client's own vocabulary, so the file marks the values
/// `immutable`; changing one changes what this server asks the client to draw.
pub fn marker_value(marker: QuestMarker) -> u32 {
    let settings = asura_config::quest_settings();
    match marker {
        QuestMarker::None => settings.marker_none(),
        QuestMarker::Offer => settings.marker_offer(),
        QuestMarker::TurnIn => settings.marker_turn_in(),
    }
}

/// The `0x009F` frame that draws (or clears) one NPC's quest marker.
pub fn marker_frame(agent_id: u32, value: u32) -> Frame {
    asura_protocol::game::agent_property_update_int(
        wire::AGENT_PROPERTY_QUEST_MARKER,
        agent_id,
        value,
    )
}

/// The `0x009F` frame that sets one NPC's marker to `marker`: property 11 with the marker's value for an
/// offer or a turn-in, and property 12 with 0 for none.
///
/// The live server never clears with property 11. Every capture's property-11 values are the offer's 5
/// and the turn-in's 4 (and 2/3 on NPCs this pack does not offer from); a giver whose quest was just
/// accepted, or a trainer whose turn-in was just taken, gets `{12, npc, 0}` instead
/// (four-professions-2026-10-11 packets, ticks 81485859 and 81601156). A property-11 value of 0 is
/// drawn by the client as a marker, which is what kept the exclamation up after an acceptance.
pub fn marker_update(agent_id: u32, marker: QuestMarker) -> Frame {
    match marker {
        QuestMarker::None => crate::vendor::protocol::talked_to(agent_id),
        _ => marker_frame(agent_id, marker_value(marker)),
    }
}

/// Add the floating marker to every NPC a frame batch creates, for the player's own profile.
///
/// The load path appends the marker itself; this is the other path, the instance tick that creates a
/// body as it walks into range. The agent id is the first dword of `0x0020`'s body, and a player create
/// is not an NPC, so it is left alone. The markers go after the batch, where the entry burst's last
/// property sits.
pub fn with_npc_markers(frames: Vec<Frame>, player: &Player) -> Vec<Frame> {
    let mut markers = Vec::new();
    let map_id = player.instance.map.map_id;
    for frame in &frames {
        if frame.header() != crate::game::WORLD_CREATE_AGENT {
            continue;
        }
        let Some(agent_id) = frame
            .body()
            .get(..4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
        else {
            continue;
        };
        let Some(name) = player
            .instance
            .npc_target(agent_id)
            .and_then(|target| target.name_text())
        else {
            continue;
        };
        let marker = marker_for(&Who::of(player), &name, map_id);
        if marker != QuestMarker::None {
            markers.push(marker_frame(agent_id, marker_value(marker)));
        }
    }
    let mut out = frames;
    out.extend(markers);
    out
}

/// Whether any quest in the pack names this NPC, on this map, as its giver or its turn-in.
pub fn is_quest_npc(npc_name: &str, map_id: i32) -> bool {
    let Some(file) = current() else {
        return false;
    };
    let name = crate::vendor::content::base_name(npc_name);
    file.quests.iter().any(|quest| {
        let offer = quest.offer.as_ref().is_some_and(|block| {
            block.map == map_id && crate::vendor::content::base_name(&block.npc) == name
        });
        let reward = quest.reward.as_ref().is_some_and(|block| {
            block.map == map_id && crate::vendor::content::base_name(&block.npc) == name
        });
        offer || reward
    })
}

/// The marker one NPC should carry right now, re-sent after any answer to a talk.
///
/// The client drops the floating marker when it is talked to (the dialog burst ends with
/// `0x009F {12, npc, 0}`), whatever the quest state. The marker is the server's to own, so after a talk,
/// a dialog choice, an acceptance or a turn-in the NPC's marker is sent again from the character's
/// profile: an offer stays an offer until it is accepted, a turn-in stays until it is rewarded, and an
/// NPC with nothing left clears. An NPC no quest names gets nothing, so merchants are untouched.
pub fn refresh_marker(player: &Player, npc: u32) -> Option<Frame> {
    let map_id = player.instance.map.map_id;
    let name = player
        .instance
        .npc_target(npc)
        .and_then(|target| target.name_text())?;
    if !is_quest_npc(&name, map_id) {
        return None;
    }
    let marker = marker_for(&Who::of(player), &name, map_id);
    Some(marker_update(npc, marker))
}

/// The marker frames for the NPCs a quest transition touches: the offer and the turn-in giver.
fn marker_frames(player: &Player, map_id: i32, names: &[&str]) -> Vec<Frame> {
    let who = Who::of(player);
    let mut frames = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        let key = crate::vendor::content::base_name(name).to_string();
        if !seen.insert(key) {
            continue;
        }
        let Some(agent_id) = player.instance.npc_agent_named(name) else {
            continue;
        };
        frames.push(marker_update(agent_id, marker_for(&who, name, map_id)));
    }
    frames
}

/// Turn the stored rows into the profile the rule reads.
pub fn profile(rows: Vec<db_quest::Row>) -> Result<Profile, String> {
    let mut profile = Profile::default();
    for row in rows {
        let stage = Stage::from_value(row.stage as u32)
            .ok_or_else(|| format!("quest {} has stage {}", row.quest_id, row.stage))?;
        profile.records.insert(
            row.quest_id,
            Progress {
                cycle: row.cycle,
                stage,
                credits: Default::default(),
            },
        );
    }
    Ok(profile)
}

/// The text and marker one stage shows. A completed attempt uses its completed variant when the capture
/// carried one; an active attempt with credits uses the variant captured at that credit count. A
/// missing counted variant is a content error, not a reason to show the previous text.
struct Shown<'a> {
    description: Option<&'a [u16]>,
    objectives: Option<&'a [u16]>,
    marker: Marker,
    map_to: u16,
    argument: Option<&'a [u16]>,
}

fn shown<'a>(quest: &'a Quest, progress: &Progress) -> Option<Shown<'a>> {
    let counted = !progress.credits.is_empty();
    let variant = if progress.stage == Stage::Completed {
        quest.completed.as_ref()
    } else if counted {
        quest
            .counted
            .iter()
            .find(|variant| usize::from(variant.credits.unwrap_or(0)) == progress.credits.len())
    } else {
        None
    };
    if counted && variant.is_none() {
        return None;
    }
    Some(match variant {
        Some(variant) => Shown {
            description: Some(&variant.description),
            objectives: Some(&variant.objectives),
            marker: variant.marker.unwrap_or(quest.marker),
            map_to: variant.map_to.unwrap_or(quest.map_to),
            argument: variant
                .character_argument
                .as_deref()
                .or(quest.character_argument.as_deref()),
        },
        None => Shown {
            description: quest.description.as_deref(),
            objectives: quest.objectives.as_deref(),
            marker: quest.marker,
            map_to: quest.map_to,
            argument: quest.character_argument.as_deref(),
        },
    })
}

/// Bind the character name a captured text carries, or copy the text when it carries none.
fn bound(text: &[u16], argument: Option<&[u16]>, name: &[u16]) -> Option<Vec<u16>> {
    match argument {
        Some(argument) => asura_rules::quest::bind_character_argument(
            text,
            argument,
            name,
            19,
            wire::QUEST_DETAIL_LIMIT,
        ),
        None => Some(text.to_vec()),
    }
}

/// The add and details messages for one attempt, with the state bits the restore owns.
pub fn add_and_details(
    quest: &Quest,
    progress: &Progress,
    character_name: &[u16],
) -> Option<Vec<Frame>> {
    entry_frames(quest, progress, character_name)
}

/// One attempt's log entry as an acceptance sends it: `0x0049` add then the details. The add is the
/// client's "quest added" event, popup and sound; a map load restores with [`load_log`] instead.
fn entry_frames(quest: &Quest, progress: &Progress, character_name: &[u16]) -> Option<Vec<Frame>> {
    let shown = shown(quest, progress)?;
    let mut state = quest.state & !3;
    if progress.stage == Stage::Completed {
        state |= 2;
    }
    let entry = wire::QuestEntry {
        id: quest.id,
        state,
        x: shown.marker.x(),
        y: shown.marker.y(),
        plane: shown.marker.plane,
        map_to: shown.map_to,
        map_from: quest.map_from,
        location: &quest.location,
        name: &quest.name,
        npc: &quest.npc,
    };
    let mut frames = vec![wire::quest_add(&entry)];
    if let (Some(description), Some(objectives)) = (shown.description, shown.objectives) {
        // Only the description carries the captured character argument
        // (`gw_quest_text_binding.h`, `bind_quest_character_description`); the objectives are copied.
        let description = bound(description, shown.argument, character_name)?;
        frames.push(wire::quest_description(quest.id, &description, objectives));
    }
    Some(frames)
}

/// The quest log a map load restores, in the three places the live server puts it.
///
/// The client starts every map - a login's or a travel's - with an empty log, and the live server
/// rebuilds it inside the load, while the loading screen is up (four-professions-2026-10-11 packets,
/// quest 222 accepted in map 148 and loaded into 146):
///
/// * `entries`: `0x0050` general info per quest, in the item answer after the gold and before the
///   faction caps (tick 81524875: `0x0140`, `0x0141`, `0x0050`, `0x00EA`..`0x00ED`) -
///   `crate::item::handle_request_items`;
/// * `markers`: `0x0053` per quest, in the player block before the load finishes (81525390, before
///   `0x018E` at 81525406) - `crate::player::handle_request_players`;
/// * `details`: `0x004C` per quest, just after the load (81525500).
///
/// `0x0050` raises the client's "quest added" event (preforged `quest-evidence.txt`, 2026-10-13: the
/// general-info handler emits QuestAdded); inside the load it shows nothing, after it the client
/// announced every quest. Not `0x0049` either, the acceptance's own add. Rewarded and abandoned
/// attempts stay in the table and out of the client's log; a record whose content or counted variant
/// is missing is logged and left out, and the rest of the log is still sent.
#[derive(Debug, Default)]
pub struct LoadLog {
    /// `0x0050` general info, one per quest.
    pub entries: Vec<Frame>,
    /// `0x0053` marker, one per quest with a marker.
    pub markers: Vec<Frame>,
    /// The quests `markers` carries, in its order.
    pub marked: Vec<u32>,
    /// `0x004C` details, one per quest with captured details.
    pub details: Vec<Frame>,
}

/// The [`LoadLog`] for this profile, in quest-id order.
pub fn load_log(profile: &Profile, character_name: &str) -> LoadLog {
    let mut log = LoadLog::default();
    let Some(file) = current() else {
        return log;
    };
    let character_name: Vec<u16> = character_name.encode_utf16().collect();
    for (id, progress) in &profile.records {
        if matches!(progress.stage, Stage::Rewarded | Stage::Abandoned) {
            continue;
        }
        let Some(quest) = file.find(*id) else {
            eprintln!(
                "world: quest {id} is active but the pack has no content for it; not restored"
            );
            continue;
        };
        let Some(shown) = shown(quest, progress) else {
            eprintln!("world: quest {id} lacks the content its stage needs; not restored");
            continue;
        };
        let details = match (shown.description, shown.objectives) {
            (Some(description), Some(objectives)) => {
                let Some(description) = bound(description, shown.argument, &character_name) else {
                    eprintln!("world: quest {id}'s description does not bind; not restored");
                    continue;
                };
                Some(wire::quest_description(quest.id, &description, objectives))
            }
            _ => None,
        };
        let mut state = quest.state & !3;
        if progress.stage == Stage::Completed {
            state |= 2;
        }
        let entry = wire::QuestEntry {
            id: quest.id,
            state,
            x: shown.marker.x(),
            y: shown.marker.y(),
            plane: shown.marker.plane,
            map_to: shown.map_to,
            map_from: quest.map_from,
            location: &quest.location,
            name: &quest.name,
            npc: &quest.npc,
        };
        log.entries.push(wire::quest_general_info(&entry));
        if entry.x.is_finite() && entry.y.is_finite() {
            log.markers.push(wire::quest_add_marker(
                entry.id,
                entry.x,
                entry.y,
                entry.plane,
                entry.map_to,
            ));
            log.marked.push(entry.id);
        }
        log.details.extend(details);
    }
    log
}

/// `GAME_CMSG_SEND_DIALOG`'s quest offer: accept the quest at `npc_name` and put it in the log.
///
/// The content names the NPC and the map; the dialog page names the quest. The rule decides whether an
/// acceptance is open - an abandoned attempt may be retaken, an active or rewarded one may not - and the
/// prerequisite ids in the pack's offer block must be neither completed nor rewarded.
#[allow(clippy::too_many_arguments)]
pub async fn handle_offer(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    quest_id: u32,
    npc: u32,
    npc_name: &str,
    map_id: i32,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(file) = current() else {
        return Ok(true);
    };
    let Some(quest) = file.find(quest_id) else {
        eprintln!("world: a quest offer names {quest_id}, which the pack does not carry");
        return Ok(true);
    };
    let Some(offer) = &quest.offer else {
        eprintln!("world: quest {quest_id} has no offer block; nothing accepted");
        return Ok(true);
    };
    if crate::vendor::content::base_name(&offer.npc) != crate::vendor::content::base_name(npc_name)
        || offer.map != map_id
    {
        eprintln!(
            "world: quest {quest_id} is offered by {:?} on map {}, not by {npc_name:?} on {map_id}; refused",
            offer.npc, offer.map
        );
        return Ok(true);
    }
    if !offer.eligible(&Who::of(player)) {
        eprintln!(
            "world: {} is not eligible for quest {quest_id} (level, profession or prerequisites); refused",
            player.name
        );
        return Ok(true);
    }
    let limits = limits();
    let Some(next) = player.quests.accept(quest_id, false, &limits) else {
        eprintln!(
            "world: {} may not accept quest {quest_id} now; refused",
            player.name
        );
        return Ok(true);
    };
    let cycle = next
        .records
        .get(&quest_id)
        .map(|progress| progress.cycle)
        .unwrap_or(0);
    let character_name: Vec<u16> = player.name.encode_utf16().collect();
    let Some(progress) = next.records.get(&quest_id) else {
        return Ok(true);
    };
    // The wire batch is prepared before the row is written, as preforged's controller does: a content
    // error then refuses the acceptance instead of storing a quest the client never saw.
    let Some(mut batch) = add_and_details(quest, progress, &character_name) else {
        eprintln!("world: quest {quest_id} lacks the content its offer needs; nothing accepted");
        return Ok(true);
    };
    if !db_quest::accept(&world.pool, player.character, quest_id, cycle)
        .await
        .map_err(GameError::Database)?
    {
        eprintln!(
            "world: {}'s acceptance of quest {quest_id} was not stored; nothing sent",
            player.name
        );
        return Ok(true);
    }
    player.quests = next;
    // The add carries the marker, so the client's marker request that follows is not answered again.
    player.quest_markers_sent.insert(quest_id);
    // The live acceptance burst at tick 169042640 is add, the turn-in giver's marker, the dialog
    // sender, the talked-to property, then the details 281 ms later.
    let details = (batch.len() > 1).then(|| batch.pop()).flatten();
    // A quest that teaches on acceptance (Grawl Invasion's three skills, for instance) does so now,
    // after the row is stored.
    let taught = crate::skill::character::teach(world, player, &offer.skills).await;
    let mut names = vec![offer.npc.as_str()];
    if let Some(block) = &quest.reward {
        names.push(block.npc.as_str());
    }
    batch.extend(marker_frames(player, map_id, &names));
    batch.push(asura_protocol::game::dialog::dialog_sender(npc));
    batch.push(crate::vendor::protocol::talked_to(npc));
    if let Some(details) = details {
        batch.push(details);
    }
    batch.extend(taught);
    send(stream, session, batch, trace).await?;
    eprintln!(
        "world: {} accepted quest {quest_id} from {npc_name:?} (cycle {cycle})",
        player.name
    );
    Ok(true)
}

/// `GAME_CMSG_SEND_DIALOG`'s quest turn-in: reward the quest, pay its experience and gold, and remove
/// it from the log.
///
/// The captured order is the marker, the active details, the removal, then the experience award and the
/// gold delta. In preforged's 2022 numbering the same batch is 76, 75, 81, 319
/// (`message-from-friend80-policy-20261030.txt`); this build's marker is `0x004D`. The database write is
/// one transaction: if the quest row is not active or completed at this cycle, nothing is paid.
#[allow(clippy::too_many_arguments)]
pub async fn handle_reward(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    quest_id: u32,
    npc: u32,
    npc_name: &str,
    map_id: i32,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(file) = current() else {
        return Ok(true);
    };
    let Some(quest) = file.find(quest_id) else {
        eprintln!("world: a quest turn-in names {quest_id}, which the pack does not carry");
        return Ok(true);
    };
    let Some(reward) = &quest.reward else {
        eprintln!("world: quest {quest_id} has no reward block; nothing paid");
        return Ok(true);
    };
    if crate::vendor::content::base_name(&reward.npc) != crate::vendor::content::base_name(npc_name)
        || reward.map != map_id
    {
        eprintln!(
            "world: quest {quest_id} is turned in at {:?} on map {}, not at {npc_name:?} on {map_id}; refused",
            reward.npc, reward.map
        );
        return Ok(true);
    }
    let Some(progress) = player.quests.records.get(&quest_id).cloned() else {
        eprintln!(
            "world: {} has no attempt at quest {quest_id}; refused",
            player.name
        );
        return Ok(true);
    };
    if !reward_open(&Who::of(player), quest_id) {
        eprintln!(
            "world: {}'s quest {quest_id} is {:?}; no turn-in",
            player.name, progress.stage
        );
        return Ok(true);
    }
    let Some(shown) = shown(quest, &progress) else {
        eprintln!("world: quest {quest_id} lacks the text its turn-in needs; nothing sent");
        return Ok(true);
    };
    let Some(curve) = crate::experience::curve() else {
        eprintln!("world: no experience curve; quest {quest_id}'s reward is not paid");
        return Ok(true);
    };
    let before = player.traits;
    let experience = before.experience.saturating_add(reward.experience);
    let level = curve.level_for(experience);
    let gained = curve
        .skill_points_for(level)
        .saturating_sub(curve.skill_points_for(u32::from(before.level.max(1))));
    let gained = gained.saturating_add(reward.skill_points);
    let skill_points = before.skill_points.saturating_add(gained);
    let skill_points_total = before.skill_points_total.saturating_add(gained);
    let gold = before.gold.saturating_add(reward.gold);
    // The stage fence is decided before the write, so a failed transition never becomes a paid one.
    let mut next = player.quests.clone();
    if progress.stage == Stage::Active {
        let Some(completed) =
            next.advance(quest_id, progress.cycle, Stage::Active, Stage::Completed)
        else {
            eprintln!("world: quest {quest_id} could not complete; nothing paid");
            return Ok(true);
        };
        next = completed;
    }
    let Some(rewarded) = next.advance(quest_id, progress.cycle, Stage::Completed, Stage::Rewarded)
    else {
        eprintln!("world: quest {quest_id} could not be rewarded; nothing paid");
        return Ok(true);
    };
    // Bind the text before the write, for the same reason the offer prepares its batch first.
    let character_name: Vec<u16> = player.name.encode_utf16().collect();
    let Some(description) = shown
        .description
        .and_then(|text| bound(text, shown.argument, &character_name))
    else {
        eprintln!("world: quest {quest_id}'s description does not bind; nothing paid");
        return Ok(true);
    };
    let Some(objectives) = shown.objectives else {
        eprintln!("world: quest {quest_id} has no objectives text; nothing paid");
        return Ok(true);
    };
    if !db_quest::reward(
        &world.pool,
        player.character,
        quest_id,
        progress.cycle,
        experience,
        u16::try_from(level).unwrap_or(u16::MAX),
        skill_points,
        skill_points_total,
        reward.gold,
    )
    .await
    .map_err(GameError::Database)?
    {
        eprintln!(
            "world: {}'s turn-in of quest {quest_id} was not stored; nothing paid",
            player.name
        );
        return Ok(true);
    }
    player.quests = rewarded;
    player.traits.experience = experience;
    player.traits.level = u16::try_from(level).unwrap_or(u16::MAX);
    player.traits.skill_points = skill_points;
    player.traits.skill_points_total = skill_points_total;
    player.traits.gold = gold;
    let mut frames = vec![
        wire::quest_marker(
            quest_id,
            shown.marker.x(),
            shown.marker.y(),
            shown.marker.plane,
            shown.map_to,
        ),
        wire::quest_description(quest_id, &description, objectives),
        wire::quest_remove(quest_id),
    ];
    // The capture sends Sir Tydus's marker again after the removal: 5 while another quest is on offer,
    // 0 when this one was the last. The offer giver is refreshed for the same reason.
    let mut names = vec![reward.npc.as_str()];
    if let Some(block) = &quest.offer {
        names.push(block.npc.as_str());
    }
    frames.extend(marker_frames(player, map_id, &names));
    // The measured award shape: `0x00EE {attribute 0, delta}`. A quest reward in this pack crosses at
    // most one level (the level-2 threshold is 2000 and the largest reward here is 500), which is the
    // shape the kill and hand-in captures carry (`docs/54-experience.md`).
    if experience > before.experience {
        frames.push(asura_protocol::game::skill::agent_kill_reward(
            asura_protocol::game::skill::ATTRIBUTE_EXPERIENCE,
            experience - before.experience,
        ));
    }
    if reward.gold > 0 {
        frames.push(asura_protocol::game::vendor::gold_character_add(
            crate::item::ITEM_STREAM,
            reward.gold,
        ));
    }
    // The skills a turn-in teaches (a profession test's two, for instance), after the row is stored.
    frames.extend(crate::skill::character::teach(world, player, &reward.skills).await);
    // Close the dialog after the reward, as the talk path does: the speaker, then the talked-to
    // property.
    frames.push(asura_protocol::game::dialog::dialog_sender(npc));
    frames.push(crate::vendor::protocol::talked_to(npc));
    send(stream, session, frames, trace).await?;
    eprintln!(
        "world: {} turned in quest {quest_id} at {npc_name:?}: {} experience, {} gold, level {}",
        player.name, reward.experience, reward.gold, level
    );
    Ok(true)
}

/// `GAME_CMSG_QUEST_REQUEST_INFOS` (`0x8012`): the client asks for the marker of a quest in its log.
///
/// It asks once for every entry it is given, including right after a load whose markers it already
/// has. The live server sends each marker once, inside the load (four-professions-2026-10-11 packets,
/// quest 222 in map 146: `0x0053` at 81525390, nothing after), and every repeat is a sound in the
/// client - five quests, five sounds at once. So a quest whose marker this connection already sent
/// (in the load, or in an acceptance's `0x0049`, which carries it) is not answered again
/// (`Player::quest_markers_sent`); one that was not gets the marker alone, `0x0053`, as preforged
/// answers its marker request (`gw_quest_restore.h`, `marker_reply_2022`). Never the entry: that made
/// the client ask again, without end. A quest the character does not hold active or completed, or
/// without a marker, is not answered.
pub async fn handle_info_request(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(quest_id) = wire::read_quest_info_request(frame.body()) else {
        eprintln!(
            "world: a quest marker request of {} bytes does not read; ignored",
            frame.body().len()
        );
        return Ok(true);
    };
    let Some(progress) = player
        .quests
        .records
        .get(&quest_id)
        .filter(|progress| matches!(progress.stage, Stage::Active | Stage::Completed))
    else {
        if trace {
            eprintln!(
                "world: {} asked for quest {quest_id}'s marker, which it does not hold",
                player.name
            );
        }
        return Ok(true);
    };
    let Some(shown) = current()
        .and_then(|file| file.find(quest_id))
        .and_then(|quest| shown(quest, progress))
    else {
        eprintln!("world: quest {quest_id} is held but the pack has no content for it");
        return Ok(true);
    };
    let (x, y) = (shown.marker.x(), shown.marker.y());
    if !x.is_finite() || !y.is_finite() {
        return Ok(true);
    }
    if player.quest_markers_sent.contains(&quest_id) {
        if trace {
            eprintln!(
                "world: {} asked for quest {quest_id}'s marker, which it already has",
                player.name
            );
        }
        return Ok(true);
    }
    let marker = wire::quest_add_marker(quest_id, x, y, shown.marker.plane, shown.map_to);
    player.quest_markers_sent.insert(quest_id);
    send(stream, session, vec![marker], trace).await?;
    if trace {
        eprintln!("world: sent {}'s marker for quest {quest_id}", player.name);
    }
    Ok(true)
}

/// `GAME_CMSG_QUEST_ABANDON` (`0x8011`): the player abandoned a quest from the log.
///
/// The rule decides whether the attempt may be abandoned (active or completed, not rewarded); the row is
/// written before anything is sent. The client then gets the removal, and the offer and turn-in givers
/// on this map get their markers again: the offer reopens, so the giver shows its marker, and the turn-in
/// closes.
pub async fn handle_abandon(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(quest_id) = wire::read_quest_abandon(frame.body()) else {
        eprintln!(
            "world: a quest abandon of {} bytes does not read; ignored",
            frame.body().len()
        );
        return Ok(true);
    };
    let Some(progress) = player.quests.records.get(&quest_id).cloned() else {
        eprintln!(
            "world: {} abandoned quest {quest_id}, which has no attempt; ignored",
            player.name
        );
        return Ok(true);
    };
    let Some(next) = player.quests.abandon(quest_id, progress.cycle) else {
        eprintln!(
            "world: {}'s quest {quest_id} is {:?}; it cannot be abandoned",
            player.name, progress.stage
        );
        return Ok(true);
    };
    if !db_quest::abandon(&world.pool, player.character, quest_id, progress.cycle)
        .await
        .map_err(GameError::Database)?
    {
        eprintln!(
            "world: {}'s abandon of quest {quest_id} was not stored; nothing sent",
            player.name
        );
        return Ok(true);
    }
    player.quests = next;
    let mut frames = vec![wire::quest_remove(quest_id)];
    if let Some(quest) = current().and_then(|file| file.find(quest_id)) {
        let map_id = player.instance.map.map_id;
        let mut names = Vec::new();
        if let Some(block) = quest.offer.as_ref().filter(|block| block.map == map_id) {
            names.push(block.npc.as_str());
        }
        if let Some(block) = quest.reward.as_ref().filter(|block| block.map == map_id) {
            names.push(block.npc.as_str());
        }
        frames.extend(marker_frames(player, map_id, &names));
    }
    send(stream, session, frames, trace).await?;
    eprintln!(
        "world: {} abandoned quest {quest_id} (cycle {})",
        player.name, progress.cycle
    );
    Ok(true)
}

/// `GAME_CMSG_SEND_DIALOG`'s talk objective: the player delivered what the quest asked, at `npc_name`.
///
/// A quest whose objective is a conversation (Unsettling Rumors' scribe, A Second Profession's trainer,
/// The Wayward Wizard's brother) completes here: the attempt moves from active to completed, the row is
/// written first, and the log gets the completed text and marker (`0x0054`, `0x004D`, the pair preforged
/// sends for a talk objective) and the givers their markers. A choice that does not complete anything
/// (an attempt that is not active) is logged and changes nothing.
#[allow(clippy::too_many_arguments)]
pub async fn handle_credit(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    quest_id: u32,
    npc_name: &str,
    map_id: i32,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(quest) = current().and_then(|file| file.find(quest_id)) else {
        eprintln!("world: a talk objective names quest {quest_id}, which the pack does not carry");
        return Ok(true);
    };
    let Some(progress) = player.quests.records.get(&quest_id).cloned() else {
        return Ok(true);
    };
    if progress.stage != Stage::Active {
        if trace {
            eprintln!(
                "world: {}'s quest {quest_id} is {:?}; the talk at {npc_name:?} completes nothing",
                player.name, progress.stage
            );
        }
        return Ok(true);
    }
    let Some(next) =
        player
            .quests
            .advance(quest_id, progress.cycle, Stage::Active, Stage::Completed)
    else {
        return Ok(true);
    };
    let Some(completed) = next.records.get(&quest_id).cloned() else {
        return Ok(true);
    };
    let shown = shown(quest, &completed);
    if !db_quest::complete(&world.pool, player.character, quest_id, progress.cycle)
        .await
        .map_err(GameError::Database)?
    {
        eprintln!(
            "world: {}'s talk objective for quest {quest_id} was not stored; nothing sent",
            player.name
        );
        return Ok(true);
    }
    player.quests = next;
    let mut frames = Vec::new();
    if let Some(shown) = shown {
        if let Some(objectives) = shown.objectives {
            frames.push(wire::quest_objectives(quest_id, objectives));
        }
        frames.push(wire::quest_marker(
            quest_id,
            shown.marker.x(),
            shown.marker.y(),
            shown.marker.plane,
            shown.map_to,
        ));
    }
    let mut names = Vec::new();
    if let Some(block) = quest.offer.as_ref().filter(|block| block.map == map_id) {
        names.push(block.npc.as_str());
    }
    if let Some(block) = quest.reward.as_ref().filter(|block| block.map == map_id) {
        names.push(block.npc.as_str());
    }
    frames.extend(marker_frames(player, map_id, &names));
    send(stream, session, frames, trace).await?;
    eprintln!(
        "world: {} completed quest {quest_id}'s objective at {npc_name:?}",
        player.name
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character with no profession yet, so no profession-bound offer is open to it.
    fn nobody(quests: &Profile) -> Who<'_> {
        Who {
            quests,
            primary: 0,
            secondary: 0,
            level: 1,
        }
    }

    fn file() -> QuestFile {
        QuestFile::from_json(include_str!("../../../config/World/Vanilla/quests.json"))
            .expect("the committed quest file")
    }

    #[test]
    fn the_committed_file_carries_the_captured_message_from_a_friend() {
        let file = file();
        assert_eq!(file.quests.len(), 48);
        let quest = file.find(80).expect("quest 80");
        assert_eq!(quest.state, 1);
        assert_eq!(quest.map_from, 148);
        assert_eq!(quest.map_to, 148);
        assert_eq!(quest.marker.x_bits, 1178012672, "0x46370c00, x 11715");
        assert_eq!(quest.marker.y_bits, 1163644928, "0x455bd000, y 3517");
        assert_eq!(quest.marker.plane, 26);
        assert_eq!(quest.location, vec![15716]);
        assert_eq!(quest.name, vec![5891, 43118, 40603, 19586]);
        assert_eq!(quest.npc, vec![13172, 42252, 54956, 8813]);
        let description = quest.description.as_ref().expect("details");
        assert_eq!(description.len(), 38);
        assert_eq!(description[0..4], [5894, 63503, 44219, 20386]);
        assert_eq!(
            quest.character_argument,
            Some("Logger".encode_utf16().collect::<Vec<_>>())
        );
        let offer = quest.offer.as_ref().expect("the Town Crier offer");
        assert_eq!(offer.npc, "Town Crier");
        assert_eq!(offer.map, 148);
        assert_eq!(
            offer.requires_none_completed,
            vec![35, 217, 218, 219, 220, 221, 222]
        );
        let reward = quest.reward.as_ref().expect("the Sir Tydus reward");
        assert_eq!(reward.npc, "Sir Tydus");
        assert_eq!(reward.map, 148);
        assert_eq!(reward.experience, 100);
        assert_eq!(reward.gold, 10);
    }

    #[test]
    fn quest_80_active_is_add_then_details_with_the_name_bound() {
        let file = file();
        let quest = file.find(80).expect("quest 80");
        let progress = Progress {
            cycle: 1,
            stage: Stage::Active,
            credits: Default::default(),
        };
        let name: Vec<u16> = "Ari".encode_utf16().collect();
        let frames = add_and_details(quest, &progress, &name).expect("the active batch");
        let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
        assert_eq!(
            headers,
            vec![
                asura_protocol::packets::game::GAME_SMSG_QUEST_ADD,
                asura_protocol::packets::game::GAME_SMSG_QUEST_DESCRIPTION
            ]
        );
        let body = frames[0].body();
        let state = u32::from_le_bytes(body[16..20].try_into().expect("four bytes"));
        assert_eq!(state & 3, 0, "the restore owns bits 1 and 2");
        // The details' first counted string is the description; the name argument is now Ari's.
        let description = frames[1].body();
        let count = usize::from(u16::from_le_bytes([description[4], description[5]]));
        let units = &description[6..6 + 2 * count];
        let units: Vec<u16> = units
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let argument = [0x0107, 0x0041, 0x0072, 0x0069, 0x0001];
        assert!(
            units
                .windows(argument.len())
                .any(|window| window == argument),
            "the character argument is not bound"
        );
    }

    #[test]
    fn a_rewarded_attempt_is_not_restored() {
        let mut profile = Profile::default();
        profile.records.insert(
            80,
            Progress {
                cycle: 1,
                stage: Stage::Rewarded,
                credits: Default::default(),
            },
        );
        let log = load_log(&profile, "Ari");
        assert!(log.entries.is_empty() && log.markers.is_empty() && log.details.is_empty());
    }

    #[test]
    fn a_load_restores_quietly_and_an_acceptance_announces() {
        // A load rebuilds the log inside the load, as the live server does (four-professions-2026-10-11,
        // quest 222): general info, marker, details. Only an acceptance sends the add.
        let profile = Profile::default()
            .accept(80, false, &limits())
            .expect("accepted");
        let log = load_log(&profile, "Ari");
        let headers = |frames: &[Frame]| {
            frames
                .iter()
                .map(|frame| frame.header())
                .collect::<Vec<_>>()
        };
        assert_eq!(headers(&log.entries), vec![0x0050]);
        assert_eq!(headers(&log.markers), vec![0x0053]);
        assert_eq!(headers(&log.details), vec![0x004C]);
        let quest = current().and_then(|file| file.find(80)).expect("quest 80");
        let accepted = add_and_details(quest, &profile.records[&80], &[0x41]).expect("the batch");
        assert_eq!(headers(&accepted), vec![0x0049, 0x004C]);
    }

    #[test]
    fn stored_rows_become_the_rule_profile() {
        let rows = vec![
            db_quest::Row {
                quest_id: 80,
                cycle: 1,
                stage: 1,
            },
            db_quest::Row {
                quest_id: 55,
                cycle: 2,
                stage: 4,
            },
        ];
        let loaded = profile(rows).expect("two rows");
        assert_eq!(loaded.records[&80].stage, Stage::Active);
        assert_eq!(loaded.records[&55].stage, Stage::Abandoned);
        assert_eq!(loaded.records[&55].cycle, 2);
        assert!(profile(vec![db_quest::Row {
            quest_id: 80,
            cycle: 1,
            stage: 9,
        }])
        .is_err());
    }

    #[test]
    fn the_town_crier_offers_and_sir_tydus_takes_the_turn_in() {
        let empty = Profile::default();
        assert_eq!(
            marker_for(&nobody(&empty), "Town Crier", 148),
            QuestMarker::Offer
        );
        assert_eq!(
            marker_for(&nobody(&empty), "Sir Tydus", 148),
            QuestMarker::None
        );
        assert_eq!(
            marker_for(&nobody(&empty), "Town Crier", 146),
            QuestMarker::None,
            "another map"
        );
        let accepted = empty.accept(80, false, &limits()).expect("accepted");
        assert_eq!(
            marker_for(&nobody(&accepted), "Town Crier", 148),
            QuestMarker::None
        );
        assert_eq!(
            marker_for(&nobody(&accepted), "Sir Tydus", 148),
            QuestMarker::TurnIn
        );
        let rewarded = accepted
            .advance(80, 1, Stage::Active, Stage::Completed)
            .and_then(|profile| profile.advance(80, 1, Stage::Completed, Stage::Rewarded))
            .expect("rewarded");
        assert_eq!(
            marker_for(&nobody(&rewarded), "Town Crier", 148),
            QuestMarker::None
        );
        assert_eq!(
            marker_for(&nobody(&rewarded), "Sir Tydus", 148),
            QuestMarker::None
        );
        // The shipped settings: the capture's 5 and 4.
        assert_eq!(marker_value(QuestMarker::Offer), 5);
        assert_eq!(marker_value(QuestMarker::TurnIn), 4);
    }

    #[test]
    fn a_marker_is_cleared_with_property_12_as_the_live_server_does() {
        let property = |frame: &Frame| {
            let body = frame.body();
            let word = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
            (word(0), word(4), word(8))
        };
        assert_eq!(
            property(&marker_update(72, QuestMarker::Offer)),
            (11, 72, 5)
        );
        assert_eq!(
            property(&marker_update(40, QuestMarker::TurnIn)),
            (11, 40, 4)
        );
        // Never {11, npc, 0}: the client draws that.
        assert_eq!(property(&marker_update(72, QuestMarker::None)), (12, 72, 0));
    }

    #[test]
    fn war_preparations_is_offered_to_its_own_profession_only() {
        let empty = Profile::default();
        let warrior = Who {
            quests: &empty,
            primary: 1,
            secondary: 0,
            level: 1,
        };
        let monk = Who {
            primary: 3,
            ..warrior
        };
        // 221 is the warrior's, 220 the monk's; both are Sir Tydus's offers.
        assert!(offer_open(&warrior, 221));
        assert!(!offer_open(&warrior, 220));
        assert!(offer_open(&monk, 220));
        assert_eq!(marker_for(&warrior, "Sir Tydus", 148), QuestMarker::Offer);
        assert_eq!(
            marker_for(&nobody(&empty), "Sir Tydus", 148),
            QuestMarker::None
        );
        // The Warrior Test is Van's, and only once nothing bars it.
        assert!(offer_open(&warrior, 85));
        assert!(!offer_open(&monk, 85));
    }

    #[test]
    fn a_talk_objective_holds_the_turn_in_until_it_is_done() {
        // Unsettling Rumors: Devona offers after a profession test, Meerak's talk completes it, Armin
        // takes the turn-in only then.
        let empty = Profile::default();
        let tested = empty
            .accept(85, false, &limits())
            .and_then(|p| p.advance(85, 1, Stage::Active, Stage::Completed))
            .and_then(|p| p.advance(85, 1, Stage::Completed, Stage::Rewarded))
            .expect("the warrior test rewarded");
        fn warrior(quests: &Profile) -> Who<'_> {
            Who {
                quests,
                primary: 1,
                secondary: 0,
                level: 2,
            }
        }
        assert!(!offer_open(&warrior(&empty), 62), "no profession test yet");
        assert!(offer_open(&warrior(&tested), 62));
        let accepted = tested.accept(62, false, &limits()).expect("accepted");
        assert!(quest_active(&warrior(&accepted), 62));
        assert!(
            !reward_open(&warrior(&accepted), 62),
            "the scribe has not been asked"
        );
        let told = accepted
            .advance(62, 1, Stage::Active, Stage::Completed)
            .expect("completed");
        assert!(reward_open(&warrior(&told), 62));
        assert_eq!(
            marker_for(&warrior(&told), "Armin Saberlin", 148),
            QuestMarker::TurnIn
        );
    }

    #[test]
    fn only_the_givers_and_turn_ins_are_quest_npcs() {
        assert!(is_quest_npc("Town Crier", 148));
        assert!(is_quest_npc("Sir Tydus", 148));
        assert!(!is_quest_npc("Sir Tydus", 146), "another map");
        assert!(!is_quest_npc("The Purveyor [Bonus Merchant]", 148));
    }

    #[test]
    fn an_abandon_reopens_the_offer_and_closes_the_turn_in() {
        let accepted = Profile::default()
            .accept(80, false, &limits())
            .expect("accepted");
        let abandoned = accepted.abandon(80, 1).expect("abandoned");
        assert_eq!(
            marker_for(&nobody(&abandoned), "Town Crier", 148),
            QuestMarker::Offer
        );
        assert_eq!(
            marker_for(&nobody(&abandoned), "Sir Tydus", 148),
            QuestMarker::None
        );
        // And it can be taken again, at the next cycle.
        let again = abandoned.accept(80, false, &limits()).expect("retaken");
        assert_eq!(again.records[&80].stage, Stage::Active);
        assert_eq!(
            marker_for(&nobody(&again), "Sir Tydus", 148),
            QuestMarker::TurnIn
        );
    }

    #[test]
    fn half_an_offer_reward_pair_is_refused() {
        let broken = r#"{
            "quests": [
                {"id": 7, "state": 1, "marker": {"x_bits": 0, "y_bits": 0, "plane": 0},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1], "npc": [1],
                 "offer": {"npc": "Someone", "map": 1}}
            ]
        }"#;
        let error = QuestFile::from_json(broken).expect_err("half a pair");
        assert!(error.contains("offer/reward pair"), "{error}");
    }

    #[test]
    fn a_completed_variant_keeps_its_own_marker() {
        let file = file();
        let quest = file.find(37).expect("quest 37");
        let completed = quest.completed.as_ref().expect("completed details");
        assert_eq!(
            completed.marker.expect("the captured marker").x_bits,
            3321032704
        );
        assert_eq!(completed.map_to, Some(161));
        assert_ne!(
            completed.marker.expect("marker").x_bits,
            quest.marker.x_bits
        );
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        let broken = r#"{
            "quests": [
                {"id": 7, "state": 1, "marker": {"x_bits": 0, "y_bits": 0, "plane": 0},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1], "npc": [1]},
                {"id": 7, "state": 1, "marker": {"x_bits": 0, "y_bits": 0, "plane": 0},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1], "npc": [1]}
            ]
        }"#;
        let error = QuestFile::from_json(broken).expect_err("a duplicate");
        assert!(error.contains("named twice"), "{error}");
    }

    #[test]
    fn an_eight_unit_name_is_refused() {
        let broken = r#"{
            "quests": [
                {"id": 7, "state": 1, "marker": {"x_bits": 0, "y_bits": 0, "plane": 0},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1,1,1,1,1,1,1,1], "npc": [1]}
            ]
        }"#;
        let error = QuestFile::from_json(broken).expect_err("an overlong name");
        assert!(error.contains("7 plus a terminator"), "{error}");
    }

    #[test]
    fn half_a_details_pair_is_refused() {
        let broken = r#"{
            "quests": [
                {"id": 7, "state": 1, "marker": {"x_bits": 0, "y_bits": 0, "plane": 0},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1], "npc": [1],
                 "description": [1]}
            ]
        }"#;
        let error = QuestFile::from_json(broken).expect_err("half a pair");
        assert!(error.contains("only half"), "{error}");
    }

    #[test]
    fn a_nan_marker_is_refused() {
        let broken = r#"{
            "quests": [
                {"id": 7, "state": 1, "marker": {"x_bits": 2143289344, "y_bits": 2143289344, "plane": 0},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1], "npc": [1]}
            ]
        }"#;
        let error = QuestFile::from_json(broken).expect_err("a NaN marker");
        assert!(error.contains("positive-infinity pair"), "{error}");
    }

    #[test]
    fn the_no_marker_pair_is_accepted() {
        let marker = Marker {
            x_bits: 0x7F80_0000,
            y_bits: 0x7F80_0000,
            plane: 0,
        };
        assert!(marker.is_none());
        let good = format!(
            r#"{{"quests": [{{"id": 7, "state": 1, "marker": {{"x_bits": {}, "y_bits": {}, "plane": 0}},
                 "map_to": 1, "map_from": 1, "location": [1], "name": [1], "npc": [1]}}]}}"#,
            marker.x_bits, marker.y_bits
        );
        assert!(QuestFile::from_json(&good).is_ok());
    }
}
