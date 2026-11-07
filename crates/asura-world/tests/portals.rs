//! Doorways come only from the selected pack; no SQL or compiled doorway fallback.
use asura_world::portals;

#[test]
fn pack_doorways_are_the_only_travel_content() {
    let arch = portals::pack_arches(148).expect("pack arches")[0];
    let middle = (
        (arch.from.0 + arch.to.0) / 2.0,
        (arch.from.1 + arch.to.1) / 2.0,
    );
    let dx = arch.to.0 - arch.from.0;
    let dy = arch.to.1 - arch.from.1;
    let length = dx.hypot(dy);
    let from = (middle.0 - dy / length, middle.1 + dx / length);
    let to = (middle.0 + dy / length, middle.1 - dx / length);
    assert_eq!(
        portals::crossed(148, from, to).unwrap().to_instance,
        arch.to_instance
    );
    assert!(portals::crossed(-1, from, to).is_none());
    assert!(portals::arrival(-1, 148).is_none());
}

#[test]
fn the_reverse_doors_name_the_map_they_come_from() {
    // 165 (Foibles Fair) and 166 (Fort Ranik) each have one door to an explorable. Upstream main
    // names the source map as itself, which cannot match the transfer; these are the corrected
    // sources, and the arrival point is the one the pre-301a97a pack stated.
    assert!(portals::arrival(165, 161).is_some(), "161 -> 165");
    assert!(portals::arrival(166, 162).is_some(), "162 -> 166");
}
