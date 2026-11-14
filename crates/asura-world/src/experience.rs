//! The experience curve, read from the pack.
//!
//! The rule is `asura_rules::experience::Curve`; this module is only the file reader and the once-per-
//! process copy, the same shape `chat.rs` uses. A missing or broken file refuses the command that needs
//! it and leaves the load block's stored values alone, rather than inventing a curve.

/// The file inside a pack that holds the experience curve.
pub const EXPERIENCE_FILE: &str = "experience.json";

/// The pack's `experience.json`, as it is written.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Rules {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// Total experience level 1 to 2 costs.
    pub first_level_xp: crate::character_defaults::Value<u32>,
    /// How much more each later level costs than the one before it.
    pub xp_per_level_step: crate::character_defaults::Value<u32>,
    /// The highest level the curve reaches.
    pub max_level: crate::character_defaults::Value<u32>,
    /// Skill points a level grants.
    pub skill_points_per_level: crate::character_defaults::Value<u32>,
}

/// Read `<pack>/experience.json` into the rule the world calls.
pub fn load(pack: &std::path::Path) -> Result<asura_rules::experience::Curve, String> {
    let path = pack.join(EXPERIENCE_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    let rules: Rules = serde_json::from_str(&text)
        .map_err(|err| format!("{} is not an experience file: {err}", path.display()))?;
    if rules.first_level_xp.value == 0 {
        return Err(format!(
            "{}: first_level_xp must be greater than zero",
            path.display()
        ));
    }
    if rules.max_level.value == 0 {
        return Err(format!(
            "{}: max_level must be greater than zero",
            path.display()
        ));
    }
    Ok(asura_rules::experience::Curve {
        first_level_xp: rules.first_level_xp.value,
        xp_per_level_step: rules.xp_per_level_step.value,
        max_level: rules.max_level.value,
        skill_points_per_level: rules.skill_points_per_level.value,
    })
}

/// The curve in force, read once per process. A missing or broken file is logged once and answers
/// `None`; `/exp` is then refused with a logged reason.
pub fn curve() -> Option<&'static asura_rules::experience::Curve> {
    static IN_FORCE: std::sync::OnceLock<Option<asura_rules::experience::Curve>> =
        std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load(&pack) {
                Ok(curve) => {
                    eprintln!(
                        "asura: experience curve from {}: level 2 at {}, level {} at {}",
                        pack.display(),
                        curve.threshold(2),
                        curve.max_level,
                        curve.max_total()
                    );
                    Some(curve)
                }
                Err(err) => {
                    eprintln!("asura: {err}; experience changes are refused");
                    None
                }
            }
        })
        .as_ref()
}
