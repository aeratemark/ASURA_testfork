//! Game rules, as decisions the server makes before a world exists.
//!
//! This crate exists because the auth service was answering a game question
//! (`docs/38-layer-decisions.md`, D25). Login has to say *where* a character enters the game, but
//! "where does a character start" is a rule of the game, not of the protocol, so it lives here and
//! both services can call it.
//!
//! Every function here is pure: the caller reads the rows, hands them in, and gets a decision back.
//! Nothing in this crate opens a connection, reads a file or prints. That is what keeps it callable
//! from the auth service, from the world, and from a test with no database.
//!
//! The rules are the owner's, recorded in `docs/22-world-model.md` ("Spawn points, and the outpost
//! rule") and `docs/38-layer-decisions.md` (D18, D19, D34).
//!
//! It holds decisions and no values. A number that describes the world - where a character enters, how
//! old a world's heartbeat may be - is configuration (`config/Server/`), handed in as an argument; the
//! numbers a client defines, like its map kinds, are the wire layer's (D34).

/// Titles: the rung a track is on, what a load and a point change send, and whether a title may be shown.
pub mod titles;

/// What the client asked for: a map id and the map type it named with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requested {
    /// The instance the client named.
    pub map_id: i32,
    /// The map type it named alongside it: 0 outpost, 1 explorable, 11 creation instance.
    pub map_type: u32,
}

/// What the character's rows say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Character {
    /// The character's map from its own row (`characters.map_id`): where it was written.
    pub map_id: i32,
    /// The last **outpost** it was placed in (`characters.last_outpost`), 0 when it never was.
    pub last_outpost: i32,
}

/// Why a placement answer differs from what the client asked for. The caller logs it; the rule does
/// not print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Substituted {
    /// The client named the creation instance as a place; the character's own map is the answer.
    CreationNamedAsPlace(&'static str),
    /// The destination is not an outpost, and the outpost rule put the character in its last one.
    NotAnOutpost,
    /// The character's row says it was never placed, and no rule here invents a place for it.
    NothingToPlace,
}

/// Whether a name the creation confirm parsed is one to create a character with.
///
/// A name with nothing in it is refused: a character the client cannot list is not a character. Whether
/// the name is *free* is the database's question and is not asked here - this is the rule, and it takes
/// the string the caller already parsed (`docs/38-layer-decisions.md`, D34).
pub fn name_is_acceptable(name: &str) -> bool {
    !name.trim().is_empty()
}

/// The map a character is placed in when it enters the game.
///
/// Three inputs decide it, and the order matters:
///
/// 1. A creation visit (`map_type` 11, map 0) is never substituted: the client is opening the creation
///    screen, not entering the world.
/// 2. A client that names the creation instance as a place (`map_type` 0, map 0) is asking the
///    mistaken question that follows a creation confirm. The answer is the character's own row, then
///    its last outpost, then [`fallback_outpost`] - never map 0, which has no map file and leaves the
///    client on a loading screen.
/// 3. Any other destination that is not a main town is replaced by the last outpost, because entering
///    the game puts the character in the last outpost it visited; with no last outpost, again the
///    fallback.
///
/// `kinds` is what the client calls a town, an explorable area and the creation instance, read from
/// `config/Server/defaults.json`. It arrives as an argument for the same reason the other two values
/// do: this crate holds no value of its own.
///
/// `creation_map` is the creation instance's map id, from `config/Server/defaults.json`
/// (`docs/38-layer-decisions.md`, D34): the one destination that is a conversation rather than a
/// place, and the value a row carries when nothing has been written. It arrives as an argument for
/// the same reason the fallback outpost does - this crate holds no value of its own.
///
/// `fallback_outpost` is where a character with no placement history is put. It is configuration - one
/// number in `config/Server/placement.json` - and it arrives as an argument, because the outpost a
/// character enters is written by character creation, by a scripted event and by travelling, never by
/// a rule. This function holds no value of its own (`docs/38-layer-decisions.md`, D34).
///
/// `destination_type` is the map type of the map being asked for, as the caller read it, or `None`
/// when the map is not in the catalogue - an unknown map is left alone for the caller to refuse.
pub fn placement(
    request: Requested,
    character: Character,
    destination_type: Option<i32>,
    fallback_outpost: i32,
    creation_map: i32,
    kinds: asura_protocol::game::MapKinds,
) -> (i32, Option<Substituted>) {
    // 1. A creation visit is its own destination.
    if request.map_id == creation_map && request.map_type == kinds.character_creation as u32 {
        return (request.map_id, None);
    }

    // 2. The mistaken creation request: map 0 named as a place.
    if request.map_id == creation_map {
        if character.map_id != creation_map {
            return (
                character.map_id,
                Some(Substituted::CreationNamedAsPlace("the character's row")),
            );
        }
        if character.last_outpost != creation_map {
            return (
                character.last_outpost,
                Some(Substituted::CreationNamedAsPlace("its last outpost")),
            );
        }
        // The row names no map and no outpost: the caller's fallback is the answer, not one of ours.
        return (fallback_outpost, Some(Substituted::NothingToPlace));
    }

    // 3. The outpost rule, for every destination that is not a main town.
    let Some(map_type) = destination_type else {
        return (request.map_id, None);
    };
    if map_type == kinds.main_town {
        return (request.map_id, None);
    }
    if character.last_outpost == creation_map {
        // Never placed, so there is no last outpost to substitute: the fallback, as above.
        return (fallback_outpost, Some(Substituted::NothingToPlace));
    }
    if character.last_outpost == request.map_id {
        (request.map_id, None)
    } else {
        (character.last_outpost, Some(Substituted::NotAnOutpost))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The creation instance's map id, as the committed defaults file states it.
    const CREATION: i32 = 0;

    /// The client's map kinds, as the committed defaults file states them.
    const KINDS: asura_protocol::game::MapKinds = asura_protocol::game::MapKinds {
        main_town: 0,
        main_explorable: 1,
        character_creation: 11,
    };

    const FRESH: Character = Character {
        map_id: CREATION,
        last_outpost: CREATION,
    };

    #[test]
    fn a_creation_visit_reaches_the_creation_instance() {
        let request = Requested {
            map_id: CREATION,
            map_type: KINDS.character_creation as u32,
        };
        assert_eq!(
            placement(request, FRESH, Some(11), 148, CREATION, KINDS),
            (CREATION, None)
        );
    }

    #[test]
    fn the_mistaken_creation_request_goes_to_the_characters_row() {
        // What the client sends after a creation confirm.
        let request = Requested {
            map_id: CREATION,
            map_type: KINDS.main_town as u32,
        };
        let character = Character {
            map_id: 212,
            last_outpost: CREATION,
        };
        let (map, why) = placement(request, character, Some(11), 148, CREATION, KINDS);
        assert_eq!(map, 212, "the map the confirm wrote");
        assert_eq!(
            why,
            Some(Substituted::CreationNamedAsPlace("the character's row"))
        );
    }

    #[test]
    fn a_character_with_no_row_map_falls_back_to_its_last_outpost() {
        let request = Requested {
            map_id: CREATION,
            map_type: KINDS.main_town as u32,
        };
        let character = Character {
            map_id: CREATION,
            last_outpost: 163,
        };
        assert_eq!(
            placement(request, character, Some(11), 148, CREATION, KINDS).0,
            163
        );
    }

    #[test]
    fn a_character_with_no_history_gets_the_fallback_it_was_handed() {
        // The outpost a character enters is written by creation, by a scripted event and by travelling.
        // A row that says none of that happened has no map to be placed in, and this rule does not make
        // one up (D34).
        let request = Requested {
            map_id: CREATION,
            map_type: KINDS.main_town as u32,
        };
        assert_eq!(
            placement(request, FRESH, Some(11), 148, CREATION, KINDS),
            (148, Some(Substituted::NothingToPlace))
        );
    }

    #[test]
    fn an_outpost_request_is_answered_as_asked() {
        let request = Requested {
            map_id: 163,
            map_type: KINDS.main_town as u32,
        };
        let character = Character {
            map_id: 163,
            last_outpost: 163,
        };
        assert_eq!(
            placement(request, character, Some(0), 148, CREATION, KINDS),
            (163, None)
        );
    }

    #[test]
    fn an_explorable_request_goes_to_the_last_outpost() {
        let request = Requested {
            map_id: 146,
            map_type: 1,
        };
        let character = Character {
            map_id: 146,
            last_outpost: 148,
        };
        assert_eq!(
            placement(request, character, Some(1), 148, CREATION, KINDS),
            (148, Some(Substituted::NotAnOutpost))
        );
    }

    #[test]
    fn a_character_that_never_saw_an_outpost_and_asks_for_an_explorable_gets_the_fallback() {
        let request = Requested {
            map_id: 146,
            map_type: 1,
        };
        assert_eq!(
            placement(request, FRESH, Some(1), 148, CREATION, KINDS),
            (148, Some(Substituted::NothingToPlace))
        );
    }

    #[test]
    fn a_name_with_nothing_in_it_is_refused() {
        assert!(!name_is_acceptable(""));
        assert!(!name_is_acceptable("   "));
        assert!(!name_is_acceptable("\u{00a0}"));
        assert!(name_is_acceptable("Asura"));
        assert!(name_is_acceptable("A"));
        assert!(name_is_acceptable("Asura One"));
    }

    #[test]
    fn an_unknown_map_is_left_for_the_caller_to_refuse() {
        let request = Requested {
            map_id: 999,
            map_type: 1,
        };
        assert_eq!(
            placement(request, FRESH, None, 148, CREATION, KINDS),
            (999, None)
        );
    }
}
