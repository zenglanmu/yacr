//! Per-document display settings: SketchUp's **Shadows** and **Fog**.
//!
//! The record is document state (`tables/display.json`, `.ymodel` minor 22): a
//! sun (for shading), and a distance-fog band. It is **not** session chrome: a
//! reopened document keeps its sun and fog, and every carrier renders the same
//! values.
//!
//! # What is really modelled
//!
//! * **Shadows** here means the sun used for **shading**: the direction the
//!   existing single directional light comes from, plus on/off. Cast shadows
//!   (a shadow map, softness, ground-plane falloff) are **NOT IMPLEMENTED** and
//!   the tray panel says so.
//! * **Fog** is distance fog: geometry blends toward the fog colour between
//!   `start` and `end` model units from the camera.
//!
//! # Conventions
//!
//! Azimuth is degrees clockwise from world `+Y` (north) seen from above;
//! elevation is degrees above the horizon (`0` = horizontal, `90` = zenith).
//! The unit vector toward the sun is `(cos(elev)·sin(az), cos(elev)·cos(az),
//! sin(elev))`. Fog distances are model units and `end > start`; the colour is
//! sRGB bytes.

use serde_json::{Map, Value};

use crate::CadError;

/// Upper bound on the fog-ramp end, in model units (a sanity bound only).
pub const MAX_FOG_DISTANCE: f64 = 1.0e9;

/// Stable refusal markers.
pub mod markers {
    /// A shadow/fog name or numeric value is outside its accepted range.
    pub const DISPLAY_INVALID_VALUE: &str = "display.invalid_value";
    /// The serialized row holds a field this build does not model.
    pub const DISPLAY_INVALID_ROW: &str = "display.invalid_row";
}

/// The document's sun: on/off and its azimuth/elevation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    /// Whether the sun replaces the renderer's camera-relative key light.
    pub enabled: bool,
    /// Degrees clockwise from world `+Y`, in `[0, 360)`.
    pub azimuth: f64,
    /// Degrees above the horizon, in `[0, 90]`.
    pub elevation: f64,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        // SketchUp's default sun is roughly south-east and high; the exact
        // value is ours, not a vendor setting.
        ShadowSettings {
            enabled: false,
            azimuth: 135.0,
            elevation: 45.0,
        }
    }
}

impl ShadowSettings {
    /// The unit vector **toward** the sun, or `None` when shadows are off.
    pub fn sun_direction(&self) -> Option<[f64; 3]> {
        if !self.enabled {
            return None;
        }
        let azimuth = self.azimuth.to_radians();
        let elevation = self.elevation.to_radians();
        Some([
            elevation.cos() * azimuth.sin(),
            elevation.cos() * azimuth.cos(),
            elevation.sin(),
        ])
    }

    fn validate(&self) -> Result<(), CadError> {
        if !self.azimuth.is_finite() || !(0.0..360.0).contains(&self.azimuth) {
            return Err(invalid(format!(
                "the sun azimuth must be a finite angle in [0, 360), got {}",
                self.azimuth
            )));
        }
        if !self.elevation.is_finite() || !(0.0..=90.0).contains(&self.elevation) {
            return Err(invalid(format!(
                "the sun elevation must be a finite angle in [0, 90], got {}",
                self.elevation
            )));
        }
        Ok(())
    }
}

/// The document's distance fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogSettings {
    /// Whether fog is rendered.
    pub enabled: bool,
    /// Distance where the fog begins, in model units (`>= 0`).
    pub start: f64,
    /// Distance where geometry is fully fogged (`> start`).
    pub end: f64,
    /// sRGB fog colour.
    pub color: [u8; 3],
}

impl Default for FogSettings {
    fn default() -> Self {
        FogSettings {
            enabled: false,
            start: 100.0,
            end: 1000.0,
            color: [200, 210, 220],
        }
    }
}

impl FogSettings {
    /// The linear fog colour and its enabled flag, or `None` when fog is off.
    pub fn frame_fog(&self) -> Option<([f32; 3], f32, f32)> {
        if !self.enabled {
            return None;
        }
        let channel = |byte: u8| byte as f32 / 255.0;
        Some((
            [
                channel(self.color[0]),
                channel(self.color[1]),
                channel(self.color[2]),
            ],
            self.start as f32,
            self.end as f32,
        ))
    }

    fn validate(&self) -> Result<(), CadError> {
        if !self.start.is_finite() || self.start < 0.0 || self.start > MAX_FOG_DISTANCE {
            return Err(invalid(format!(
                "the fog start must be finite and in [0, {MAX_FOG_DISTANCE}], got {}",
                self.start
            )));
        }
        if !self.end.is_finite() || self.end <= self.start || self.end > MAX_FOG_DISTANCE {
            return Err(invalid(format!(
                "the fog end must be finite, greater than the start and at most \
                 {MAX_FOG_DISTANCE}, got {}",
                self.end
            )));
        }
        Ok(())
    }
}

/// The whole per-document display record.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DisplaySettings {
    /// The sun used for shading.
    pub shadows: ShadowSettings,
    /// Distance fog.
    pub fog: FogSettings,
}

impl DisplaySettings {
    /// Whether the record is the default (nothing to persist).
    pub fn is_default(&self) -> bool {
        *self == DisplaySettings::default()
    }

    /// Validate every value.
    pub fn validate(&self) -> Result<(), CadError> {
        self.shadows.validate()?;
        self.fog.validate()
    }

    /// Encode one table row.
    pub fn to_json(&self) -> Value {
        let mut shadows = Map::new();
        shadows.insert("enabled".into(), Value::Bool(self.shadows.enabled));
        shadows.insert("azimuth".into(), Value::from(self.shadows.azimuth));
        shadows.insert("elevation".into(), Value::from(self.shadows.elevation));
        let mut fog = Map::new();
        fog.insert("enabled".into(), Value::Bool(self.fog.enabled));
        fog.insert("start".into(), Value::from(self.fog.start));
        fog.insert("end".into(), Value::from(self.fog.end));
        fog.insert(
            "color".into(),
            Value::Array(self.fog.color.iter().map(|c| Value::from(*c)).collect()),
        );
        let mut row = Map::new();
        row.insert("shadows".into(), Value::Object(shadows));
        row.insert("fog".into(), Value::Object(fog));
        Value::Object(row)
    }

    /// Decode one row strictly (unknown keys and wrong types are refused).
    pub fn from_json(value: &Value) -> Result<Self, CadError> {
        let Value::Object(row) = value else {
            return Err(invalid_row("row: expected a JSON object"));
        };
        let shadows = match row.get("shadows") {
            Some(Value::Object(s)) => {
                let enabled = take_bool(s, "shadows", "enabled")?;
                let azimuth = take_f64(s, "shadows", "azimuth")?;
                let elevation = take_f64(s, "shadows", "elevation")?;
                refuse_unknown(s, "shadows")?;
                ShadowSettings {
                    enabled,
                    azimuth,
                    elevation,
                }
            }
            Some(_) => return Err(invalid_row("shadows: expected an object")),
            None => return Err(invalid_row("shadows: missing")),
        };
        let fog = match row.get("fog") {
            Some(Value::Object(f)) => {
                let enabled = take_bool(f, "fog", "enabled")?;
                let start = take_f64(f, "fog", "start")?;
                let end = take_f64(f, "fog", "end")?;
                let color = match f.get("color") {
                    Some(Value::Array(channels)) if channels.len() == 3 => {
                        let mut color = [0_u8; 3];
                        for (index, channel) in channels.iter().enumerate() {
                            let value = channel
                                .as_i64()
                                .filter(|v| (0..=255).contains(v))
                                .ok_or_else(|| {
                                    invalid_row(format!(
                                        "fog.color: channel {index} must be an integer 0..=255"
                                    ))
                                })?;
                            color[index] = value as u8;
                        }
                        color
                    }
                    Some(_) => return Err(invalid_row("fog.color: expected three channels")),
                    None => return Err(invalid_row("fog.color: missing")),
                };
                refuse_unknown(f, "fog")?;
                FogSettings {
                    enabled,
                    start,
                    end,
                    color,
                }
            }
            Some(_) => return Err(invalid_row("fog: expected an object")),
            None => return Err(invalid_row("fog: missing")),
        };
        if let Some(key) = row.keys().find(|key| *key != "shadows" && *key != "fog") {
            return Err(invalid_row(format!("unknown field {key:?}")));
        }
        let settings = DisplaySettings { shadows, fog };
        settings.validate()?;
        Ok(settings)
    }
}

fn take_bool(object: &Map<String, Value>, section: &str, key: &str) -> Result<bool, CadError> {
    match object.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(invalid_row(format!("{section}.{key}: expected a boolean"))),
        None => Err(invalid_row(format!("{section}.{key}: missing"))),
    }
}

fn take_f64(object: &Map<String, Value>, section: &str, key: &str) -> Result<f64, CadError> {
    match object.get(key) {
        Some(Value::Number(value)) => value
            .as_f64()
            .ok_or_else(|| invalid_row(format!("{section}.{key}: expected a finite number"))),
        Some(_) => Err(invalid_row(format!("{section}.{key}: expected a number"))),
        None => Err(invalid_row(format!("{section}.{key}: missing"))),
    }
}

fn refuse_unknown(object: &Map<String, Value>, section: &str) -> Result<(), CadError> {
    let known: &[&str] = match section {
        "shadows" => &["enabled", "azimuth", "elevation"],
        _ => &["enabled", "start", "end", "color"],
    };
    if let Some(key) = object.keys().find(|key| !known.contains(&key.as_str())) {
        return Err(invalid_row(format!("{section}: unknown field {key:?}")));
    }
    Ok(())
}

fn invalid(detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{}: {detail}", markers::DISPLAY_INVALID_VALUE))
}

fn invalid_row(detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{}: {detail}", markers::DISPLAY_INVALID_ROW))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_vector_points_from_the_azimuth_and_elevation() {
        // Azimuth 0, elevation 0: toward world +Y.
        let sun = ShadowSettings {
            enabled: true,
            azimuth: 0.0,
            elevation: 0.0,
        };
        let direction = sun.sun_direction().expect("enabled");
        assert!((direction[0]).abs() < 1e-12, "{direction:?}");
        assert!((direction[1] - 1.0).abs() < 1e-12, "{direction:?}");
        assert!((direction[2]).abs() < 1e-12, "{direction:?}");
        // Azimuth 90, elevation 0: toward world +X.
        let east = ShadowSettings {
            enabled: true,
            azimuth: 90.0,
            elevation: 0.0,
        };
        let direction = east.sun_direction().expect("enabled");
        assert!((direction[0] - 1.0).abs() < 1e-12, "{direction:?}");
        assert!(direction[1].abs() < 1e-12, "{direction:?}");
        // Zenith: straight up, unit length.
        let zenith = ShadowSettings {
            enabled: true,
            azimuth: 12.0,
            elevation: 90.0,
        };
        let direction = zenith.sun_direction().expect("enabled");
        assert!((direction[2] - 1.0).abs() < 1e-12, "{direction:?}");
        assert!(direction[0].abs() < 1e-9 && direction[1].abs() < 1e-9);
        // Shadows off has no sun at all.
        assert_eq!(ShadowSettings::default().sun_direction(), None);
    }

    #[test]
    fn the_default_record_is_not_written_and_validates() {
        let default = DisplaySettings::default();
        assert!(default.is_default());
        default.validate().expect("the default is valid");
        assert!(!FogSettings::default().enabled);
        assert_eq!(FogSettings::default().frame_fog(), None);
    }

    #[test]
    fn a_fog_record_round_trips_and_sanitises_the_band() {
        let settings = DisplaySettings {
            shadows: ShadowSettings {
                enabled: true,
                azimuth: 45.0,
                elevation: 20.0,
            },
            fog: FogSettings {
                enabled: true,
                start: 10.0,
                end: 20.0,
                color: [1, 2, 3],
            },
        };
        let decoded = DisplaySettings::from_json(&settings.to_json()).expect("round trip");
        assert_eq!(decoded, settings);
        assert!(!settings.is_default());
        let (color, start, end) = settings.fog.frame_fog().expect("fog enabled");
        assert!((color[0] - 1.0 / 255.0).abs() < 1e-6);
        assert_eq!((start, end), (10.0, 20.0));
    }

    #[test]
    fn an_out_of_range_record_is_refused_with_a_stable_marker() {
        let bad_azimuth = DisplaySettings {
            shadows: ShadowSettings {
                enabled: false,
                azimuth: 360.0,
                elevation: 10.0,
            },
            ..DisplaySettings::default()
        };
        let error = bad_azimuth.validate().expect_err("azimuth 360");
        assert!(error.to_string().contains(markers::DISPLAY_INVALID_VALUE));

        let bad_fog = DisplaySettings {
            fog: FogSettings {
                enabled: true,
                start: 20.0,
                end: 10.0,
                color: [0, 0, 0],
            },
            ..DisplaySettings::default()
        };
        bad_fog.validate().expect_err("end <= start");

        let wrong_type = serde_json::json!({
            "shadows": {"enabled": "yes", "azimuth": 0.0, "elevation": 0.0},
            "fog": {"enabled": false, "start": 1.0, "end": 2.0, "color": [0, 0, 0]}
        });
        let error = DisplaySettings::from_json(&wrong_type).expect_err("bool as string");
        assert!(error.to_string().contains(markers::DISPLAY_INVALID_ROW));

        let unknown = serde_json::json!({
            "shadows": {"enabled": false, "azimuth": 0.0, "elevation": 0.0, "softness": 1},
            "fog": {"enabled": false, "start": 1.0, "end": 2.0, "color": [0, 0, 0]}
        });
        DisplaySettings::from_json(&unknown).expect_err("unknown field");
    }
}
