//! Typed argument grammars of the 2026-11-07 layout / viewport batch.
//!
//! These are **argument forms of canonical selectors**, never a second IPC:
//! [`LAYOUT_SELECTOR`] … [`LAYVPI_SELECTOR`] are admitted by
//! [`super::ARGUMENT_SELECTORS`], parsed here, and dispatched by the session as
//! the matching [`cad_app::CommandId`] with a [`cad_app::LayoutPayload`].
//!
//! The grammar is deliberately narrow and documented:
//!
//! * `LAYOUT LIST|NEW <name>|RENAME <old> <new>|DELETE <name>|SET <name>`;
//! * `LAYOUTMERGE <source> <target>`;
//! * `LAYOUTWIZARD <name> [<paper> <width> <height>]`;
//! * `VPORTS <rows> <cols>` (`-VPORTS` normalizes here);
//! * `VPCLIP RECT <minx> <miny> <maxx> <maxy>`;
//! * `VPSCALE <scale-name-or-ratio>`;
//! * `LAYVPI <layer>[,<layer>...]`.
//!
//! A single-name form takes the whole remainder (spaces allowed). The two-name
//! forms split at the first whitespace, so their **left-hand** name cannot
//! contain a space; that is a grammar boundary, not a guessed name.
//! `LAYOUTWIZARD`'s name is the first token because the optional paper triple
//! follows it.

use cad_app::{LayoutPayload, PaperSetup};
use cad_domain::Point3;

/// `LAYOUT`: list/modify the layout table and switch the active space.
pub const LAYOUT_SELECTOR: &str = "LAYOUT";
/// `LAYOUTMERGE`: merge one layout's entities into another.
pub const LAYOUTMERGE_SELECTOR: &str = "LAYOUTMERGE";
/// `LAYOUTWIZARD`: create a layout with an optional page setup.
pub const LAYOUTWIZARD_SELECTOR: &str = "LAYOUTWIZARD";
/// `VPORTS`: create a rectangular viewport grid.
pub const VPORTS_SELECTOR: &str = "VPORTS";
/// `VPCLIP`: set a viewport clip boundary (RECT only in this build).
pub const VPCLIP_SELECTOR: &str = "VPCLIP";
/// `VPSCALE`: set the selected viewport's scale.
pub const VPSCALE_SELECTOR: &str = "VPSCALE";
/// `LAYVPI`: freeze layers in every other layout viewport.
pub const LAYVPI_SELECTOR: &str = "LAYVPI";

/// Parse one `LAYOUT <option>` argument.
pub fn parse_layout_argument(argument: &str) -> Result<LayoutPayload, String> {
    let Some((word, rest)) = split_word(argument) else {
        return Err(invalid(
            "layout",
            "LAYOUT needs LIST, NEW, RENAME, DELETE or SET",
        ));
    };
    match word.to_ascii_uppercase().as_str() {
        "LIST" if rest.is_empty() => Ok(LayoutPayload::List),
        "NEW" if !rest.is_empty() => Ok(LayoutPayload::New {
            name: rest.to_string(),
        }),
        "DELETE" if !rest.is_empty() => Ok(LayoutPayload::Delete {
            name: rest.to_string(),
        }),
        "SET" if !rest.is_empty() => Ok(LayoutPayload::Set {
            name: rest.to_string(),
        }),
        "RENAME" => {
            let Some((old, new)) = split_word(rest) else {
                return Err(invalid("layout", "LAYOUT RENAME needs <old> <new>"));
            };
            if old.is_empty() || new.is_empty() {
                return Err(invalid("layout", "LAYOUT RENAME needs <old> <new>"));
            }
            Ok(LayoutPayload::Rename {
                old: old.to_string(),
                new: new.to_string(),
            })
        }
        _ => Err(invalid(
            "layout",
            "LAYOUT needs LIST, NEW <name>, RENAME <old> <new>, DELETE <name> or SET <name>",
        )),
    }
}

/// Parse one `LAYOUTMERGE <source> <target>` argument.
pub fn parse_layoutmerge_argument(argument: &str) -> Result<LayoutPayload, String> {
    let Some((source, target)) = split_word(argument) else {
        return Err(invalid(
            "layoutmerge",
            "LAYOUTMERGE needs <source> <target>",
        ));
    };
    if source.is_empty() || target.is_empty() {
        return Err(invalid(
            "layoutmerge",
            "LAYOUTMERGE needs <source> <target>",
        ));
    }
    Ok(LayoutPayload::Merge {
        source: source.to_string(),
        target: target.to_string(),
    })
}

/// Parse one `LAYOUTWIZARD <name> [<paper> <width> <height>]` argument.
pub fn parse_layoutwizard_argument(argument: &str) -> Result<LayoutPayload, String> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let name = match tokens.as_slice() {
        [] => {
            return Err(invalid(
                "layoutwizard",
                "LAYOUTWIZARD needs <name> [<paper> <width> <height>]",
            ))
        }
        [name] => (*name).to_string(),
        [name, paper, width, height] => {
            let width = width.parse::<f64>().map_err(|_| {
                invalid(
                    "layoutwizard",
                    "LAYOUTWIZARD paper width and height must be finite numbers > 0",
                )
            })?;
            let height = height.parse::<f64>().map_err(|_| {
                invalid(
                    "layoutwizard",
                    "LAYOUTWIZARD paper width and height must be finite numbers > 0",
                )
            })?;
            if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
                return Err(invalid(
                    "layoutwizard",
                    "LAYOUTWIZARD paper width and height must be finite numbers > 0",
                ));
            }
            return Ok(LayoutPayload::Wizard {
                name: (*name).to_string(),
                paper: Some(PaperSetup {
                    name: (*paper).to_string(),
                    width,
                    height,
                }),
            });
        }
        _ => {
            return Err(invalid(
                "layoutwizard",
                "LAYOUTWIZARD needs <name> [<paper> <width> <height>]",
            ))
        }
    };
    Ok(LayoutPayload::Wizard { name, paper: None })
}

/// Parse one `VPORTS <rows> <cols>` argument.
pub fn parse_vports_argument(argument: &str) -> Result<LayoutPayload, String> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    let [rows, cols] = tokens.as_slice() else {
        return Err(invalid("vports", "VPORTS needs <rows> <cols> >= 1"));
    };
    let rows = rows
        .parse::<u32>()
        .ok()
        .filter(|value| *value >= 1)
        .ok_or_else(|| invalid("vports", "VPORTS needs <rows> <cols> >= 1"))?;
    let cols = cols
        .parse::<u32>()
        .ok()
        .filter(|value| *value >= 1)
        .ok_or_else(|| invalid("vports", "VPORTS needs <rows> <cols> >= 1"))?;
    Ok(LayoutPayload::Viewports { rows, cols })
}

/// Parse one `VPCLIP RECT <minx> <miny> <maxx> <maxy>` argument.
pub fn parse_vpclip_argument(argument: &str) -> Result<LayoutPayload, String> {
    let Some((kind, rest)) = split_word(argument) else {
        return Err(invalid(
            "vpclip",
            "VPCLIP needs RECT <minx> <miny> <maxx> <maxy>",
        ));
    };
    if !kind.eq_ignore_ascii_case("RECT") {
        return Err(format!(
            "vpclip.polygonal_not_implemented: VPCLIP only supports RECT <minx> <miny> <maxx> \
             <maxy>; polygonal and object clip boundaries are NOT IMPLEMENTED (got {:?})",
            kind
        ));
    }
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let [minx, miny, maxx, maxy] = tokens.as_slice() else {
        return Err(invalid(
            "vpclip",
            "VPCLIP RECT needs <minx> <miny> <maxx> <maxy>",
        ));
    };
    let value = |text: &str| {
        text.parse::<f64>().map_err(|_| {
            invalid(
                "vpclip",
                "VPCLIP RECT needs four finite numbers <minx> <miny> <maxx> <maxy>",
            )
        })
    };
    let min = Point3 {
        x: value(minx)?,
        y: value(miny)?,
        z: 0.0,
    };
    let max = Point3 {
        x: value(maxx)?,
        y: value(maxy)?,
        z: 0.0,
    };
    if !min.x.is_finite()
        || !min.y.is_finite()
        || !max.x.is_finite()
        || !max.y.is_finite()
        || min.x >= max.x
        || min.y >= max.y
    {
        return Err(
            "vpclip.degenerate_rect: VPCLIP RECT needs finite minx < maxx and miny < maxy".into(),
        );
    }
    Ok(LayoutPayload::ClipRect { min, max })
}

/// Parse one `VPSCALE <scale>` argument.
///
/// A ratio (`a:b`) is parsed and its components validated here; a bare word is
/// passed through for the drawing's scale table (the application resolves it).
/// Zero, negative and non-finite numbers are refused before dispatch.
pub fn parse_vpscale_argument(argument: &str) -> Result<LayoutPayload, String> {
    let name = argument.trim();
    if name.is_empty() || name.split_whitespace().count() != 1 {
        return Err(invalid(
            "vpscale",
            "VPSCALE needs one scale name or ratio, e.g. `VPSCALE 1:100`",
        ));
    }
    let number = |text: &str| {
        text.parse::<f64>().map_err(|_| {
            invalid(
                "vpscale",
                "VPSCALE ratio components must be finite numbers, e.g. `1:100`",
            )
        })
    };
    if let Some((paper, drawing)) = name.split_once(':') {
        let paper = number(paper)?;
        let drawing = number(drawing)?;
        if !paper.is_finite() || !drawing.is_finite() {
            return Err("vpscale.non_finite_scale: VPSCALE needs finite ratio components".into());
        }
        if paper == 0.0 || drawing == 0.0 {
            return Err("vpscale.zero_scale: VPSCALE ratio components must be non-zero".into());
        }
        if paper < 0.0 || drawing < 0.0 {
            return Err("vpscale.invalid_scale: VPSCALE ratio components must be positive".into());
        }
    } else if let Ok(value) = name.parse::<f64>() {
        if !value.is_finite() {
            return Err("vpscale.non_finite_scale: VPSCALE needs a finite scale".into());
        }
        if value == 0.0 {
            return Err("vpscale.zero_scale: VPSCALE needs a non-zero scale".into());
        }
        if value < 0.0 {
            return Err("vpscale.invalid_scale: VPSCALE needs a positive scale".into());
        }
    }
    Ok(LayoutPayload::Scale {
        name: name.to_string(),
    })
}

/// Parse one `LAYVPI <layer>[,<layer>...]` argument.
pub fn parse_layvpi_argument(argument: &str) -> Result<LayoutPayload, String> {
    let mut layers = Vec::new();
    for token in argument.split(',') {
        let name = token.trim();
        if name.is_empty() {
            return Err(invalid(
                "layvpi",
                "LAYVPI needs one or more comma-separated layer names",
            ));
        }
        layers.push(name.to_string());
    }
    if layers.is_empty() {
        return Err(invalid(
            "layvpi",
            "LAYVPI needs one or more comma-separated layer names",
        ));
    }
    Ok(LayoutPayload::FreezeViewportLayers { layers })
}

/// Split the first whitespace-delimited word from the trimmed remainder.
fn split_word(argument: &str) -> Option<(&str, &str)> {
    let trimmed = argument.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.find(char::is_whitespace) {
        Some(index) => Some((
            &trimmed[..index],
            trimmed[index..].trim_start_matches(char::is_whitespace),
        )),
        None => Some((trimmed, "")),
    }
}

/// The stable refusal sentence for an argument outside a selector's grammar.
fn invalid(selector: &str, expected: &str) -> String {
    format!("unsupported CAD command: {selector} argument (expected {expected})")
}
