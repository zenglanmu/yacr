//! The typed argument lines of the 2026-11-07 utility/named-object batch:
//! `PURGE`, `RENAME`, `SETVAR`, `MREDO` and `MULTIPLE`.
//!
//! These are argument forms of their own canonical selectors, not new commands
//! and not a second IPC. `SETVAR` writes are deliberately **not** a second write
//! path: each writable variable is parsed here and then dispatched to the
//! existing `PDMODE`/`PDSIZE`/`LTSCALE`/`SNAPUNIT`/`GRID`/`SNAP`/`ORTHO`
//! `CommandId`, or to the existing session `set_tool_parameter` API for
//! `TEXTSIZE`. `SETVAR <name>` (no value) dispatches the read-only report.
//!
//! `MULTIPLE <command>` resolves the name against the one command vocabulary
//! before dispatching, so an unknown command is a named refusal rather than a
//! stored phantom.

use cad_app::{CommandId, CommandPayload, ExtendedCommandId, ToolParameter, ToolValue};
use cad_domain::Point3;
use cad_host::{CadSettingKey, CadSettingValue};

use super::commands::{
    invalid_mredo_argument_reason, invalid_multiple_argument_reason, invalid_purge_argument_reason,
    invalid_rename_argument_reason, invalid_setvar_value_reason,
    invalid_sysvarmonitor_argument_reason, parse_mredo_argument, parse_multiple_argument,
    parse_purge_argument, parse_rename_argument, parse_setvar_argument, parse_sysvar_toggle,
    parse_sysvarmonitor_argument, MREDO_SELECTOR, MULTIPLE_SELECTOR, PURGE_SELECTOR,
    RENAME_SELECTOR, SETVAR_SELECTOR, SYSVARMONITOR_SELECTOR,
};
use super::shell_settings::parse_display_mask;
use super::CadSession;

/// The closed set of variables `SETVAR` reports or writes.
const MODELLED_VARIABLES: &[&str] = &[
    "PDMODE",
    "PDSIZE",
    "SNAPUNIT",
    "LTSCALE",
    "GRID",
    "SNAP",
    "ORTHO",
    "TEXTSIZE",
    "WIPEOUTFRAME",
    // Wave 31: four values whose storage already existed but that `SETVAR` did
    // not list. Each one writes through its ordinary command (one setter per
    // value): `CVPORT` is session-only and writes **nothing** to the drawing,
    // while `INSBASE`/`LIMMIN`/`LIMMAX` are drawing-header values written by
    // `BASE`/`LIMITS`. The list mirrors the application's
    // `MODELLED_SYSTEM_VARIABLES` closed set (the reader the reports use), so a
    // variable is never writable here without being readable there.
    "CVPORT",
    "INSBASE",
    "LIMMIN",
    "LIMMAX",
    // The two session tokens the create funnel and `FS` read: `ELEV` (the
    // elevation new objects are created at) and `FSMODE` (the `FS` chain mode).
    // Both are session-only: the write goes through the very command the typed
    // `ELEV <height>` / `FSMODE ON|OFF` line dispatches and never touches the
    // drawing.
    "ELEV",
    "FSMODE",
];

/// The three shell settings `SETVAR` can **write** but not read.
///
/// They are persisted host preferences (`navvcubeDisplay`,
/// `viewCubeIn2d`, `viewCubeIn3d`), not session/drawing sysvars, so
/// `SETVAR <name>` (the report form) stays explicitly unmodelled: it returns a
/// clear "use the setter" refusal rather than a fabricated value.
const WRITE_ONLY_VARIABLES: &[&str] = &[
    "NAVVCUBEDISPLAY",
    "DISPLAYVIEWCUBEIN2D",
    "DISPLAYVIEWCUBEIN3D",
];

impl CadSession {
    /// Route one typed utility-batch line to its command path.
    pub(super) fn utility_argument(
        &self,
        selector: &'static str,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        match selector {
            PURGE_SELECTOR => self.purge_argument(argument, text),
            RENAME_SELECTOR => self.rename_argument(argument, text),
            SETVAR_SELECTOR => self.setvar_argument(argument, text),
            MREDO_SELECTOR => self.mredo_argument(argument, text),
            MULTIPLE_SELECTOR => self.multiple_argument(argument, text),
            SYSVARMONITOR_SELECTOR => self.sysvarmonitor_argument(argument, text),
            _ => self.refuse_argument(format!("unsupported utility selector: {selector}")),
        }
    }

    /// `PURGE <kind>`: dispatch the one named kind (a bare `PURGE` goes through
    /// the name-only path and purges every kind it can).
    fn purge_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(target) = parse_purge_argument(argument) else {
            return self.refuse_argument(invalid_purge_argument_reason(text));
        };
        self.dispatch_typed_argument(
            PURGE_SELECTOR,
            CommandId::Purge,
            CommandPayload::Purge(target),
        )
    }

    /// `RENAME <kind> <old> <new>`.
    fn rename_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(rename) = parse_rename_argument(argument) else {
            return self.refuse_argument(invalid_rename_argument_reason(text));
        };
        self.dispatch_typed_argument(
            RENAME_SELECTOR,
            CommandId::Rename,
            CommandPayload::Rename(rename),
        )
    }

    /// `MREDO <n>`.
    fn mredo_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(count) = parse_mredo_argument(argument) else {
            return self.refuse_argument(invalid_mredo_argument_reason(text));
        };
        self.dispatch_typed_argument(
            MREDO_SELECTOR,
            CommandId::MRedo,
            CommandPayload::MRedo(count),
        )
    }

    /// `MULTIPLE <command>`: resolve the target against the command vocabulary.
    fn multiple_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(name) = parse_multiple_argument(argument) else {
            return self.refuse_argument(invalid_multiple_argument_reason(text));
        };
        let Some(canonical) = super::canonical_command_name(&name) else {
            return self.refuse_argument(format!(
                "multiple.unknown_command: '{name}' maps to no admitted CAD command"
            ));
        };
        self.dispatch_typed_argument(
            MULTIPLE_SELECTOR,
            CommandId::Multiple,
            CommandPayload::Multiple(Some(canonical.to_string())),
        )
    }

    /// `SYSVARMONITOR ADD|REMOVE|LIST [<name>]`: one session watch-set action.
    fn sysvarmonitor_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some(action) = parse_sysvarmonitor_argument(argument) else {
            return self.refuse_argument(invalid_sysvarmonitor_argument_reason(text));
        };
        self.dispatch_typed_argument(
            SYSVARMONITOR_SELECTOR,
            CommandId::Extended(cad_app::ExtendedCommandId::SysVarMonitor),
            CommandPayload::SysVarMonitor(action),
        )
    }

    /// `SETVAR <name>` (report) or `SETVAR <name> <value>` (write through the
    /// existing variable handler).
    fn setvar_argument(&self, argument: &str, text: &str) -> Result<(), String> {
        let Some((name, value)) = parse_setvar_argument(argument) else {
            return self.refuse_argument(format!(
                "setvar.needs_name: '{text}' needs `<name>` or `<name> <value>`"
            ));
        };
        let name = name.to_ascii_uppercase();
        match value {
            // The write form is routed first: the three write-only shell
            // settings are writable even though their report form is refused.
            Some(value) => self.setvar_write(&name, &value),
            None => {
                if WRITE_ONLY_VARIABLES.contains(&name.as_str()) {
                    return self.refuse_argument(format!(
                        "setvar.write_only_variable: {name} is a persisted shell setting; \
                         use `{name} <value>`"
                    ));
                }
                if !MODELLED_VARIABLES.contains(&name.as_str()) {
                    return self.refuse_argument(format!(
                        "setvar.unknown_variable: SETVAR does not model '{name}'; the closed set is {}",
                        MODELLED_VARIABLES.join(", ")
                    ));
                }
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetVar,
                    CommandPayload::SetVar(name),
                )
            }
        }
    }

    /// Write one modelled variable through the **existing** handler.
    fn setvar_write(&self, name: &str, value: &str) -> Result<(), String> {
        let invalid = |expected: &str| {
            self.refuse_argument(invalid_setvar_value_reason(name, value, expected))
        };
        match name {
            "PDMODE" => {
                let Some(mode) = super::parse_pdmode_argument(value) else {
                    return invalid("one integer that fits i16");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetPointDisplayMode,
                    CommandPayload::PointDisplayMode(mode),
                )
            }
            "PDSIZE" => {
                let Some(size) = super::parse_pdsize_argument(value) else {
                    return invalid("one signed finite decimal");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetPointDisplaySize,
                    CommandPayload::PointDisplaySize(size),
                )
            }
            "SNAPUNIT" => {
                let Some(spacing) = super::parse_snap_unit_argument(value) else {
                    return invalid("one positive decimal spacing");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetSnapSpacing,
                    CommandPayload::SnapSpacing(spacing),
                )
            }
            "LTSCALE" => {
                let Some(scale) = super::parse_ltscale_argument(value) else {
                    return invalid("one positive finite decimal");
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::SetLinetypeScale,
                    CommandPayload::LinetypeScale(scale),
                )
            }
            // The three switches only have toggle commands. `SETVAR` needs
            // set-to-value semantics, so compare the current session state and
            // dispatch the ordinary toggle only when the requested value differs;
            // an already-matching value is reported, never flipped away.
            "GRID" | "SNAP" | "ORTHO" => {
                let Some(desired) = parse_sysvar_toggle(value) else {
                    return invalid("ON, OFF, 1 or 0");
                };
                let controller = self.core.controller.borrow();
                let current = match name {
                    "GRID" => controller.grid_enabled(),
                    "SNAP" => controller.snap_grid_enabled(),
                    _ => controller.ortho_enabled(),
                };
                drop(controller);
                if current == desired {
                    return self.dispatch_typed_argument(
                        SETVAR_SELECTOR,
                        CommandId::SetVar,
                        CommandPayload::SetVar(name.to_string()),
                    );
                }
                let id = match name {
                    "GRID" => CommandId::ToggleGrid,
                    "SNAP" => CommandId::ToggleSnapGrid,
                    _ => CommandId::ToggleOrtho,
                };
                self.dispatch_typed_argument(SETVAR_SELECTOR, id, CommandPayload::None)
            }
            "TEXTSIZE" => {
                let Some(height) = super::parse_snap_unit_argument(value) else {
                    return invalid("one positive finite text height");
                };
                self.core.record_command(SETVAR_SELECTOR);
                match self.set_tool_parameter(ToolParameter::TextHeight, ToolValue::Number(height))
                {
                    Ok(()) => {
                        *self.error.borrow_mut() = None;
                        Ok(())
                    }
                    Err(text) => {
                        *self.error.borrow_mut() = Some(text.clone());
                        Err(text)
                    }
                }
            }
            // `WIPEOUTFRAME` is session display state, so `SETVAR` writes it
            // through the very same command the typed `WIPEOUTFRAME <n>` line
            // uses — one setter, one diagnostic, one refusal for `0..=2` outside
            // the domain (never a clamp).
            "WIPEOUTFRAME" => match super::commands::parse_wipeoutframe_argument(value) {
                Ok(payload) => {
                    self.dispatch_typed_argument(SETVAR_SELECTOR, CommandId::WipeoutFrame, payload)
                }
                Err(reason) => self.refuse_argument(reason),
            },
            // `CVPORT` is the session's current-viewport marker: the write goes
            // through the same command the typed `CVPORT <n>` line uses and the
            // drawing database is untouched.
            "CVPORT" => match super::commands::parse_cvport_argument(value) {
                Ok(payload) => {
                    self.dispatch_typed_argument(SETVAR_SELECTOR, CommandId::Cvport, payload)
                }
                Err(reason) => self.refuse_argument(reason),
            },
            // `ELEV` and `FSMODE` are session tokens (the create funnel and `FS`
            // read them): each write rides the same command the typed setter line
            // dispatches, so there is exactly one setter and no drawing write.
            "ELEV" => match super::commands::parse_elev_argument(value) {
                Ok(payload) => self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::Extended(ExtendedCommandId::Elev),
                    payload,
                ),
                Err(reason) => self.refuse_argument(reason),
            },
            "FSMODE" => match super::commands::parse_fsmode_argument(value) {
                Ok(payload) => self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::Extended(ExtendedCommandId::FsMode),
                    payload,
                ),
                Err(reason) => self.refuse_argument(reason),
            },
            // `INSBASE` is the model-space insertion base point, written by the
            // ordinary `BASE` command (one header mutation, one revision).
            "INSBASE" => match parse_sysvar_point(value, 2, 3) {
                Some([x, y, z]) => self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::Extended(ExtendedCommandId::BasePoint),
                    CommandPayload::Points(vec![Point3 { x, y, z }]),
                ),
                _ => invalid("two or three decimals (`<x> <y> [<z>]`)"),
            },
            // `LIMMIN`/`LIMMAX` are the two corners of one rectangle: the payload
            // carries both, so the *unwritten* corner is read from the
            // authoritative drawing instead of being defaulted here. A drawing
            // that stores no other corner is refused by name — a half rectangle
            // is never invented.
            "LIMMIN" | "LIMMAX" => {
                let Some([x, y, _]) = parse_sysvar_point(value, 2, 2) else {
                    return invalid("two decimals (`<x> <y>`)");
                };
                let (min, max) = {
                    let controller = self.core.controller.borrow();
                    let Some(drawing) = controller.drawing() else {
                        return self.refuse_argument(
                            "setvar.no_document: SETVAR LIMMIN/LIMMAX needs an open drawing whose \
                             other limit corner can be read"
                                .to_string(),
                        );
                    };
                    (drawing.limits_min(), drawing.limits_max())
                };
                let (new_min, new_max) = if name == "LIMMIN" {
                    let Some(max) = max else {
                        return self.refuse_argument(format!(
                            "limits.other_corner_unset: SETVAR LIMMIN writes the minimum corner and                              keeps the stored LIMMAX, but this drawing stores no LIMMAX; set both                              corners with `LIMITS <min_x> <min_y> <max_x> <max_y>` (value {value:?}                              was not applied)"
                        ));
                    };
                    ([x, y], max)
                } else {
                    let Some(min) = min else {
                        return self.refuse_argument(format!(
                            "limits.other_corner_unset: SETVAR LIMMAX writes the maximum corner and                              keeps the stored LIMMIN, but this drawing stores no LIMMIN; set both                              corners with `LIMITS <min_x> <min_y> <max_x> <max_y>` (value {value:?}                              was not applied)"
                        ));
                    };
                    (min, [x, y])
                };
                self.dispatch_typed_argument(
                    SETVAR_SELECTOR,
                    CommandId::Extended(ExtendedCommandId::DrawingLimits),
                    CommandPayload::Points(vec![
                        Point3 {
                            x: new_min[0],
                            y: new_min[1],
                            z: 0.0,
                        },
                        Point3 {
                            x: new_max[0],
                            y: new_max[1],
                            z: 0.0,
                        },
                    ]),
                )
            }
            // The three persisted shell settings write through the same
            // validated/persisted settings path the `NAVVCUBEDISPLAY`
            // `DISPLAYVIEWCUBEIN2D` / `DISPLAYVIEWCUBEIN3D` selectors use. The
            // report form stays unmodelled (see `WRITE_ONLY_VARIABLES`), so this
            // never invents a read value.
            "NAVVCUBEDISPLAY" => {
                let Some(display) = parse_display_mask(value) else {
                    return invalid("0, 1, 2 or 3");
                };
                self.write_shell_setting(
                    SETVAR_SELECTOR,
                    CadSettingKey::NavvcubeDisplay,
                    CadSettingValue::Number(display),
                )
            }
            "DISPLAYVIEWCUBEIN2D" | "DISPLAYVIEWCUBEIN3D" => {
                let Some(enabled) = parse_sysvar_toggle(value) else {
                    return invalid("ON, OFF, 1 or 0");
                };
                let key = if name == "DISPLAYVIEWCUBEIN2D" {
                    CadSettingKey::ViewCubeIn2d
                } else {
                    CadSettingKey::ViewCubeIn3d
                };
                self.write_shell_setting(SETVAR_SELECTOR, key, CadSettingValue::Bool(enabled))
            }
            _ => self.refuse_argument(format!("setvar.unknown_variable: '{name}'")),
        }
    }
}

/// Parse a `SETVAR` point value: 2 or 3 finite plain decimals.
///
/// Components are separated by whitespace **or** a single comma, because both
/// spellings are natural on a command line (`SETVAR INSBASE 1,2` / `SETVAR
/// INSBASE 1 2`). An empty component is refused (it would shift the axes).
/// The count is bounded by the caller; anything else (an exponent, a sign-less
/// fraction of words, an empty component) is refused, never repaired. A missing
/// `z` is `0.0` — the only default, and the one DXF itself uses for a 2D point.
fn parse_sysvar_point(value: &str, min: usize, max: usize) -> Option<[f64; 3]> {
    let mut components: Vec<&str> = Vec::new();
    for token in value.split_whitespace() {
        for component in token.split(',') {
            if component.is_empty() {
                // An empty component (`1,,2`) would silently shift the axes.
                return None;
            }
            components.push(component);
        }
    }
    if components.len() < min || components.len() > max {
        return None;
    }
    let mut point = [0.0; 3];
    for (index, component) in components.iter().enumerate() {
        let parsed = super::commands::parse_signed_decimal(component)?;
        if !parsed.is_finite() {
            return None;
        }
        point[index] = parsed;
    }
    Some(point)
}
