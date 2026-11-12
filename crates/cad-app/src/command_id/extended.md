//! The sustained "extended command" wrapper and its batches.
//!
//! [`ExtendedCommandId`] is deliberately **not** a second command funnel: each
//! variant is carried by the single [`crate::CommandId::Extended`] variant and
//! dispatched by `crate::app_dispatch`. That keeps the main `CommandId` enum
//! from growing one arm per payload-free helper while the vocabulary table still
//! names every selector.
//!
//! # The pilot batch (2026-11-09)
//!
//! All ten are **Viewer-legal session/report commands**: they open no
//! transaction, bump no revision and record no `cad-history` entry. Five are
//! session display toggles (`QTEXT`, `FILL`, `TFRAMES`, `REGENAUTO`,
//! `ISODRAFT`); their behaviour is not consumed by this build, so the handler
//! reports an honest partial instead of claiming a visual effect. Five are
//! read-only reports (`BCOUNT`, `COUNT`, `EDITTIME`, `JULIAN`, `XLIST`).
//!
//! # Wave 2 (2026-11-09)
//!
//! Thirty more payload-free names, grouped exactly as they are dispatched:
//!
//! * **Session tokens** (`DRAGMODE`, `APERTURE`, `ISOPLANE`, `HIGHLIGHTNEW`,
//!   `TRAYSETTINGS`, `TEXTSCR`, `GRAPHSCR`, `FINDINCLUDE`, `FINDMATCH`,
//!   `REVCLOUDPROPERTIES`): one session flag/token each, no database write and
//!   no renderer consumer; the two read-only forms (`APERTURE`,
//!   `REVCLOUDPROPERTIES`) report their token and name their setter as NOT
//!   IMPLEMENTED.
//! * **Read-only reports** (`SYSVARMONITOR`, `SYSVDLG`, `VTOPTIONS`,
//!   `ALIASEDIT`, `WHOHAS`, `ACADINFO`, `PTYPE`, `SCALELISTEDIT`): pure reads of
//!   data this build really has; anything absent is a named `not_tracked`/
//!   `write_unsupported` diagnostic, never an invented value.
//! * **Named refusals** (`HYPERLINKOPTIONS`, `INPUTSEARCHOPTIONS`,
//!   `-INPUTSEARCHOPTIONS`, `SIGVALIDATE`, `BASE`, `LIMITS`,
//!   `VIEWPLOTDETAILS`, `VPLAYER`, `PSBSCALE`, `PSTSCALE`,
//!   `MVSETUP`): `CadError::Unsupported` with a stable reason string, because
//!   the feature has no storage model or no API in this build. A refusal writes
//!   nothing, bumps no revision and records no history. (`MVIEW` graduated to a
//!   real Wave 13 viewport write and is no longer a refusal.)
//!
//! # Wave 3 (2026-11-09)
//!
//! Thirty more payload-free names, again grouped by dispatch:
//!
//! * **Reports / session** (`MSPACE`, `PSPACE`, `STYLE`, `-STYLE`,
//!   `MLEADERSTYLE`, `BLOCK?`, `DIMREASSOC`, `DIMREASSOCIATE`): two space
//!   switches that reuse the existing `MODEL`/`SPACE` session path (never a
//!   second space authority), read-only style/block reports over the tables this
//!   build really stores, and two non-associative no-op reports mirroring
//!   `DIMREGEN`.
//! * **Named refusals** (`DIMJOGGED` through `CONSTRAINTSETTINGS`): features
//!   with no storage model or API in this build, refused with a stable
//!   `CadError::Unsupported` reason.
//!
//! Every variant in Waves 1–3 stays Viewer-legal: no database write, no
//! revision bump, no `cad-history` entry.
//!
//! # Wave 4 (2026-11-09)
//!
//! Thirty more names, and the first batch with **real entity writes**:
//!
//! * **Selection-based 2D edits** (`CENTERMARK`, `CENTERLINE`, `NCOPY`,
//!   `COPYTOLAYER`, `-COPYTOLAYER`, `CDORDER`, `FLATTEN`, `XPLODE`, `BTRIM`,
//!   `BEXTEND`, `CHANGE`, `CHPROP`): each reuses an existing planner/write path
//!   (creation funnel, `plan_trim`/`plan_extend`, `plan_entity`, EXPLODE). They
//!   are Work-only and commit exactly one transaction / one undo step.
//! * **Named refusals** (`BSCALE` through `CHSPACE`): features with no storage
//!   model or API in this build, refused with a stable
//!   `CadError::Unsupported` reason. Refusals stay Viewer-legal.
//!
//! # Wave 5 (2026-11-09)
//!
//! Thirty more names over the attribute and external-reference surfaces:
//!
//! * **Session token** (`ATTDISP`): one `attdisp` flag (default ON) in
//!   [`crate::session_state::DraftingExtras`]. This build has no block-attribute
//!   model, so nothing is rendered either way and the outcome says exactly that.
//! * **Read-only reports** (`BATTMAN`, `BATTORDER`, `XREF`, `-XREF`,
//!   `EXTERNALREFERENCES`, `EXTERNALREFERENCESCLOSE`): block definitions/member
//!   counts from `drawing.blocks()` and external-reference provenance from
//!   `drawing.external_references()`. Absent data is named
//!   (`*.attributes_not_modeled`, `externalreferences.palette_not_implemented`),
//!   never invented.
//! * **Named refusals** (`ATTDEF` through `CLASSICXREF`): no block-attribute
//!   model, no entity XDATA store, no xref bind/attach/edit/palette API. Each is
//!   `CadError::Unsupported` with a stable reason.
//!
//! Every Wave 5 variant stays Viewer-legal: no database write, no revision bump,
//! no `cad-history` entry.
//!
//! # Wave 6 (2026-11-09)
//!
//! Thirty more names over the file/plot/export surfaces:
//!
//! * **Session tokens** (`DWGLOG`, `LOGFILEON`, `LOGFILEOFF`): one `dwglog`
//!   flag (default OFF) in [`crate::session_state::DraftingExtras`]. This build
//!   writes no log file, so the handler reports the token and says exactly that.
//! * **Read-only reports** (`DWGPROPS`, `DWFFORMAT`, `EXPORTPDF`): document
//!   id/revision/entity counts, and two honest capability reports that name the
//!   absent export path instead of pretending to export. None is an error.
//! * **Named refusals** (`SAVEALL` through `AUTOPUBLISH`): no multi-document
//!   model, no reopen-to-source path, no plotter/plot-style/export-settings
//!   model, no plot preview/stamp, no batch publish/eTransmit/archive packaging.
//!   Each is `CadError::Unsupported` with a stable reason.
//!
//! Every Wave 6 variant stays Viewer-legal: no database write, no revision bump,
//! no `cad-history` entry, and no host-owned file I/O is re-implemented here.
//!
//! # Wave 7 (2026-11-09)
//!
//! Thirty more names over the dynamic-block and constraint surfaces:
//!
//! * **Read-only reports** (`BVSTATE`, `-BVSTATE`): the active dynamic-block
//!   visibility state of every directly selected INSERT, read through
//!   `DrawingDatabase::block_dynamic_visibility` (the read half of
//!   `set_block_visibility_state`). The report never sets a state; a selection
//!   with no INSERT is `*.no_selection`, and an INSERT whose block records no
//!   resolved state is `*.not_set` — nothing is invented.
//! * **Named refusals** (`BACTION` through `DCLINEAR`): no dynamic-block action,
//!   parameter, lookup-table, construction-geometry or visibility-hide/show
//!   model, no block authoring palette/editor, no parameter-reset API, and no
//!   geometric/dimensional constraint model. Each is `CadError::Unsupported`
//!   with a stable reason.
//!
//! Every Wave 7 variant stays Viewer-legal: no database write, no revision bump,
//! no `cad-history` entry.
//!
//! # Wave 8 (2026-11-09)
//!
//! `ELEV` (read-only session elevation report; setter NOT IMPLEMENTED) and
//! `UCSICON` (session flag, no icon) plus eight named refusals (`UCS`, `UCSMAN`,
//! `TABLET`, `TASKBAR`, `REINIT`, `SYSWINDOWS`, `MENU`, `RIBBON`) for the absent
//! UCS/tablet/menu models and host-owned chrome. All Viewer-legal.
