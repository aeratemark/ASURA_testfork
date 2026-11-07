//! Geometry values for directional movement, separate from report reconciliation.
use crate::Described;
use std::sync::OnceLock;

/// Distances used when tracing manual movement across connected surfaces.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionalMovement {
    /// Extra distance beyond the offset supplied by the client.
    pub distance_slack: Described<f32>,
    /// Contact precision in map units.
    pub edge_tolerance: Described<f32>,
}

/// Read once; missing or invalid geometry configuration is reported and refused.
pub fn current() -> Result<&'static DirectionalMovement, String> {
    static SETTINGS: OnceLock<Result<DirectionalMovement, String>> = OnceLock::new();
    SETTINGS
        .get_or_init(|| {
            let path = crate::resolve("config/Server/directional_movement.json");
            let result = std::fs::read_to_string(&path)
                .map_err(|err| err.to_string())
                .and_then(|text| {
                    serde_json::from_str::<DirectionalMovement>(&text)
                        .map_err(|err| err.to_string())
                })
                .and_then(|value| {
                    if value.distance_slack.value.is_finite()
                        && value.distance_slack.value >= 0.0
                        && value.edge_tolerance.value.is_finite()
                        && value.edge_tolerance.value > 0.0
                    {
                        Ok(value)
                    } else {
                        Err("directional movement distances must be finite; slack must be nonnegative and edge tolerance positive".to_string())
                    }
                });
            if let Err(err) = &result {
                eprintln!("asura: {}: {err}", path.display());
            }
            result
        })
        .as_ref()
        .map_err(Clone::clone)
}
