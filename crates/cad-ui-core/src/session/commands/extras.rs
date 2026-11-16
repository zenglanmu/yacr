//! Typed-argument grammars of the 2026-11-07 extras batch.
//!
//! These are **argument forms of canonical selectors**, never new IPC:
//! [`PAN_SELECTOR`] (`-PAN`), [`TRACE_SELECTOR`], [`MOCORO_SELECTOR`],
//! [`MSTRETCH_SELECTOR`], [`VIEWRES_SELECTOR`], [`VSCURRENT_SELECTOR`],
//! [`WIPEOUTFRAME_SELECTOR`], [`CVPORT_SELECTOR`], [`ELEV_SELECTOR`],
//! [`FSMODE_SELECTOR`], [`COLOR_SELECTOR`] (`-COLOR` is the same token) and
//! [`LINETYPE_SELECTOR`] are admitted by [`super::ARGUMENT_SELECTORS`], parsed
//! here, and dispatched by the session as the matching [`cad_app::CommandId`]
//! with a payload.
//!
//! `SKETCH` has **no** command-line grammar in this build: its complete input (a
//! sampled point sequence) is carried by a host/IPC payload, not by one selector
//! argument. `MSTRETCH`'s typed form (base, delta, then one or more logical
//! windows) is [`parse_mstretch_argument`].
//!
//! Grammar:
//!
//! * `-PAN <dx> <dy>` or `-PAN <x1> <y1> <x2> <y2>` → one `Points([delta])`
//!   payload for the existing `PAN` camera path;
//! * `TRACE <x1> <y1> <x2> <y2> <width>` (five finite numbers, width > 0);
//! * `MOCORO MOVE|COPY <dx> <dy>` / `MOCORO ROTATE <bx> <by> <degrees>` /
//!   `MOCORO SCALE <bx> <by> <factor>`;
//! * `MSTRETCH <base_x> <base_y> <delta_x> <delta_y> <x1> <y1> <x2> <y2> […]`;
//! * `VIEWRES <integer 1..=1000>`;
//! * `VSCURRENT <WIREFRAME|2DWIREFRAME|2D|SHADED|SHADEDWITHEDGES>` (the command
//!   layer names the unsupported styles);
//! * `WIPEOUTFRAME <0|1|2>` (the system-variable spelling of `WIPEOUT Frames`;
//!   the bare selector reports the current value);
//! * `CVPORT <n>` (the session current-viewport marker of the active paper
//!   layout, one-based; the bare selector reports it);
//! * `ELEV <height>` (the session elevation new objects are created at; the bare
//!   selector reports the token);
//! * `FSMODE ON|OFF` (the session `FS` chain mode; the bare selector reports it);
//! * `COLOR <aci 0..=256>` / `COLOR BYLAYER|BYBLOCK` (the session current-colour
//!   token every created entity inherits; the bare selector reports it);
//! * `LINETYPE LIST` / `LINETYPE SET <name>` (the name takes the whole
//!   remainder, spaces allowed) / `LINETYPE CREATE <name> <dash>,<gap>,…
//!   [DESCRIPTION <text>]` / `LINETYPE DELETE <name>`. `CREATE`/`DELETE` are the
//!   table writes; `-LINETYPE` is the alias of the same selector and therefore
//!   the same grammar and the same handler.

use cad_app::{CommandPayload, LinetypeCommand, MocoroCommand};
use cad_domain::Point3;

use super::arguments::parse_signed_decimal;

/// `PAN` / `-PAN`: the typed world-delta form of the existing camera path.
pub const PAN_SELECTOR: &str = "PAN";
/// `TRACE <x1> <y1> <x2> <y2> <width>`.
pub const TRACE_SELECTOR: &str = "TRACE";
/// `MOCORO MOVE|COPY|ROTATE|SCALE`.
pub const MOCORO_SELECTOR: &str = "MOCORO";
/// `VIEWRES <1..=1000>`.
pub const VIEWRES_SELECTOR: &str = "VIEWRES";
/// `VSCURRENT <WIREFRAME|2DWIREFRAME|2D|SHADED|SHADEDWITHEDGES>`.
pub const VSCURRENT_SELECTOR: &str = "VSCURRENT";
/// `WIPEOUTFRAME [0|1|2]` — the system-variable spelling of `WIPEOUT Frames`.
pub const WIPEOUTFRAME_SELECTOR: &str = "WIPEOUTFRAME";
/// `CVPORT [<n>]` — the session current-viewport marker of the active layout.
pub const CVPORT_SELECTOR: &str = "CVPORT";
/// `ELEV [<height>]` — the session elevation new objects are created at.
///
/// A bare `ELEV` is the read-only report (the name-only funnel); only
/// [`parse_elev_argument`]'s value form is listed in
/// [`super::ARGUMENT_SELECTORS`].
pub const ELEV_SELECTOR: &str = "ELEV";
/// `FSMODE ON|OFF` — the session `FS` chain mode (`FS` reads it).
pub const FSMODE_SELECTOR: &str = "FSMODE";
/// `MSTRETCH <base_x> <base_y> <delta_x> <delta_y> <x1> <y1> <x2> <y2> …`.
pub const MSTRETCH_SELECTOR: &str = "MSTRETCH";
/// `COLOR <aci>|BYLAYER|BYBLOCK` — the session current-colour token.
pub const COLOR_SELECTOR: &str = "COLOR";
/// `-COLOR <aci>|BYLAYER|BYBLOCK` — the prompt spelling of the same token.
pub const DASH_COLOR_SELECTOR: &str = "-COLOR";
/// `LINETYPE LIST|SET|CREATE|DELETE` (the `-LINETYPE` alias rides it too).
pub const LINETYPE_SELECTOR: &str = "LINETYPE";

/// `-PAN <dx> <dy>` or `-PAN <x1> <y1> <x2> <y2>`.
pub fn parse_pan_argument(argument: &str) -> Option<CommandPayload> {
    let tokens = tokens(argument);
    let number = |text: &str| parse_signed_decimal(text);
    let delta = match tokens.as_slice() {
        [dx, dy] => Point3 {
            x: number(dx)?,
            y: number(dy)?,
            z: 0.0,
        },
        [x1, y1, x2, y2] => Point3 {
            x: number(x2)? - number(x1)?,
            y: number(y2)? - number(y1)?,
            z: 0.0,
        },
        _ => return None,
    };
    Some(CommandPayload::Points(vec![delta]))
}

/// `TRACE <x1> <y1> <x2> <y2> <width>`.
pub fn parse_trace_argument(argument: &str) -> Option<CommandPayload> {
    let tokens = tokens(argument);
    let [x1, y1, x2, y2, width] = tokens.as_slice() else {
        return None;
    };
    let start = Point3 {
        x: parse_signed_decimal(x1)?,
        y: parse_signed_decimal(y1)?,
        z: 0.0,
    };
    let end = Point3 {
        x: parse_signed_decimal(x2)?,
        y: parse_signed_decimal(y2)?,
        z: 0.0,
    };
    let width = parse_signed_decimal(width)?;
    Some(CommandPayload::Trace { start, end, width })
}

/// `MOCORO MOVE|COPY <dx> <dy>` / `ROTATE <bx> <by> <degrees>` /
/// `SCALE <bx> <by> <factor>`.
pub fn parse_mocoro_argument(argument: &str) -> Result<CommandPayload, String> {
    let tokens = tokens(argument);
    let Some((word, rest)) = tokens.split_first() else {
        return Err(invalid(
            "MOCORO",
            "MOCORO MOVE|COPY <dx> <dy>, ROTATE <bx> <by> <degrees> or SCALE <bx> <by> <factor>",
        ));
    };
    let operation = match word.to_ascii_uppercase().as_str() {
        "MOVE" | "COPY" if rest.len() == 2 => {
            let delta = Point3 {
                x: decimal(rest[0], "MOCORO")?,
                y: decimal(rest[1], "MOCORO")?,
                z: 0.0,
            };
            if word.eq_ignore_ascii_case("MOVE") {
                MocoroCommand::Move { delta }
            } else {
                MocoroCommand::Copy { delta }
            }
        }
        "ROTATE" if rest.len() == 3 => MocoroCommand::Rotate {
            base: point(rest[0], rest[1], "MOCORO")?,
            degrees: decimal(rest[2], "MOCORO")?,
        },
        "SCALE" if rest.len() == 3 => {
            let factor = decimal(rest[2], "MOCORO")?;
            if factor <= 0.0 {
                return Err("mocoro.invalid_factor: MOCORO SCALE needs a finite factor > 0".into());
            }
            MocoroCommand::Scale {
                base: point(rest[0], rest[1], "MOCORO")?,
                factor,
            }
        }
        _ => {
            return Err(invalid(
                "MOCORO",
                "MOCORO MOVE|COPY <dx> <dy>, ROTATE <bx> <by> <degrees> or SCALE <bx> <by> \
                 <factor>",
            ))
        }
    };
    Ok(CommandPayload::Mocoro(operation))
}

/// `VIEWRES <integer percentage 1..=1000>`.
pub fn parse_viewres_argument(argument: &str) -> Result<CommandPayload, String> {
    let token = argument.trim();
    if token.is_empty() || token.split_whitespace().count() != 1 {
        return Err(invalid(
            "VIEWRES",
            "VIEWRES <integer percentage in 1..=1000>",
        ));
    }
    let percent = token.parse::<u32>().map_err(|_| {
        invalid(
            "VIEWRES",
            "VIEWRES needs one integer percentage in 1..=1000",
        )
    })?;
    if !(1..=1000).contains(&percent) {
        return Err(format!(
            "viewres.out_of_range: VIEWRES needs an integer percentage in 1..=1000, got {percent}; \
             nothing was changed"
        ));
    }
    Ok(CommandPayload::ViewRes(percent))
}

/// `VSCURRENT <style>`: the parser admits one word; the command maps the
/// accepted spellings and refuses every other style by name.
///
/// Accepted: `WIREFRAME`/`2DWIREFRAME`/`2D` (bodies as edges only) and
/// `SHADED`/`SHADEDWITHEDGES` (filled bodies plus stored edges, the session
/// default). The style token is session-only and changes 3D body display only.
pub fn parse_vscurrent_argument(argument: &str) -> Result<CommandPayload, String> {
    let style = argument.trim();
    if style.is_empty() || style.split_whitespace().count() != 1 {
        return Err(invalid(
            "VSCURRENT",
            "VSCURRENT <WIREFRAME|2DWIREFRAME|2D|SHADED|SHADEDWITHEDGES>",
        ));
    }
    Ok(CommandPayload::VisualStyle(style.to_string()))
}

/// `WIPEOUTFRAME <0|1|2>`: the session frame state for every wipeout.
///
/// The grammar is the three documented values only. A non-integer and an
/// out-of-range integer are refused **by name** (`wipeoutframe.out_of_range`,
/// `invalid WIPEOUTFRAME argument`), never clamped to the nearest valid state:
/// a silent clamp would make `WIPEOUTFRAME 3` look like a successful hide. The
/// stored token is the very same [`cad_app::WipeoutFrames`] the `WIPEOUT Frames`
/// option and the drawing-settings panel write.
pub fn parse_wipeoutframe_argument(argument: &str) -> Result<CommandPayload, String> {
    let token = argument.trim();
    if token.is_empty() || token.split_whitespace().count() != 1 {
        return Err(invalid(
            "WIPEOUTFRAME",
            "WIPEOUTFRAME <0|1|2> (0 = off, 1 = on, 2 = display but do not plot)",
        ));
    }
    let Ok(value) = token.parse::<i64>() else {
        return Err(invalid(
            "WIPEOUTFRAME",
            "WIPEOUTFRAME needs one integer: 0, 1 or 2",
        ));
    };
    let frames = cad_app::WipeoutFrames::from_number(value).ok_or_else(|| {
        format!(
            "wipeoutframe.out_of_range: WIPEOUTFRAME accepts 0 (off), 1 (on) or 2 (display but \
             do not plot); got {value} and nothing was changed"
        )
    })?;
    Ok(CommandPayload::WipeoutFrame(frames))
}

/// `CVPORT <n>`: the session current-viewport marker of the active paper layout.
///
/// The grammar is one **positive** integer: a viewport *position* on the command
/// line, one-based like AutoCAD (`1` is the layout's first viewport). `0`,
/// non-integers and multi-token input are refused by name before dispatch; the
/// upper bound is the active layout's viewport count, which only the command
/// layer knows, so it is validated there (`cvport.out_of_range`, never clamped).
pub fn parse_cvport_argument(argument: &str) -> Result<CommandPayload, String> {
    let token = argument.trim();
    if token.is_empty() || token.split_whitespace().count() != 1 {
        return Err(invalid(
            "CVPORT",
            "CVPORT <viewport position> (one-based; the bare selector reports it)",
        ));
    }
    let Ok(position) = token.parse::<u32>() else {
        return Err(invalid(
            "CVPORT",
            "CVPORT needs one positive integer viewport position",
        ));
    };
    if position == 0 {
        return Err(
            "cvport.out_of_range: CVPORT positions are one-based, so 0 is not a viewport; \
             nothing was changed"
                .into(),
        );
    }
    Ok(CommandPayload::Cvport(position))
}

/// `256`: the ACI the handler and the create funnel read as `ByLayer`.
const BY_LAYER_ACI: i16 = 256;

/// `0`: the ACI the handler and the create funnel read as `ByBlock`.
const BY_BLOCK_ACI: i16 = 0;

/// The handler's own reason code for an ACI outside the closed domain.
fn color_out_of_domain_reason(token: &str) -> String {
    format!(
        "color.invalid_value: COLOR needs an ACI in 0..=256 (0 = ByBlock, 1..=255 = ACI, 256 = \
         ByLayer), got {token}; nothing was changed"
    )
}

/// `COLOR <aci>|BYLAYER|BYBLOCK`: the session current-colour token.
///
/// The bare selector is the `color.report` and never reaches this parser, so only
/// a real value is admitted. Two shapes are accepted:
///
/// * one ACI written as an integer in `0..=256` (`0` = ByBlock, `1..=255` = the
///   palette entry, `256` = ByLayer);
/// * one of the prompt's symbolic words, case-insensitive: `BYLAYER` (and its
///   `BYNAME` spelling) → `256`, `BYBLOCK` → `0`.
///
/// Nothing is clamped or defaulted: a number outside the domain is refused by
/// name with the handler's own `color.invalid_value` code, so `COLOR 257` can
/// never read as "some colour", and a word that is not one of the two symbols
/// (a palette name such as `RED`) is refused with `color.invalid_argument`
/// because this build has no name→ACI table and must not invent one. The
/// command layer's `set_current_color` keeps the same domain for direct payload
/// callers, so both layers answer the same fact the same way.
pub fn parse_color_argument(argument: &str) -> Result<CommandPayload, String> {
    let token = argument.trim();
    if token.eq_ignore_ascii_case("BYLAYER") || token.eq_ignore_ascii_case("BYNAME") {
        return Ok(CommandPayload::CurrentColor(BY_LAYER_ACI));
    }
    if token.eq_ignore_ascii_case("BYBLOCK") {
        return Ok(CommandPayload::CurrentColor(BY_BLOCK_ACI));
    }
    // A whole number is the only other admitted shape: one token, digits only
    // after an optional sign (so `1.0`, `1e3`, `0x1` and `1 2` are not numbers
    // here). The domain check is the same one the handler applies.
    let digits = token.strip_prefix(['-', '+']).unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "color.invalid_argument: unsupported CAD command: COLOR {token} (COLOR needs one ACI \
             in 0..=256, BYLAYER or BYBLOCK; a bare COLOR reports the current colour and a colour \
             name has no table in this build)"
        ));
    }
    let value: i64 = token
        .parse()
        .map_err(|_| color_out_of_domain_reason(token))?;
    if !(0..=256).contains(&value) {
        return Err(color_out_of_domain_reason(token));
    }
    Ok(CommandPayload::CurrentColor(value as i16))
}

/// `LINETYPE LIST|SET|CREATE|DELETE` (the `-LINETYPE` alias shares this grammar).
///
/// * `LIST` — the table report;
/// * `SET <name>` — the name takes the whole remainder (spaces allowed);
/// * `CREATE <name> <dash>,<gap>,… [DESCRIPTION <text>]` — the name is one
///   (optionally quoted) token, the pattern is one comma-separated token of
///   decimal lengths (positive dash, negative gap, DXF group 49) and a trailing
///   `DESCRIPTION` takes the rest of the line. The pattern's *semantic* rules
///   (non-empty, non-zero lengths, at least one dash) live in the database
///   (`linetype.bad_pattern`), so this parser only proves the shape;
/// * `DELETE <name>` — the name takes the whole remainder (spaces allowed).
///
/// Anything else is a grammar refusal: nothing is defaulted and no keyword is
/// silently ignored.
pub fn parse_linetype_argument(argument: &str) -> Result<CommandPayload, String> {
    let expected = "LINETYPE LIST, LINETYPE SET <name>, LINETYPE CREATE <name> <dash>,<gap>,… \
                    [DESCRIPTION <text>] or LINETYPE DELETE <name>";
    let trimmed = argument.trim();
    let (word, rest) = match trimmed.find(char::is_whitespace) {
        Some(index) => (
            &trimmed[..index],
            trimmed[index..].trim_start_matches(char::is_whitespace),
        ),
        None => (trimmed, ""),
    };
    if word.eq_ignore_ascii_case("LIST") && rest.is_empty() {
        return Ok(CommandPayload::Linetype(LinetypeCommand::List));
    }
    if word.eq_ignore_ascii_case("SET") && !rest.is_empty() {
        return Ok(CommandPayload::Linetype(LinetypeCommand::Set(
            rest.to_string(),
        )));
    }
    if word.eq_ignore_ascii_case("DELETE") && !rest.is_empty() {
        return Ok(CommandPayload::Linetype(LinetypeCommand::Delete {
            name: rest.to_string(),
        }));
    }
    if word.eq_ignore_ascii_case("CREATE") {
        return parse_linetype_create(rest).ok_or_else(|| invalid("LINETYPE", expected));
    }
    Err(invalid("LINETYPE", expected))
}

/// `CREATE <name> <pattern> [DESCRIPTION <text>]`.
///
/// `None` is a shape refusal (missing name/pattern, an empty comma entry, a
/// non-numeric length, a trailing token that is not `DESCRIPTION`).
fn parse_linetype_create(rest: &str) -> Option<CommandPayload> {
    let tokens = crate::session::commands::arguments::split_quoted_tokens(rest)?;
    let (name, remainder) = tokens.split_first()?;
    let (pattern_token, remainder) = remainder.split_first()?;
    if name.trim().is_empty() {
        return None;
    }
    let mut pattern = Vec::new();
    for entry in pattern_token.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            return None;
        }
        pattern.push(parse_signed_decimal(entry)?);
    }
    let description = match remainder {
        [] => None,
        [keyword, words @ ..] if keyword.eq_ignore_ascii_case("DESCRIPTION") => {
            if words.is_empty() {
                return None;
            }
            Some(words.join(" "))
        }
        _ => return None,
    };
    Some(CommandPayload::Linetype(LinetypeCommand::Create {
        name: name.trim().to_string(),
        description,
        pattern,
    }))
}

/// `ELEV <height>`: the session elevation the create funnel offsets new objects
/// by.
///
/// One finite decimal, in drawing units; a negative height is legal (objects
/// below the XY plane). The bare selector is the report and never reaches this
/// parser. A non-number, a multi-token line or a non-finite value is the stable
/// `elev.argument` refusal — nothing is clamped and no default is invented.
pub fn parse_elev_argument(argument: &str) -> Result<CommandPayload, String> {
    let tokens = tokens(argument);
    let [height] = tokens.as_slice() else {
        return Err(
            "elev.argument: ELEV needs one finite height in drawing units (`ELEV <height>`); \
             nothing was changed"
                .into(),
        );
    };
    let height = parse_signed_decimal(height).ok_or_else(|| {
        "elev.argument: ELEV needs one finite decimal height in drawing units; nothing was changed"
            .to_string()
    })?;
    Ok(CommandPayload::Elevation(height))
}

/// `FSMODE ON|OFF`: the session `FS` chain mode.
///
/// `ON`/`1` follows the transitive tangent chain, `OFF`/`0` (AutoCAD's initial
/// value) adds only the directly touching objects. The two symbolic words and the
/// two numeric spellings are the whole grammar: anything else is
/// `fsmode.invalid_value`, never a toggle, and the bare selector is the report.
pub fn parse_fsmode_argument(argument: &str) -> Result<CommandPayload, String> {
    let tokens = tokens(argument);
    let [value] = tokens.as_slice() else {
        return Err(
            "fsmode.invalid_value: FSMODE needs ON or OFF (1 or 0 also parse); nothing was changed"
                .into(),
        );
    };
    match crate::session::commands::parse_sysvar_toggle(value) {
        Some(enabled) => Ok(CommandPayload::FsMode(enabled)),
        None => Err(format!(
            "fsmode.invalid_value: FSMODE needs ON or OFF (1 or 0 also parse), got '{value}'; \
             nothing was changed"
        )),
    }
}

/// `MSTRETCH <base_x> <base_y> <delta_x> <delta_y> <x1> <y1> <x2> <y2> […]`:
/// the STRETCH planner over one or more logical crossing windows.
///
/// The typed line mirrors [`CommandPayload::MStretch`] field for field: the four
/// leading decimals are the world `base` point and the world `delta` point (the
/// handler moves by `delta - base`), and every following group of four decimals is
/// one crossing window in **canvas logical pixels**, the same convention as
/// `STRETCH`'s `rect_logical` and `ZOOM WINDOW`. The windows are a flat list, so:
///
/// * fewer than two window groups (eight decimals) is `mstretch.argument`;
/// * a token count that cannot be split into whole four-decimal windows is
///   `mstretch.argument_odd` (never a truncated window);
/// * a token that is not a finite decimal is `mstretch.invalid_number`.
///
/// The **semantic** checks stay in the handler, before its transaction:
/// `mstretch.degenerate_window` (zero width/height after the viewport mapping),
/// `mstretch.zero_displacement` (`base == delta`) and the plan-view/model-space
/// requirements. Nothing here defaults a missing window or a missing delta.
pub fn parse_mstretch_argument(argument: &str) -> Result<CommandPayload, String> {
    let tokens = tokens(argument);
    let (displacement, windows) = tokens.split_at(tokens.len().min(4));
    if displacement.len() < 4 {
        return Err(
            "mstretch.argument: MSTRETCH needs <base_x> <base_y> <delta_x> <delta_y> and at least \
             one <x1> <y1> <x2> <y2> window; nothing was changed"
                .into(),
        );
    }
    if windows.len() % 4 != 0 {
        return Err(format!(
            "mstretch.argument_odd: MSTRETCH needs every crossing window as exactly four decimals \
             (x1 y1 x2 y2); {} window token(s) cannot be split into whole windows and nothing was \
             changed",
            windows.len()
        ));
    }
    if windows.is_empty() {
        return Err(
            "mstretch.argument: MSTRETCH needs at least one crossing window (x1 y1 x2 y2) after \
             the base and delta points; nothing was changed"
                .into(),
        );
    }
    let number = |text: &str| {
        parse_signed_decimal(text).ok_or_else(|| {
            format!(
                "mstretch.invalid_number: MSTRETCH needs finite decimals, got '{text}'; nothing \
                 was changed"
            )
        })
    };
    let base = Point3 {
        x: number(displacement[0])?,
        y: number(displacement[1])?,
        z: 0.0,
    };
    let delta = Point3 {
        x: number(displacement[2])?,
        y: number(displacement[3])?,
        z: 0.0,
    };
    let mut rects_logical = Vec::with_capacity(windows.len() / 4);
    for window in windows.as_chunks::<4>().0 {
        rects_logical.push([
            [number(window[0])?, number(window[1])?],
            [number(window[2])?, number(window[3])?],
        ]);
    }
    Ok(CommandPayload::MStretch {
        rects_logical,
        base,
        delta,
    })
}

/// Split one argument into non-empty whitespace-delimited tokens.
fn tokens(argument: &str) -> Vec<&str> {
    argument.split_whitespace().collect()
}

/// Parse one finite decimal or produce a stable selector refusal.
fn decimal(text: &str, selector: &str) -> Result<f64, String> {
    parse_signed_decimal(text).ok_or_else(|| {
        invalid(
            selector,
            "every numeric component must be one finite decimal number",
        )
    })
}

/// Parse one finite `x y` point or produce a stable selector refusal.
fn point(x: &str, y: &str, selector: &str) -> Result<Point3, String> {
    Ok(Point3 {
        x: decimal(x, selector)?,
        y: decimal(y, selector)?,
        z: 0.0,
    })
}

/// The stable refusal sentence for an argument outside a selector's grammar.
fn invalid(selector: &str, expected: &str) -> String {
    format!("unsupported CAD command: {selector} argument (expected {expected})")
}
