//! The draw-capture preview projection (UI-facing, read-only).
//!
//! Split out of [`super`] to keep the capture state machine's module inside the
//! code-size limit; [`super::DrawPreview`] is re-exported under its original name,
//! so `crate::draw_tool::DrawPreview` and the crate-root re-export are unchanged.

use cad_domain::{Point3, SelectionRef};

use super::kind::{DrawToolKind, PointArity};

/// Snapshot of an in-progress draw for the UI preview layer.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawPreview {
    pub kind: DrawToolKind,
    /// Captured points in order.
    pub points: Vec<Point3>,
    /// Entities resolved at capture time, in step order.
    ///
    /// Empty for the free-point kinds. For the entity-picking kinds
    /// ([`DrawToolKind::needs_picked_entities`]) this is one parallel entry per
    /// captured step, so readiness is honest: a TRIM/EXTEND with two raw points
    /// but fewer than two resolved entities is **not** confirmable.
    pub picks: Vec<SelectionRef>,
    /// Live cursor position, if any; never committed until confirmed.
    pub cursor: Option<Point3>,
    /// The entity under the **current cursor** for the pending capture step, or
    /// `None`.
    ///
    /// Resolved through the exact pick path a click uses, only while a step is
    /// pending, and never fabricated: it is presentation feedback for the
    /// follow-up highlight wiring, not a commit source.
    pub candidate: Option<SelectionRef>,
    /// Captured points still needed before the operation is defined.
    ///
    /// This is the *minimum* complement ([`PointArity::minimum`] minus the
    /// captured count), so it is `0` once the operation can be confirmed. For an
    /// **open-ended** tool a further point is still accepted while this is `0`;
    /// the honest projection is [`Self::remaining_points`] (which returns `None`
    /// there) — this raw count is retained as the legacy numeric field the
    /// `cad-host`/`cad-engine` wire shape reads.
    pub remaining: usize,
    /// Number of selected entities the tool was started with (MOVE needs > 0).
    pub selection_count: usize,
    /// Whether the operation can be committed right now.
    pub ready: bool,
    /// The POLYGON side count for a live POLYGON capture, or `None` for every
    /// other kind.
    ///
    /// The count is the session sysvar (ADR 0015 decision C), injected by the
    /// session when it builds the preview so the outline tracks the *current*
    /// value even if it changes mid-capture. It is `None` whenever the active
    /// tool is not POLYGON, which is the same rule the frozen frontend
    /// `CadTool.params.sides` projection uses.
    pub polygon_sides: Option<u32>,
    /// Whether the capture is a text tool whose point complement is complete and
    /// whose typed text has not been submitted yet (ADR 0019 decision A.2).
    ///
    /// This is the **only** honest source for the frontend's text prompt phase: a
    /// UI must not infer it from `kind` + `points`, because `ready` alone cannot
    /// distinguish "missing points" from "missing text". While this is `true` no
    /// `Confirm` can complete the command — `SetToolText` is the terminal step.
    pub awaiting_text: bool,
    /// The **round-trippable current string** of the entity a text tool will
    /// edit, or `None` (ADR 0020 amendment A).
    ///
    /// Only `TEXTEDIT` fills this, and only once its entity pick resolved to a
    /// single `Text`: it is that entity's stored string, sent so the UI can show
    /// it for editing instead of making the user retype it. A fresh `TEXT` never
    /// has a prefill (there is nothing to edit yet), and no non-text tool
    /// projects one. It is **not** a display row: it is the exact value
    /// `TEXTEDIT` will replace.
    pub text_prefill: Option<String>,
}

impl DrawPreview {
    /// Whether the captured parameters can be committed right now.
    ///
    /// A text tool whose points are complete but whose text has not been
    /// submitted is **not** confirmable: `SetToolText` is the terminal step
    /// (ADR 0019 decision A.1), so reporting `true` here would claim a `Confirm`
    /// that the host refuses.
    pub fn can_confirm(&self) -> bool {
        self.ready && !self.awaiting_text
    }

    /// The capture arity of the tool this preview describes.
    pub fn arity(&self) -> PointArity {
        self.kind.arity()
    }

    /// Whether this capture is open-ended (accepts points past its minimum).
    pub fn is_open_ended(&self) -> bool {
        self.kind.arity().is_open_ended()
    }

    /// Honest projection of how many more points the capture requires.
    ///
    /// `Some(k)` means exactly `k` more points are needed to become confirmable
    /// (`Some(0)` = the exact complement is complete). `None` means the minimum
    /// is met on an **open-ended** tool: more points are still accepted up to
    /// the hard cap and the user finishes with Enter, so there is no fixed
    /// "remaining" count to report (never a fabricated `0`).
    pub fn remaining_points(&self) -> Option<usize> {
        let minimum = self.kind.required_points();
        if self.points.len() < minimum {
            Some(minimum - self.points.len())
        } else if self.kind.arity().is_open_ended() {
            None
        } else {
            Some(0)
        }
    }

    /// One-line, locale-facing capture step for the tool panel.
    ///
    /// Kept in `cad-app` so the UI layer never re-implements the state
    /// machine's counting rules. The entity-picking kinds count *resolved
    /// entities* ([`Self::picks`]) rather than raw points, so the step text
    /// cannot claim a step that an empty-space click never resolved. The
    /// open-ended kinds report the captured count and the minimum to confirm.
    ///
    /// NOTE (pre-existing i18n deviation): the branches below hardcode Chinese.
    /// The open-ended branch adds **locale-neutral ASCII** text instead of more
    /// hardcoded copy, so this change does not widen that deviation.
    pub fn status_line(&self) -> String {
        if self.kind.requires_text() {
            // TEXT/TEXTEDIT (ADR 0019 decision A, ADR 0020 amendment A):
            // locale-neutral ASCII only — the catalog (`tool.prompt.text.*`,
            // `status.tool_text.*`) is the chrome source, and this frozen line
            // must not add CJK literals on the Rust side.
            if self.kind == DrawToolKind::TextEdit {
                return if self.awaiting_text {
                    format!(
                        "{}: text picked ({}), type the replacement",
                        self.kind.key(),
                        self.picks.len()
                    )
                } else {
                    format!("{}: pick a text entity to edit", self.kind.key())
                };
            }
            if self.kind == DrawToolKind::MLeader {
                return if self.awaiting_text {
                    format!(
                        "{}: arrow and landing captured ({}), type the text",
                        self.kind.key(),
                        self.points.len()
                    )
                } else {
                    format!(
                        "{}: specify the arrow position and the landing point",
                        self.kind.key()
                    )
                };
            }
            // MTEXT: the two corners define the wrap rectangle (its width is the
            // horizontal span); the text is the terminal step. Locale-neutral
            // ASCII like the other fallback lines.
            if self.kind == DrawToolKind::MText {
                return if self.awaiting_text {
                    format!(
                        "{}: two corners captured ({}), type the text",
                        self.kind.key(),
                        self.points.len()
                    )
                } else {
                    format!(
                        "{}: specify the first and the opposite corner of the wrap rectangle",
                        self.kind.key()
                    )
                };
            }
            return if self.awaiting_text {
                format!(
                    "{}: insertion point captured ({}), type the text",
                    self.kind.key(),
                    self.points.len()
                )
            } else {
                format!("{}: specify the insertion point", self.kind.key())
            };
        }
        if self.kind.arity().is_open_ended() {
            let minimum = self.kind.required_points();
            // DIMBASELINE/DIMCONTINUE: the first capture step is the base
            // dimension pick, so the measured count excludes it (locale-neutral
            // ASCII, like the other open-ended lines below).
            if matches!(
                self.kind,
                DrawToolKind::DimBaseline | DrawToolKind::DimContinue
            ) {
                return if self.picks.is_empty() {
                    format!("{}: pick a linear dimension as the base", self.kind.key())
                } else {
                    format!(
                        "{}: {} point(s) after the base, press Enter to finish",
                        self.kind.key(),
                        self.points.len().saturating_sub(1)
                    )
                };
            }
            if self.points.len() < minimum {
                return format!(
                    "{}: {} point(s) captured, at least {} needed",
                    self.kind.key(),
                    self.points.len(),
                    minimum
                );
            }
            return format!(
                "{}: {} point(s) captured, press Enter to finish",
                self.kind.key(),
                self.points.len()
            );
        }
        if self.kind.requires_selection() && self.selection_count == 0 {
            return format!("{}：请先选择对象", self.kind.label());
        }
        if self.kind == DrawToolKind::Trim {
            return match self.picks.len() {
                0 => "修剪：请选择要修剪的目标".to_string(),
                1 => "修剪：请选择边界".to_string(),
                _ => "修剪：已选目标与边界，可确认".to_string(),
            };
        }
        if self.kind == DrawToolKind::Extend {
            return match self.picks.len() {
                0 => "延伸：请选择要延伸的目标".to_string(),
                1 => "延伸：请选择边界".to_string(),
                _ => "延伸：已选目标与边界，可确认".to_string(),
            };
        }
        if self.remaining > 0 {
            format!(
                "{}：已选 {} 点，还需 {} 点",
                self.kind.label(),
                self.points.len(),
                self.remaining
            )
        } else {
            format!(
                "{}：已选 {} 点，可确认",
                self.kind.label(),
                self.points.len()
            )
        }
    }
}
