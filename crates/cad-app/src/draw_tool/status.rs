//! The draw preview's one-line capture step for the tool panel.
//!
//! Split out of [`super`] to keep the state-machine module inside the code-size
//! limit. [`DrawPreview::status_line`] stays an inherent method on the same type,
//! so every caller path (`DrawPreview::status_line`) is unchanged.

use super::{DrawPreview, DrawToolKind};

impl DrawPreview {
    /// One-line, locale-facing capture step for the tool panel.
    ///
    /// Kept in `cad-app` so the UI layer never re-implements the state
    /// machine's counting rules. The entity-picking kinds count *resolved
    /// entities* ([`DrawPreview::picks`]) rather than raw points, so the step text
    /// cannot claim a step that an empty-space click never resolved. The
    /// open-ended kinds report the captured count and the minimum to confirm.
    ///
    /// NOTE (pre-existing i18n deviation): the branches below hardcode Chinese.
    /// The open-ended branch adds **locale-neutral ASCII** text instead of more
    /// hardcoded copy, so this change does not widen that deviation.
    pub fn status_line(&self) -> String {
        // CHAMFEREDGE (ADR-0036): one pick, no text, and no edge highlight or
        // preview geometry exists — the copy must not imply either. The seed is
        // the pixel pick hit point and the body is the current selection.
        if self.kind == DrawToolKind::ChamferEdge {
            return if self.picks.is_empty() {
                "chamferedge: pick an edge on the selected body".to_string()
            } else {
                "chamferedge: edge picked, press Enter to chamfer".to_string()
            };
        }
        // PRESSPULL: same one-pick shape over the selected body; the signed
        // distance is the session typed channel, so the copy names no value.
        if self.kind == DrawToolKind::Extrude {
            return if self.picks.is_empty() {
                "extrude: select a closed profile, set the distance, press Enter".to_string()
            } else {
                "extrude: profile ready, press Enter".to_string()
            };
        }
        // FOLLOWME: zero points, no parameter. The selection is the whole input,
        // and the command is the one that can name a bad partition, so the copy
        // names Enter and nothing it cannot check here.
        if self.kind == DrawToolKind::FollowMe {
            return "followme: press Enter to sweep the selected profile along the selected path"
                .to_string();
        }
        // FREEHAND is a drag, not clicks: the line names the gesture and the
        // sample count, never "press Enter" (the release confirms).
        if self.kind == DrawToolKind::Freehand {
            return if self.points.is_empty() {
                "freehand: drag to trace a stroke; release to finish".to_string()
            } else {
                format!(
                    "freehand: {} sample(s) captured; release to finish",
                    self.points.len()
                )
            };
        }
        if self.kind == DrawToolKind::PositionCamera {
            return if self.points.is_empty() {
                "camera: pick the eye position".to_string()
            } else {
                "camera: pick the target point".to_string()
            };
        }
        if self.kind == DrawToolKind::PressPull {
            return if self.picks.is_empty() {
                "presspull: pick a face on the selected body".to_string()
            } else {
                "presspull: face picked, press Enter to move it".to_string()
            };
        }
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
        if self.kind.has_mixed_first_step() {
            // DIMANGULAR's first prompt is a real choice ("select a line **or**
            // specify a vertex"), so the line names the pending step instead of
            // counting points. Locale-neutral ASCII, like the other post-ADR-0015
            // fallback lines: the catalog carries the chrome copy.
            return if self.points.is_empty() {
                format!("{}: pick a line, or specify the vertex", self.kind.key())
            } else if self.picks.is_empty() {
                format!(
                    "{}: three-point form, {} point(s) captured, {} more needed",
                    self.kind.key(),
                    self.points.len(),
                    self.remaining
                )
            } else if self.picks.len() < 2 {
                format!("{}: pick the second line", self.kind.key())
            } else if self.ready {
                format!("{}: ready, press Enter to confirm", self.kind.key())
            } else {
                format!("{}: pick the dimension-arc location", self.kind.key())
            };
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
