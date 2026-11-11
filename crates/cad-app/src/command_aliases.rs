//! The canonical command **alias** table, owned by `cad-app`.
//!
//! This table lives here, not in `cad-ui-core`, because the `ALIASEDIT`
//! read-only report runs in `cad-app` (`Application::execute` → extended-command
//! dispatch) and `cad-app` may not depend on the UI layer. Keeping the one copy
//! in the lower crate and re-exporting it from
//! `cad_ui_core::session::commands::COMMAND_ALIASES` means there is still
//! exactly one authority: the router, the docs gate and the report all read these
//! same rows.
//!
//! Full-word synonyms and AutoCAD-style abbreviations, always accepted, that map
//! onto a canonical `COMMAND_TABLE` selector (which stays in `cad-ui-core`).
//!
//! The single/double-letter rows mirror AutoCAD's default `acad.pgp` command
//! abbreviations **for the commands this build already implements only**: an
//! alias is admitted only when its target is a real selector, so `MA`
//! (`MATCHPROP`) is present now that MATCHPROP exists (and was absent while the
//! command itself was absent). There is no `acad.pgp` file parsing; this table is
//! the whole (curated) shipped subset, and every row is contract-tested to
//! canonicalise onto a `COMMAND_TABLE` name, to be unique, and to collide with no
//! canonical name.
//!
//! Runtime aliases are **session state**, not rows here: `CadSession` owns the
//! `ALIASEDIT ADD|REMOVE|LIST` command-line forms (`crates/cad-ui-core`), which
//! shadow these rows and the canonical selectors for the lifetime of the session
//! and are never written back to a file. `ALIASEDIT`'s payload-path report below
//! therefore still describes this build table only.
//!
//! There is no `SNAP` alias row. AutoCAD's `SNAP` is **grid snap** (F9), a real
//! selector of its own (`CommandId::ToggleSnapGrid`, with the `SNAPUNIT` spacing
//! sysvar); it is a different feature from object snap (`OSNAP`) and from the
//! display grid (`GRID`). Aliasing it onto `ToggleSnap` would silently turn a
//! request for grid snap into an object-snap toggle, and aliasing it onto
//! `ToggleGrid` would silently turn it into a display toggle — neither is grid
//! snap, so no alias exists and the canonical name is the only spelling.
pub const COMMAND_ALIASES: &[(&str, &str)] = &[
    ("EXPORT", "PLOT"),
    ("PRINT", "PLOT"),
    ("SAVEAS", "SAVE"),
    ("QSAVE", "SAVE"),
    ("ZOOM EXTENTS", "FIT"),
    ("ENTER", "CONFIRM"),
    // `DBA`/`DCO` are the conventional AutoCAD abbreviations of the baseline and
    // continue dimension pair. Admitting them is our declared convenience (the
    // AutoCAD command reference documents the full names, not these shorthand
    // forms), and an alias never changes behaviour: it only normalizes a typed
    // name onto the canonical selector (ADR 0017 decision C, P5f).
    // `LAYUNISO` is AutoCAD's "turn layer isolation off", which is exactly what
    // `RESTORELAYERS` does here (drop the session's temporary overrides).
    // `LAYON` ("turn all layers on") is the same restore reached by its own name.
    //
    // `LAYERP` is deliberately **not** in this table any more (2026-10-27): it is a
    // canonical selector of its own ([`CommandId::LayerPrevious`]) that restores
    // the *previous* layer state from the session's bounded override history. An
    // alias onto `RESTORELAYERS` would silently clear every override instead.
    ("LAYUNISO", "RESTORELAYERS"),
    // `LAYON` is AutoCAD's "turn all layers on", which is exactly what the one
    // restore path does here (drop every temporary override). It carries no
    // `CommandId` of its own, like `LAYUNISO`.
    ("LAYON", "RESTORELAYERS"),
    // The command-line (`-`) spellings of the 2026-11-07 layer batch name the
    // same commands: this build has no layer dialog, so the dialog-free form is
    // the whole behaviour and a second `CommandId` would be a second path.
    ("-LAYMCH", "LAYMCH"),
    ("-LAYMRG", "LAYMRG"),
    ("-LAYDEL", "LAYDEL"),
    // `-OVERKILL` is AutoCAD's dialog-free spelling of the same command; this
    // build has one selection-based path, so the alias adds no second engine.
    ("-OVERKILL", "OVERKILL"),
    ("DBA", "DIMBASELINE"),
    ("DCO", "DIMCONTINUE"),
    // `NEWLAYER` is a **project** word synonym for creating a layer, not an
    // AutoCAD command (AutoCAD reaches layer creation through `LAYER` /
    // `-LAYER`); it is admitted only because `CREATELAYER` itself exists, the
    // same rule every other alias row follows. It is the only non-`acad.pgp`
    // spelling in this table, and it changes nothing but the normalized name.
    ("NEWLAYER", "CREATELAYER"),
    // AutoCAD's default `acad.pgp` single/double-letter abbreviations, listed
    // alphabetically (draw, modify, view, annotate, drafting switches). Only the
    // subset whose target selector exists here: `MA` (MATCHPROP) joined the table
    // when MATCHPROP itself was implemented, and inventing an abbreviation for an
    // absent command is exactly what this table must never do.
    ("A", "ARC"),
    ("AL", "ALIGN"),
    // AutoCAD's `AR` is ARRAY; `ARRAYRECT` is the array command this build
    // implements, so the abbreviation points at the implemented selector.
    ("AR", "ARRAYRECT"),
    // AutoCAD's `BR` is BREAK, this build's two-point break.
    ("BR", "BREAK"),
    ("C", "CIRCLE"),
    ("CHA", "CHAMFER"),
    ("CO", "COPY"),
    ("CP", "COPY"),
    ("DAL", "DIMALIGNED"),
    ("DAN", "DIMANGULAR"),
    ("DCE", "DIMCENTER"),
    ("DDI", "DIMDIAMETER"),
    // `DIV` / `LEN` are AutoCAD's abbreviations of the two zero-point single-curve
    // edits implemented here (DIVIDE and LENGTHEN).
    ("DIV", "DIVIDE"),
    ("DLI", "DIMLINEAR"),
    ("DRA", "DIMRADIUS"),
    ("E", "ERASE"),
    ("EL", "ELLIPSE"),
    ("EX", "EXTEND"),
    ("F", "FILLET"),
    ("GR", "GRID"),
    ("H", "HATCH"),
    // AutoCAD's default abbreviation for INSERT (an existing block reference).
    ("I", "INSERT"),
    // AutoCAD's default abbreviation for BLOCK (define a named block).
    ("B", "BLOCK"),
    ("L", "LINE"),
    ("LEN", "LENGTHEN"),
    ("M", "MOVE"),
    ("MA", "MATCHPROP"),
    ("MI", "MIRROR"),
    ("O", "OFFSET"),
    ("OR", "ORTHO"),
    ("OS", "OSNAP"),
    ("P", "PAN"),
    ("PL", "PLINE"),
    ("PO", "POINT"),
    ("POL", "POLYGON"),
    ("REC", "RECTANG"),
    ("RO", "ROTATE"),
    ("S", "STRETCH"),
    ("SC", "SCALE"),
    ("SPL", "SPLINE"),
    ("T", "TEXT"),
    ("TR", "TRIM"),
    ("U", "UNDO"),
    ("X", "EXPLODE"),
    // AutoCAD's default abbreviation for XLINE. `RAY` deliberately has no alias:
    // AutoCAD's `acad.pgp` defines none, and this table never invents spellings.
    ("XL", "XLINE"),
    ("Z", "ZOOM"),
    // The 2026-11-07 text batch aliases. `TJUST` is AutoCAD's `JUSTIFYTEXT`
    // alias (same `CommandId`, same typed `<H> <V>` grammar); `MTEDIT` is
    // AutoCAD's name for editing the selected text and maps onto the existing
    // `TEXTEDIT` pick tool; `-MTEXT` is AutoCAD's command-line MTEXT form,
    // mapping onto the existing two-corner `MTEXT` tool. None is a second path.
    ("TJUST", "JUSTIFYTEXT"),
    ("MTEDIT", "TEXTEDIT"),
    ("-MTEXT", "MTEXT"),
    // The command-line (`-`) spellings of PURGE/RENAME name the same commands:
    // this build has no dialog form, so the dialog-free selector is the whole
    // behaviour and a second `CommandId` would be a second path.
    ("-PURGE", "PURGE"),
    ("-RENAME", "RENAME"),
    // `-VIEW` is AutoCAD's command-line form of `VIEW`. This build has no view
    // dialog, so the dialog-free typed form is the whole behaviour and the alias
    // maps onto the same `CommandId`/path — never a second gallery.
    ("-VIEW", "VIEW"),
    // `-VPORTS` is AutoCAD's command-line spelling of VPORTS; this build has no
    // viewport dialog, so the alias only normalizes the name onto the one
    // selector and its one named refusal.
    ("-VPORTS", "VPORTS"),
    // `-DIMSTYLE` is AutoCAD's dialog-free spelling of the same command; this
    // build has one selector path, so it normalizes onto `DIMSTYLE`.
    ("-DIMSTYLE", "DIMSTYLE"),
    // `-GROUP` is AutoCAD's command-line group form; this build has one
    // session-only group path, so it normalizes onto `GROUP`.
    ("-GROUP", "GROUP"),
    // The 2026-11-07 extras aliases: `REGEN3` is the 3D spelling of `REGEN`, and
    // `-PAN`/`-LINETYPE` are the command-line spellings of the same paths.
    ("REGEN3", "REGEN"),
    ("-PAN", "PAN"),
    ("-LINETYPE", "LINETYPE"),
];
