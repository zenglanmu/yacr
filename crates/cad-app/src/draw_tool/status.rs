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
