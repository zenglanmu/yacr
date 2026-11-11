//! Command-name resolution, selector constants and the admitted vocabulary.
//!
//! Split out of `commands.rs` for the code-size gate (the authority tables stay
//! in `commands.rs`, which the documentation gate parses line by line). The
//! public paths are unchanged: `commands.rs` re-exports this module.

use cad_app::{CommandId, MeasurementToolKind};

use super::*;

/// Split a typed line into an argument-taking selector and its argument.
///
/// `Some((selector, argument))` only when `text` starts with one of
/// [`ARGUMENT_SELECTORS`] followed by one or more spaces/tabs and a non-empty
/// argument (`ZOOM 2`, `  snapunit\t0.5 `); a name-only line (`ZOOM`,
/// `SNAPUNIT`) returns `None` so it keeps the name-only funnel and its existing
/// payload refusal. The argument keeps its internal text: `ZOOM 2 3` reaches
/// [`parse_zoom_argument`] as `2 3` and is refused there rather than being split
/// a second time.
pub fn typed_command_argument(text: &str) -> Option<(&'static str, &str)> {
    let trimmed = text.trim();
    // A whole line that is itself a canonical selector is a name-only command,
    // never a `name + argument` split: `CHAMFER POLYLINE` / `FILLET POLYLINE`
    // stay their own selectors even though `CHAMFER`/`FILLET` now take options.
    if canonical_command_name(trimmed).is_some() {
        return None;
    }
    let split = trimmed.find([' ', '\t'])?;
    let (name, rest) = trimmed.split_at(split);
    let argument = rest.trim_start_matches([' ', '\t']).trim_end();
    if argument.is_empty() {
        return None;
    }
    let selector = canonical_command_name(name)?;
    ARGUMENT_SELECTORS
        .contains(&selector)
        .then_some((selector, argument))
}

/// Split a typed line into a shell appearance/navigation selector and its
/// non-empty argument (`NAVVCUBE ON`, `NAVVCUBEDISPLAY 3`,
/// `DISPLAYVIEWCUBEIN2D OFF`, …).
///
/// These five names are settings selectors rather than `CommandId` dispatches,
/// so they are deliberately **not** in [`ARGUMENT_SELECTORS`]: the session's
/// shell-settings intercept resolves the raw line before the drawing-argument
/// path ever sees it. The wire gate ([`crate::shell::check_command`]) still has
/// to admit the line before the queue is touched, and this is the one helper
/// that answers "is this an admitted shell argument form?" for it.
pub fn shell_setting_argument(text: &str) -> Option<(&'static str, &str)> {
    let trimmed = text.trim();
    let split = trimmed.find([' ', '\t'])?;
    let (name, rest) = trimmed.split_at(split);
    let argument = rest.trim_start_matches([' ', '\t']).trim_end();
    if argument.is_empty() {
        return None;
    }
    let selector = canonical_command_name(name)?;
    SHELL_SETTING_SELECTORS
        .contains(&selector)
        .then_some((selector, argument))
}

/// AutoCAD's `ZOOM n` / `ZOOM ALL`: the first argument-taking selector.
///
/// A positive factor reuses the `ZOOMIN`/`ZOOMOUT` step path (anchored at the
/// viewport centre); `ALL`/`EXTENTS` dispatches the ordinary `FIT`; `WINDOW`
/// starts the two-corner capture and `PREVIOUS` restores the last discrete view
/// (see [`ZoomArgument`]). The last two still carry **no** `CommandId` of their
/// own: they ride [`CommandId::Zoom`] with their own payloads.
pub const ARGUMENT_SELECTOR: &str = "ZOOM";

/// AutoCAD's `SNAPUNIT <value>`: the session grid-snap spacing sysvar.
///
/// The value is a positive decimal in drawing units and is stored as session
/// state only (no database, history or preference write). It is the spacing the
/// `SNAP` (F9) quantiser uses.
pub const SNAPUNIT_SELECTOR: &str = "SNAPUNIT";

/// AutoCAD's `PDMODE <n>`: the drawing-global POINT display **mode** sysvar.
///
/// The value is an integer stored **raw** in the drawing header (`$PDMODE`), so
/// it is a document write (one revision bump, no undo entry), not session state.
/// The display layer interprets the bits.
pub const PDMODE_SELECTOR: &str = "PDMODE";

/// AutoCAD's `PDSIZE <n>`: the drawing-global POINT display **size** sysvar.
///
/// The value is a finite decimal stored **raw** in the drawing header
/// (`$PDSIZE`): `> 0` absolute drawing units, `0` viewport-relative, `< 0` a
/// percentage of the viewport height. A document write like
/// [`PDMODE_SELECTOR`], not session state.
pub const PDSIZE_SELECTOR: &str = "PDSIZE";

/// The `PEDIT <option>` selector: one declared v1 polyline option (AutoCAD
/// 2026).
///
/// AutoCAD's `PEDIT` shows an option prompt after the polyline pick; this build
/// has no in-command option prompt, so the option is typed on the same command
/// line. The grammar ([`parse_pedit_argument`]) is `CLOSE`/`C`, `OPEN`/`O` and
/// `REVERSE`/`R`, case-insensitive. The current selection (exactly one
/// model-space LWPOLYLINE) is still the whole input, so there is no second IPC.
/// A bare `PEDIT` is admitted by name and refused by the application with
/// `pedit.needs_option`; an unknown argument is refused before IPC with
/// [`invalid_pedit_argument_reason`]. `Join`, `Width`/`Halfwidth`, `Fit`,
/// `Spline`, `Decurve`, `Ltype gen`, `Undo`, `Multiple`, `Edit vertex` and the
/// `PEDITACCEPT` conversion prompt are NOT IMPLEMENTED.
pub const PEDIT_SELECTOR: &str = "PEDIT";
/// The `HATCHEDIT <option>` selector: the typed v1 hatch-edit surface (AutoCAD
/// 2026).
///
/// AutoCAD's `HATCHEDIT` opens a dialog; this build has no in-command dialog, so
/// the option is typed on the command line. The grammar
/// ([`super::parse_hatchedit_argument`]) is `ANGLE <degrees>`, `SCALE <factor>`,
/// `SOLID`, `GRADIENT`, `ADDBOUNDARIES` and `REMOVEBOUNDARIES`,
/// case-insensitive. The current selection (exactly one model-space hatch) is
/// still the whole input, so there is no second IPC. A
/// bare `HATCHEDIT` is admitted by name and refused by the application with
/// `hatchedit.needs_option`; an unknown argument is refused before IPC with
/// [`super::invalid_hatchedit_argument_reason`]. `Pattern`, `Color`,
/// `Properties`, `Draw order` and associativity are NOT IMPLEMENTED.
/// `ADDBOUNDARIES`/`REMOVEBOUNDARIES` are real pre-selection boundary edits.
pub const HATCHEDIT_SELECTOR: &str = "HATCHEDIT";
/// AutoCAD's `MEASUREGEOM <option>`: the inquiry router's typed argument form.
///
/// This is an **argument form of the canonical `MEASUREGEOM` selector**, not a
/// new command: `DISTANCE`/`ANGLE`/`AREA`/`POLYLINE` start the existing
/// measurement tools, `RADIUS` dispatches the read-only report and `VOLUME` is
/// an explicit not-implemented refusal. Full contract:
/// [`cad_app::command_id::measuregeom`].
pub const MEASUREGEOM_SELECTOR: &str = "MEASUREGEOM";

/// The `NAVBAR` selector: the shell navigation-bar visibility switch.
///
/// A **settings write**, not a CAD command: no `CommandId` exists for it and
/// [`normalize_command`] therefore returns `None`. It is admitted by
/// [`canonical_command_name`] (and advertised by [`admissible_names`]) because
/// `CadSession::command` resolves it before the command table and applies the
/// root `navbarVisible` preference through the host's validated settings path.
/// `NAVBAR` toggles; `NAVBAR ON|OFF|1|0` sets the value.
pub const NAVBAR_SELECTOR: &str = "NAVBAR";

/// The `NAVVCUBE` selector: the current view class's ViewCube visibility.
///
/// `NAVVCUBE` toggles the bit of the **current** view class (bit 1 = 3D, bit 2 =
/// 2D) in the root `navvcubeDisplay` preference; `NAVVCUBE ON|OFF|1|0`
/// sets/clears it. `NAVVCUBE PROPERTIES` is refused explicitly because this build
/// has no ViewCube settings dialog.
pub const NAVVCUBE_SELECTOR: &str = "NAVVCUBE";

/// The `NAVVCUBEDISPLAY` selector: write the two-bit ViewCube class mask.
///
/// Takes `0..=3`; the bare form is an explicit `needs_value` refusal.
pub const NAVVCUBEDISPLAY_SELECTOR: &str = "NAVVCUBEDISPLAY";

/// The `DISPLAYVIEWCUBEIN2D` selector: the ViewCube's 2D-view visibility.
pub const DISPLAYVIEWCUBEIN2D_SELECTOR: &str = "DISPLAYVIEWCUBEIN2D";

/// The `DISPLAYVIEWCUBEIN3D` selector: the ViewCube's 3D-view visibility.
pub const DISPLAYVIEWCUBEIN3D_SELECTOR: &str = "DISPLAYVIEWCUBEIN3D";

/// The five shell settings selectors (`NAVBAR`, `NAVVCUBE`, `NAVVCUBEDISPLAY`,
/// `DISPLAYVIEWCUBEIN2D`, `DISPLAYVIEWCUBEIN3D`).
///
/// They are admitted names **without** a [`CommandId`]: the command table must
/// not grow a settings-write arm, and `EXPECTED_COMMAND_COUNT` must not change.
pub const SHELL_SETTING_SELECTORS: [&str; 5] = [
    NAVBAR_SELECTOR,
    NAVVCUBE_SELECTOR,
    NAVVCUBEDISPLAY_SELECTOR,
    DISPLAYVIEWCUBEIN2D_SELECTOR,
    DISPLAYVIEWCUBEIN3D_SELECTOR,
];

/// `UNDEFINE <name>`: disable one built-in command **for this session**.
///
/// A session-only vocabulary control (AutoCAD's `UNDEFINE`), not a drawing
/// command: there is no [`CommandId`], nothing is written to the drawing and no
/// `cad-history` entry is recorded. The typed argument is resolved against the
/// static vocabulary, so `UNDEFINE LINE` disables `LINE` and every alias whose
/// target is `LINE`. The essential commands ([`ESSENTIAL_COMMANDS`]) cannot be
/// undefined and are refused with `redefine.essential`.
pub const UNDEFINE_SELECTOR: &str = "UNDEFINE";

/// `REDEFINE <name>`: re-enable a built-in command disabled by `UNDEFINE`.
///
/// The counterpart of [`UNDEFINE_SELECTOR`]; session-only, no drawing effect. A
/// name that is not currently disabled is a named `redefine.not_undefined`
/// refusal, never a silent success.
pub const REDEFINE_SELECTOR: &str = "REDEFINE";

/// `ALIASEDIT`: the real runtime alias table (`ADD`/`REMOVE`/`LIST`).
///
/// The name itself is already a canonical `COMMAND_TABLE` selector (the existing
/// read-only `AliasEdit` report). `CadSession` intercepts the typed forms
/// `ALIASEDIT ADD <alias> <target>`, `ALIASEDIT REMOVE <alias>` and
/// `ALIASEDIT LIST` (bare `ALIASEDIT` is `LIST`) before the `CommandId` dispatch,
/// because the runtime rows are session state the application layer does not
/// hold. Session-only: no `acad.pgp` file is read or written.
pub const ALIASEDIT_SELECTOR: &str = "ALIASEDIT";

/// The selectors that exist only in the session's own resolution layer.
///
/// They are admitted names ([`canonical_command_name`] and
/// [`admissible_names`]) without a [`CommandId`], exactly like the shell
/// settings selectors: `CadSession::command` resolves and applies them before
/// the command table, so `normalize_command` keeps returning `None`.
pub const SESSION_SELECTORS: &[&str] = &[UNDEFINE_SELECTOR, REDEFINE_SELECTOR];

/// The commands `UNDEFINE` may never disable (AutoCAD's essential set).
///
/// Disabling these would remove the user's only way to recover (`REDEFINE`), to
/// undo the session change (`UNDEFINE`/`ALIASEDIT`), or to leave/reach a document
/// (`OPEN`/`SAVE`/`CANCEL`). Both `UNDEFINE` and `REDEFINE` refuse them with the
/// stable `redefine.essential` code, and `ALIASEDIT ADD` refuses to shadow them.
pub const ESSENTIAL_COMMANDS: &[&str] = &[
    ALIASEDIT_SELECTOR,
    "CANCEL",
    "OPEN",
    REDEFINE_SELECTOR,
    "SAVE",
    UNDEFINE_SELECTOR,
];

/// The `SPACE <argument>` selector: switch the active drawing space.
///
/// This is the same [`CommandId::SwitchSpace`] command the IPC/panel path uses;
/// the typed argument only saves a second IPC for a layout-tab click. `MODEL`
/// selects model space, `#<u128>` names a `LayoutId` directly, and any other
/// whole argument is resolved as a layout **display name** (exact match first,
/// then a unique case-insensitive one). A bare `SPACE` keeps the existing
/// explicit "needs a space payload" refusal.
pub const SPACE_SELECTOR: &str = "SPACE";

/// The canonical name in `table` matching the already-uppercased `key`, if any.
fn table_name<T>(table: &[(&'static str, T)], key: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(name, _)| *name)
}

/// The measurement kind an interaction-only selector names, if any.
///
/// The selector must already be canonical (as returned by
/// [`canonical_command_name`]); an alias never reaches here because aliases map
/// onto a canonical selector first.
pub fn measurement_selector_kind(name: &str) -> Option<MeasurementToolKind> {
    MEASUREMENT_SELECTORS
        .iter()
        .find(|(selector, _)| *selector == name)
        .map(|(_, kind)| *kind)
}

/// The fixed zoom factor a zoom-step selector names, if any.
pub fn zoom_step_factor(name: &str) -> Option<f64> {
    ZOOM_STEP_SELECTORS
        .iter()
        .find(|(selector, _)| *selector == name)
        .map(|(_, factor)| *factor)
}

/// The alias table as `(alias, canonical selector)` pairs, resolved through
/// [`canonical_command_name`].
///
/// This is what a host publishes to a frontend (the `cad_command_aliases` IPC):
/// the frontend may offer aliases in its suggestion list without keeping a
/// hand-written mirror of the table. An alias whose target does not resolve is
/// skipped rather than guessed; the alias invariants are contract-tested, so no
/// such row exists.
pub fn alias_pairs() -> Vec<(&'static str, &'static str)> {
    COMMAND_ALIASES
        .iter()
        .filter_map(|(alias, target)| canonical_command_name(target).map(|name| (*alias, name)))
        .collect()
}

/// The canonical, stable uppercase selector a command name resolves to, or
/// `None` when the name maps to no admitted command.
///
/// Covers [`COMMAND_TABLE`], the interaction-only [`MEASUREMENT_SELECTORS`] /
/// [`ZOOM_STEP_SELECTORS`], [`SHELL_SETTING_SELECTORS`], the session-only
/// [`SESSION_SELECTORS`] (`UNDEFINE`/`REDEFINE`) and [`COMMAND_ALIASES`]. Names are
/// matched case-insensitively after trimming surrounding whitespace; internal
/// whitespace is significant (multi-word selectors keep one space).
///
/// This is the **static** vocabulary: the session's runtime alias/undefined
/// tables are consulted first by `CadSession::command` (a runtime alias may
/// shadow any non-essential name, and a disabled name never dispatches), so this
/// function must stay free of session state.
pub fn canonical_command_name(text: &str) -> Option<&'static str> {
    let key = text.trim().to_ascii_uppercase();
    if let Some(name) = table_name(COMMAND_TABLE, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(REFUSAL_TABLE, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(MEASUREMENT_SELECTORS, &key) {
        return Some(name);
    }
    if let Some(name) = table_name(ZOOM_STEP_SELECTORS, &key) {
        return Some(name);
    }
    if let Some(name) = SHELL_SETTING_SELECTORS
        .into_iter()
        .find(|selector| *selector == key)
    {
        return Some(name);
    }
    if let Some(name) = SESSION_SELECTORS
        .iter()
        .copied()
        .find(|selector| *selector == key)
    {
        return Some(name);
    }
    COMMAND_ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .and_then(|(_, canonical)| canonical_command_name(canonical))
}

/// The canonical selector a [`CommandId`] is named by, if it has one.
///
/// This is the reverse of [`COMMAND_TABLE`] and exists for typed entry points
/// that dispatch a `CommandId` directly without going through a text selector,
/// so the command echo still records the same stable machine key the router
/// would. Interaction-only selectors (`DIST`, `ZOOMIN`, …) carry no
/// `CommandId` and are recorded by the selector path itself.
pub fn command_name(id: CommandId) -> Option<&'static str> {
    COMMAND_TABLE
        .iter()
        .chain(REFUSAL_TABLE.iter())
        .find(|(_, candidate)| *candidate == id)
        .map(|(name, _)| *name)
}

/// The `CommandId` a command name names, if any.
///
/// Covers the full [`COMMAND_TABLE`] vocabulary (plus [`COMMAND_ALIASES`]); an
/// unmapped name — including the interaction-only [`MEASUREMENT_SELECTORS`] /
/// [`ZOOM_STEP_SELECTORS`], which carry no `CommandId` — returns `None` so the
/// caller can dispatch or refuse it explicitly.
pub fn normalize_command(text: &str) -> Option<CommandId> {
    let name = canonical_command_name(text)?;
    COMMAND_TABLE
        .iter()
        .chain(REFUSAL_TABLE.iter())
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, id)| *id)
}

/// The stable rejection reason for a name that maps to no command.
pub fn unsupported_command_reason(text: &str) -> String {
    format!("unsupported CAD command: {}", text.trim())
}

/// Every name the host admits, for a frontend that must disable what cannot run
/// instead of keeping a hand-written mirror that drifts.
///
/// Canonical [`COMMAND_TABLE`] selectors come first, then the interaction-only
/// selectors ([`MEASUREMENT_SELECTORS`], [`ZOOM_STEP_SELECTORS`]), then the
/// shell-settings and session-only selectors, then every alias that is not
/// already canonical. Order is stable, so a frontend may sort it for display
/// without losing the canonical-first grouping.
pub fn admissible_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = COMMAND_TABLE
        .iter()
        .chain(REFUSAL_TABLE.iter())
        .map(|(name, _)| *name)
        .collect();
    for (name, _) in MEASUREMENT_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for (name, _) in ZOOM_STEP_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for name in SHELL_SETTING_SELECTORS {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    for name in SESSION_SELECTORS {
        if !names.contains(name) {
            names.push(name);
        }
    }
    for (alias, _) in COMMAND_ALIASES {
        if !names.contains(alias) {
            names.push(alias);
        }
    }
    names
}
