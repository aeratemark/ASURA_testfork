//! The doorway table's search order, over the real tables.
//!
//! One property, and it is the one that broke: a doorway read from the database is an *addition* to the
//! measured crossings, never a replacement. 14 of the 22 positions in `CROSSED` have no `map_portal` row
//! within `portals::REACH` of them, so a table that substituted the database rows for the whole table would
//! stop those crossings firing without saying so - which happened on 2026-11-05 and read as movement losing
//! sync (`docs/HANDOFF.md` decisions).
//!
//! This lives in `tests/` because `portals::install` is process-global: a unit test next to the table would
//! install rows for every other test in that binary.

use asura_world::portals::{self, Portal};

#[test]
fn a_measured_crossing_still_answers_when_the_database_supplies_doorways() {
    // A measured crossing: map 146 at (7198, 5177) leads to Ascalon City. Nothing is installed yet, so this
    // is the compiled table answering, which is what the world did before the table existed.
    let measured = portals::portal_at(146, 7198.0, 5177.0).expect("the measured crossing");
    assert_eq!(measured.to_instance, Some(148));
    assert_eq!(
        measured.arrive,
        Some((7739.0, 5900.0)),
        "its measured landing point"
    );

    // Install a doorway somewhere else entirely, as `main` does at start-up from `map_portal`.
    assert!(
        portals::install(vec![Portal {
            from_instance: 146,
            x: 9000.0,
            y: -9000.0,
            to_instance: Some(160),
            arrive: None,
        }]),
        "the first install is the one that counts"
    );

    // The measured crossing is still the first answer, unchanged.
    let again = portals::portal_at(146, 7198.0, 5177.0).expect("still the measured crossing");
    assert_eq!(again.to_instance, Some(148));
    assert_eq!(again.arrive, Some((7739.0, 5900.0)));

    // And the installed row is reachable, which is the whole point of reading it.
    let installed = portals::portal_at(146, 9000.0, -9000.0).expect("the installed doorway");
    assert_eq!(installed.to_instance, Some(160));

    // A walk through the measured door still crosses it, with the database in the table.
    let walked = portals::portal_along(146, (7100.0, 5100.0), (7300.0, 5250.0))
        .expect("a walk through the measured door");
    assert_eq!(walked.to_instance, Some(148));
}
