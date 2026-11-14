//! NPC dialogs: the greeting an NPC says when talked to, the choices under it, and what a choice does.
//!
//! The wire is `asura_protocol::game::dialog`: one `0x0080` body (split at 121 units), `0x0081` naming
//! the speaker, one `0x007E` per choice, then `0x009F {12, npc, 0}`. The live server's own dialog, its
//! seven choices and two of their answers are `recordings/client/feature/movement/`
//! `movement-live-20261109-200703.jsonl` seq 5176-5185 and 5261-5288 (`docs/55-npc-dialogs.md`).
//!
//! What an NPC says is content: `config/World/<pack>/dialogs.json` names a map, an NPC and its pages.
//! The body and the choice labels are **encoded strings** - the client's own string ids and control
//! codes - so the file stores the units the capture carried, not text this server composed. A pack
//! author can write literal text too: `GAME_SMSG_DIALOG_BODY` accepts the `0x0108 0x0107 <text> 0x0001`
//! form preforged's `dialogue::open` uses (`gw_npc_dialogue.h`).

use crate::*;

/// The file inside a pack that holds dialogs.
pub const DIALOG_FILE: &str = "dialogs.json";

/// The most choices one page may carry (`gw_npc_dialogue.h`: `buttons.size() > 32` is invalid).
pub const MAX_BUTTONS: usize = 32;

/// A pack's `dialogs.json`, as it is written.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct DialogFile {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// What each key means.
    #[serde(default)]
    pub fields: Option<std::collections::BTreeMap<String, String>>,
    /// The dialogs, one per map and NPC.
    pub dialogs: Vec<Dialog>,
}

/// One NPC's dialog on one map: its pages, in the order a choice or the talk opens them.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Dialog {
    /// The map the NPC stands on.
    pub map: i32,
    /// The placement's literal name, with or without its bracketed title.
    pub npc: String,
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// How near, in map units, a player must come for a talk to be answered, when this NPC needs more
    /// than the movement file's `interact_range`: one that stands on a stand or behind a counter the
    /// player cannot walk onto. Absent means the usual range.
    #[serde(default)]
    pub talk_range: Option<f32>,
    /// One or more pages; the first is what a talk opens.
    pub pages: Vec<Page>,
}

/// One dialog page: the body, the choices under it, and when the page is shown.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Page {
    /// The `0x0080` units, as the capture carried them.
    pub body: Vec<u16>,
    /// The `0x007E` choices, in the order they are drawn.
    pub buttons: Vec<Button>,
    /// When the page is open. Absent means always.
    #[serde(default)]
    pub when: Option<When>,
}

/// The state a page or a choice waits for. A quest page is only shown while its offer or turn-in is
/// open; the capture was taken in that state, and the live server would not have shown the same page
/// otherwise. The offer and turn-in conditions are the quest pack's own (`crate::quest::offer_open`,
/// `crate::quest::reward_open`): level, profession and prerequisite quests included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum When {
    /// The quest can be accepted now.
    QuestOfferOpen {
        /// The client's quest id.
        quest: u32,
    },
    /// The quest can be turned in now.
    QuestRewardOpen {
        /// The client's quest id.
        quest: u32,
    },
    /// The quest is accepted and its objective not yet complete.
    QuestActive {
        /// The client's quest id.
        quest: u32,
    },
    /// Only as the answer to a choice: never what a talk opens.
    Answer,
}

impl When {
    /// Whether the condition holds for this character.
    pub fn holds(&self, who: &crate::quest::Who) -> bool {
        match *self {
            When::QuestOfferOpen { quest } => crate::quest::offer_open(who, quest),
            When::QuestRewardOpen { quest } => crate::quest::reward_open(who, quest),
            When::QuestActive { quest } => crate::quest::quest_active(who, quest),
            When::Answer => false,
        }
    }

    fn quest(&self) -> Option<u32> {
        match *self {
            When::QuestOfferOpen { quest }
            | When::QuestRewardOpen { quest }
            | When::QuestActive { quest } => Some(quest),
            When::Answer => None,
        }
    }
}

impl Page {
    /// Whether a talk may open this page for this character: its own condition holds, and a page that
    /// has choices still has one to show (a quest menu whose every quest is closed is not shown).
    pub fn is_open(&self, who: &crate::quest::Who) -> bool {
        if !self.when.is_none_or(|when| when.holds(who)) {
            return false;
        }
        self.buttons.is_empty() || self.buttons.iter().any(|button| button.visible(who))
    }

    /// The choices this character is shown.
    pub fn visible<'a: 'w, 'w>(
        &'a self,
        who: &'w crate::quest::Who<'w>,
    ) -> impl Iterator<Item = &'a Button> + 'w {
        self.buttons
            .iter()
            .filter(move |button| button.visible(who))
    }

    /// Whether the page offers or rewards a quest: what the dialog reopens on after a quest step, as the
    /// live server opened Sir Tydus's War Preparations offer 750 ms after Message from a Friend's
    /// turn-in.
    pub fn has_quest_step(&self, who: &crate::quest::Who) -> bool {
        self.visible(who).any(|button| {
            matches!(
                button.action,
                Action::Quest {
                    step: QuestStep::Offer | QuestStep::Reward,
                    ..
                }
            )
        })
    }
}

impl Button {
    /// Whether this character is shown the choice.
    pub fn visible(&self, who: &crate::quest::Who) -> bool {
        self.when.is_none_or(|when| when.holds(who))
    }
}

/// One choice: its icon, its encoded label, the id the client sends back, the skill it names, and what
/// the server does when it is chosen.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Button {
    /// The icon the client draws (`0x007E`'s first byte).
    pub icon: u8,
    /// The encoded label units.
    pub label: Vec<u16>,
    /// The id the client sends back in `0x803B`.
    pub id: u32,
    /// The skill the choice names; the live server sent 0 on one choice and `0xFFFFFFFF` on the rest.
    #[serde(default = "no_skill")]
    pub skill: u32,
    /// What choosing it does.
    pub action: Action,
    /// When the choice is shown. Absent means always.
    #[serde(default)]
    pub when: Option<When>,
}

/// What choosing a dialog button does.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Close the dialog and answer nothing.
    Close,
    /// Open a window of this kind, owned by the NPC (`0x0083`).
    Window { window: u8 },
    /// Open the NPC's merchant window, through `crate::vendor`'s own content and rules.
    Merchant,
    /// Show another page of this dialog.
    Page { page: usize },
    /// A quest button: offer the quest, take its turn-in, or complete its talk objective
    /// (`docs/58-quests.md`).
    Quest {
        /// The client's quest id.
        quest: u32,
        /// Whether the choice offers the quest, rewards it, or completes its objective.
        #[serde(default)]
        step: QuestStep,
        /// The page shown after the step, when the capture shows one (a trainer's reply to a delivered
        /// message). Absent: the dialog reopens on a page that offers or rewards a quest, if one is open.
        #[serde(default)]
        page: Option<usize>,
    },
    /// A choice whose answer the capture does not show. Logged, and nothing opens.
    Unanswered,
}

/// Which end of a quest a dialog choice is: the offer or the turn-in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestStep {
    /// The NPC offers the quest.
    #[default]
    Offer,
    /// The NPC takes the turn-in.
    Reward,
    /// Talking to this NPC is the quest's objective; the choice completes it.
    Credit,
}

/// The skill field of a choice that names no skill, as the live server sends it.
fn no_skill() -> u32 {
    asura_protocol::game::dialog::DIALOG_NO_SKILL
}

impl DialogFile {
    /// Parse and check the file's text.
    pub fn from_json(text: &str) -> Result<DialogFile, String> {
        let file: DialogFile = serde_json::from_str(text).map_err(|err| err.to_string())?;
        file.check()?;
        Ok(file)
    }

    /// Check every dialog: a name and a map, at least one page, at most 32 choices, unique ids and a
    /// page index that exists.
    fn check(&self) -> Result<(), String> {
        for dialog in &self.dialogs {
            if dialog.map <= 0 {
                return Err(format!("{}: map must be positive", dialog.npc));
            }
            if dialog.npc.trim().is_empty() {
                return Err("a dialog has no NPC name".to_string());
            }
            if dialog.pages.is_empty() {
                return Err(format!("{}: no pages", dialog.npc));
            }
            if let Some(range) = dialog.talk_range {
                if !range.is_finite() || range <= 0.0 {
                    return Err(format!(
                        "{}: talk_range must be a finite number above 0",
                        dialog.npc
                    ));
                }
            }
            for (index, page) in dialog.pages.iter().enumerate() {
                if page.body.is_empty() {
                    return Err(format!("{} page {index}: empty body", dialog.npc));
                }
                if page.when.and_then(|when| when.quest()) == Some(0) {
                    return Err(format!(
                        "{} page {index}: its condition names quest 0",
                        dialog.npc
                    ));
                }
                if page.buttons.len() > MAX_BUTTONS {
                    return Err(format!(
                        "{} page {index}: {} choices, the client takes at most {MAX_BUTTONS}",
                        dialog.npc,
                        page.buttons.len()
                    ));
                }
                let mut ids = std::collections::BTreeSet::new();
                for button in &page.buttons {
                    if button.id == 0 {
                        return Err(format!("{} page {index}: a choice id is 0", dialog.npc));
                    }
                    if !ids.insert(button.id) {
                        return Err(format!(
                            "{} page {index}: choice id {:#x} is named twice",
                            dialog.npc, button.id
                        ));
                    }
                    if button.label.is_empty()
                        || button.label.len() > asura_protocol::game::dialog::DIALOG_LABEL_UNITS
                    {
                        return Err(format!(
                            "{} page {index}: choice {:#x} has {} label units",
                            dialog.npc,
                            button.id,
                            button.label.len()
                        ));
                    }
                    if let Action::Window { window: 0 } = button.action {
                        return Err(format!(
                            "{} page {index}: choice {:#x} opens window kind 0",
                            dialog.npc, button.id
                        ));
                    }
                    if let Some(When::Answer) = button.when {
                        return Err(format!(
                            "{} page {index}: choice {:#x} waits for answer, which only a page can",
                            dialog.npc, button.id
                        ));
                    }
                    if button.when.and_then(|when| when.quest()) == Some(0) {
                        return Err(format!(
                            "{} page {index}: choice {:#x}'s condition names quest 0",
                            dialog.npc, button.id
                        ));
                    }
                    if let Action::Quest {
                        page: Some(target), ..
                    } = button.action
                    {
                        if target >= dialog.pages.len() {
                            return Err(format!(
                                "{} page {index}: choice {:#x} names page {target}, and there are {}",
                                dialog.npc,
                                button.id,
                                dialog.pages.len()
                            ));
                        }
                    }
                    if let Action::Quest { quest: 0, .. } = button.action {
                        return Err(format!(
                            "{} page {index}: choice {:#x} names quest 0",
                            dialog.npc, button.id
                        ));
                    }
                    if let Action::Page { page: target } = button.action {
                        if target >= dialog.pages.len() {
                            return Err(format!(
                                "{} page {index}: choice {:#x} names page {target}, and there are {}",
                                dialog.npc,
                                button.id,
                                dialog.pages.len()
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The dialog an NPC called `name` has on `map_id`, if any. A bracketed title such as
    /// `[Bonus Merchant]` is not part of the name, as `crate::vendor::content::base_name` defines it.
    pub fn find(&self, map_id: i32, name: &str) -> Option<&Dialog> {
        let name = crate::vendor::content::base_name(name);
        self.dialogs.iter().find(|dialog| {
            dialog.map == map_id && crate::vendor::content::base_name(&dialog.npc) == name
        })
    }
}

impl Page {
    /// The frames a talk (or a page choice) sends: the body, the speaker, every choice, and the
    /// `0x009F {12, npc, 0}` the live server sends after an answer to a talk.
    pub fn frames(&self, npc: u32, who: &crate::quest::Who) -> Vec<Frame> {
        let mut frames = asura_protocol::game::dialog::dialog_body(&self.body);
        frames.push(asura_protocol::game::dialog::dialog_sender(npc));
        for button in self.visible(who) {
            frames.push(asura_protocol::game::dialog::dialog_button(
                button.icon,
                &button.label,
                button.id,
                button.skill,
            ));
        }
        frames.push(crate::vendor::protocol::talked_to(npc));
        frames
    }
}

/// The talk range the pack gives the NPC called `name` on `map_id`, if it gives one.
pub fn talk_range_for(map_id: i32, name: &str) -> Option<f32> {
    current()?.find(map_id, name)?.talk_range
}

/// Read `<pack>/dialogs.json`.
pub fn load(pack: &std::path::Path) -> Result<DialogFile, String> {
    let path = pack.join(DIALOG_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    DialogFile::from_json(&text)
        .map_err(|err| format!("{} is not a dialogs file: {err}", path.display()))
}

/// The dialogs in force, read once per process. A missing or broken file is logged once and answers
/// `None`; a talk then falls back to the NPC's window, which is what every NPC had before this.
pub fn current() -> Option<&'static DialogFile> {
    static IN_FORCE: std::sync::OnceLock<Option<DialogFile>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load(&pack) {
                Ok(dialogs) => {
                    eprintln!(
                        "asura: {} dialog(s) from {}",
                        dialogs.dialogs.len(),
                        pack.display()
                    );
                    Some(dialogs)
                }
                Err(err) => {
                    eprintln!("asura: {err}; NPC dialogs are off");
                    None
                }
            }
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use asura_protocol::packets::game as pkt;

    /// The captured dialog: The Purveyor [Bonus Merchant, map 148, agent 0x36 in the live session.
    fn file() -> DialogFile {
        DialogFile::from_json(include_str!("../../../config/World/Vanilla/dialogs.json"))
            .expect("the committed dialogs file")
    }

    #[test]
    fn the_purveyor_dialog_is_the_captured_one() {
        let file = file();
        let dialog = file
            .find(148, "The Purveyor [Bonus Merchant")
            .expect("The Purveyor's dialog");
        let page = &dialog.pages[0];
        // seq 5176: [0x8102, 0x3E7E, 0xD7B5, 0xDBE4, 0x49F2].
        assert_eq!(
            page.body,
            vec![33026, 15998, 55221, 56292, 18930],
            "the captured body"
        );
        // seq 5178-5184, in the order the live server sent them.
        let ids: Vec<u32> = page.buttons.iter().map(|button| button.id).collect();
        assert_eq!(ids, vec![0x8C, 0x87, 0x8D, 0x8E, 0x8F, 0x8B, 0x85]);
        let empty = asura_rules::quest::Profile::default();
        let frames = page.frames(0x36, &warrior(&empty));
        let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
        assert_eq!(
            headers,
            vec![
                pkt::GAME_SMSG_DIALOG_BODY,
                pkt::GAME_SMSG_DIALOG_SENDER,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_DIALOG_BUTTON,
                pkt::GAME_SMSG_AGENT_PROPERTY_UPDATE_INT,
            ],
            "body, sender, seven choices, talked-to"
        );
        // The first body's wire bytes, from the builder's own test: header, count 5, the units.
        assert_eq!(
            frames[0].encode(),
            vec![0x80, 0x00, 5, 0, 0x02, 0x81, 0x7E, 0x3E, 0xB5, 0xD7, 0xE4, 0xDB, 0xF2, 0x49]
        );
        // The first choice: icon 28, two label units, id 0x8C, skill 0.
        assert_eq!(
            frames[2].encode(),
            vec![
                0x7E, 0x00, 28, 2, 0, 0x02, 0x81, 0x47, 0x3B, 0x8C, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn a_dialog_choice_may_not_repeat_an_id() {
        let duplicate = r#"{
            "dialogs": [{
                "map": 148,
                "npc": "Someone",
                "pages": [{"body": [1], "buttons": [
                    {"icon": 1, "label": [1], "id": 7, "action": {"kind": "close"}},
                    {"icon": 1, "label": [1], "id": 7, "action": {"kind": "close"}}
                ]}]
            }]
        }"#;
        let error = DialogFile::from_json(duplicate).expect_err("a duplicate id");
        assert!(error.contains("named twice"), "{error}");
    }

    #[test]
    fn a_page_choice_must_name_a_page_that_exists() {
        let broken = r#"{
            "dialogs": [{
                "map": 148,
                "npc": "Someone",
                "pages": [{"body": [1], "buttons": [
                    {"icon": 1, "label": [1], "id": 7, "action": {"kind": "page", "page": 2}}
                ]}]
            }]
        }"#;
        let error = DialogFile::from_json(broken).expect_err("a page that does not exist");
        assert!(error.contains("names page 2"), "{error}");
    }

    #[test]
    fn sir_tydus_is_talked_to_from_off_his_stand() {
        let file = file();
        let tydus = file.find(148, "Sir Tydus").expect("Sir Tydus");
        assert!(tydus.talk_range.is_some_and(|range| range > 50.0));
        let crier = file.find(148, "Town Crier").expect("the Town Crier");
        assert_eq!(crier.talk_range, None, "the usual range");
        let broken = r#"{
            "dialogs": [{
                "map": 148,
                "npc": "Someone",
                "talk_range": 0,
                "pages": [{"body": [1], "buttons": []}]
            }]
        }"#;
        let error = DialogFile::from_json(broken).expect_err("a zero talk range");
        assert!(error.contains("talk_range"), "{error}");
    }

    fn warrior(quests: &asura_rules::quest::Profile) -> crate::quest::Who<'_> {
        crate::quest::Who {
            quests,
            primary: 1,
            secondary: 0,
            level: 1,
        }
    }

    /// The page a talk opens: the first open one.
    fn opened<'a>(dialog: &'a Dialog, who: &crate::quest::Who) -> Option<&'a Page> {
        dialog.pages.iter().find(|page| page.is_open(who))
    }

    #[test]
    fn the_town_crier_offers_quest_80_and_sir_tydus_rewards_it() {
        let file = file();
        let crier = file.find(148, "Town Crier").expect("the Town Crier");
        let tydus = file.find(148, "Sir Tydus").expect("Sir Tydus");
        let empty = asura_rules::quest::Profile::default();

        // A new warrior: the Town Crier offers quest 80 with its captured choices, and Sir Tydus offers
        // the warrior's War Preparations (221), not another profession's.
        let offer = opened(crier, &warrior(&empty)).expect("an offer");
        assert_eq!(offer.when, Some(When::QuestOfferOpen { quest: 80 }));
        assert_eq!(offer.buttons[0].id, 0x805001);
        assert_eq!(
            offer.buttons[0].action,
            Action::Quest {
                quest: 80,
                step: QuestStep::Offer,
                page: None
            }
        );
        assert_eq!(offer.buttons[1].action, Action::Close);
        let war = opened(tydus, &warrior(&empty)).expect("War Preparations");
        assert_eq!(war.when, Some(When::QuestOfferOpen { quest: 221 }));
        assert!(war.has_quest_step(&warrior(&empty)));

        // Quest 80 accepted: Sir Tydus takes the turn-in first, and the Town Crier's menu shows only its
        // captured reminder choice.
        let accepted = empty
            .accept(80, false, &crate::quest::limits())
            .expect("accepted");
        let turn_in = opened(tydus, &warrior(&accepted)).expect("the turn-in");
        assert_eq!(turn_in.when, Some(When::QuestRewardOpen { quest: 80 }));
        assert_eq!(turn_in.buttons[0].id, 0x805007);
        let menu = opened(crier, &warrior(&accepted)).expect("the menu");
        let shown: Vec<u32> = menu
            .visible(&warrior(&accepted))
            .map(|button| button.id)
            .collect();
        assert_eq!(shown, vec![0x805005]);
    }

    #[test]
    fn a_menu_with_nothing_to_show_is_not_opened() {
        // Devona's menu holds only quest choices; a new character is eligible for none of them.
        let file = file();
        let devona = file.find(146, "Devona").expect("Devona");
        let empty = asura_rules::quest::Profile::default();
        assert!(!devona.pages[0].is_open(&warrior(&empty)));
        assert!(opened(devona, &warrior(&empty)).is_none());
    }

    #[test]
    fn a_talk_objective_is_a_credit_step_with_its_reply() {
        let file = file();
        let meerak = file.find(164, "Meerak the Scribe").expect("Meerak");
        let button = &meerak.pages[0].buttons[0];
        assert_eq!(button.id, 0x803E04);
        assert_eq!(button.when, Some(When::QuestActive { quest: 62 }));
        let Action::Quest {
            quest: 62,
            step: QuestStep::Credit,
            page: Some(reply),
        } = button.action
        else {
            panic!("not a credit step: {:?}", button.action);
        };
        assert_eq!(meerak.pages[reply].when, Some(When::Answer));
    }

    #[test]
    fn every_generated_quest_choice_names_a_quest_the_pack_offers() {
        let file = file();
        let quests = crate::quest::current().expect("the quest pack");
        for dialog in &file.dialogs {
            for page in &dialog.pages {
                for button in &page.buttons {
                    if let Action::Quest { quest, step, .. } = button.action {
                        let content = quests.find(quest).expect("quest content");
                        match step {
                            QuestStep::Offer => assert!(content.offer.is_some(), "{quest}"),
                            QuestStep::Reward => assert!(content.reward.is_some(), "{quest}"),
                            QuestStep::Credit => assert!(content.reward.is_some(), "{quest}"),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_titled_name_is_found_by_its_literal_name() {
        let file = file();
        assert!(file.find(148, "The Purveyor [Bonus Merchant").is_some());
        assert!(file.find(148, "Sanura").is_none());
    }
}
