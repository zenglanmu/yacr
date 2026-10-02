//! state module.

use super::*;

/// Snapshot of the measurement panel state pushed into the shell (audit U03/U04).
///
/// These are plain values so the whole panel can be derived from
/// `HostController::measurement_preview()` without the UI re-running the tool.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementUiState {
    /// Whether a tool is running.
    pub active: bool,
    /// Whether the confirm affordance is valid right now.
    pub can_confirm: bool,
    /// Whether a confirmed measurement exists to save as an annotation (F06/F07).
    ///
    /// Enabled by the host pushing `can_save_annotation` from
    /// `HostController::has_last_measurement()`; it is never true without a real
    /// confirmed record.
    pub can_save_annotation: bool,
    pub kind: MeasurementToolKind,
    /// Human-facing "picked N, need M" step text; empty when idle.
    pub step_label: String,
    pub unit_label: String,
}

impl Default for MeasurementUiState {
    fn default() -> Self {
        MeasurementUiState {
            active: false,
            can_confirm: false,
            can_save_annotation: false,
            kind: MeasurementToolKind::Distance,
            step_label: String::new(),
            unit_label: String::new(),
        }
    }
}

impl MeasurementUiState {
    /// Derive the panel state from an optional active preview and unit label.
    pub fn from_preview(
        preview: Option<&cad_app::MeasurementPreview>,
        unit_label: impl Into<String>,
    ) -> Self {
        match preview {
            Some(preview) => MeasurementUiState {
                active: true,
                can_confirm: preview.can_confirm(),
                can_save_annotation: false,
                kind: preview.kind,
                step_label: preview.status_line(),
                unit_label: unit_label.into(),
            },
            None => MeasurementUiState {
                active: false,
                can_confirm: false,
                can_save_annotation: false,
                kind: MeasurementToolKind::Distance,
                step_label: String::new(),
                unit_label: unit_label.into(),
            },
        }
    }

    /// Record whether a confirmed measurement is available to save.
    ///
    /// Hosts call this with `HostController::has_last_measurement()` after
    /// pushing the preview; the save affordance is enabled only when true.
    pub fn set_can_save_annotation(&mut self, can_save: bool) -> &mut Self {
        self.can_save_annotation = can_save;
        self
    }

    /// Combobox index for [`MeasurementToolKind::ALL`].
    pub fn kind_index(&self) -> i32 {
        self.kind.index() as i32
    }
}

/// View/observation state pushed into the shell (F13/F14).
///
/// `is_3d` drives the 2D/3D affordance and the adapter's orbit-by-drag gate;
/// `perspective` drives the projection toggle's pressed state. Both are
/// derivations of the authoritative application viewport, never invented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewStateUi {
    pub is_3d: bool,
    pub perspective: bool,
}

/// Mode switch state pushed into the shell (audit U02).
///
/// `work` drives the Work-only affordances (`enabled: work`) and `label` is the
/// catalog text for the *current* mode. Both are derived from the authoritative
/// `cad_app::AppMode`, so the shell can never show a mode the command layer does
/// not enforce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeUiState {
    pub work: bool,
    /// Catalog label for the current mode (`mode.enhanced` / `mode.viewer`).
    pub label: String,
}

impl ModeUiState {
    /// Derive the mode state from the authoritative application mode.
    pub fn from_mode(mode: cad_app::AppMode, messages: &MessageSource) -> Self {
        ModeUiState {
            work: mode == cad_app::AppMode::Work,
            label: status::mode_label(messages, mode),
        }
    }
}

impl Default for ModeUiState {
    fn default() -> Self {
        ModeUiState {
            work: true,
            label: String::new(),
        }
    }
}

/// One layer row pushed into the shell (audit F03/U03).
///
/// `id` is only a display value; the adapter keeps the ordered `LayerId` list so
/// a toggle maps back to the exact id without a lossy `u128 → i32` cast.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerRowUi {
    pub id: i32,
    pub name: String,
    /// Effective visibility the scene honours (database flag ⊕ override).
    pub visible: bool,
    /// Whether a temporary session override is active for this layer.
    pub overridden: bool,
}

/// Layer-panel snapshot derived from [`cad_app::layers::LayerRow`]s.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerPanelState {
    pub rows: Vec<LayerRowUi>,
    /// Number of layers currently overridden; drives the "restore" affordance.
    pub override_count: usize,
    /// Explicit empty-state text shown when there are no layers.
    pub empty_label: String,
}

impl LayerPanelState {
    /// Build the panel state from real layer rows.
    pub fn from_rows(rows: &[cad_app::layers::LayerRow], empty_label: impl Into<String>) -> Self {
        LayerPanelState {
            rows: rows
                .iter()
                .map(|row| LayerRowUi {
                    // Display-only: the low 32 bits are enough to label a row;
                    // the exact LayerId round-trips through the adapter's order.
                    id: (row.id.0 & 0xFFFF_FFFF) as i32,
                    name: row.name.clone(),
                    visible: row.effective_visible,
                    overridden: row.is_overridden(),
                })
                .collect(),
            override_count: rows.iter().filter(|r| r.is_overridden()).count(),
            empty_label: empty_label.into(),
        }
    }
}

/// One layout row pushed into the shell (audit F04/U03).
///
/// `id` is a display value; the adapter keeps the ordered `LayoutId` list so a
/// switch maps back to the exact id. `supported` is false when the layout has a
/// viewport this build cannot draw; `reason` then explains why.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutRowUi {
    pub id: i32,
    pub name: String,
    pub supported: bool,
    pub reason: String,
    pub viewport_count: i32,
}

/// Layout-panel snapshot derived from the database's real layout table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutPanelState {
    pub rows: Vec<LayoutRowUi>,
    /// Index of the active row in `rows`, or `None` for model space.
    pub active_index: Option<i32>,
    /// Explicit empty-state text shown when the drawing has no paper layouts.
    pub empty_label: String,
}

impl LayoutPanelState {
    /// Build the panel state from the representation layer's descriptors.
    pub fn from_descriptors(
        descriptors: &[cad_representation::LayoutDescriptor],
        active: cad_representation::SpaceSelection,
        empty_label: impl Into<String>,
    ) -> Self {
        let rows: Vec<LayoutRowUi> = descriptors
            .iter()
            .map(|d| LayoutRowUi {
                // Display-only: the low 32 bits label a row; the exact LayoutId
                // round-trips through the adapter's order.
                id: (d.id.0 & 0xFFFF_FFFF) as i32,
                name: d.name.clone(),
                supported: d.supported,
                reason: d.reason.clone(),
                viewport_count: d.viewport_count as i32,
            })
            .collect();
        let active_index = active.layout().and_then(|id| {
            descriptors
                .iter()
                .position(|d| d.id == id)
                .map(|i| i as i32)
        });
        LayoutPanelState {
            rows,
            active_index,
            empty_label: empty_label.into(),
        }
    }
}

/// One read-only property row pushed into the shell (audit F05/U03).
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRowUi {
    pub key: String,
    pub value: String,
}

/// One annotation row pushed into the shell (audit F09/U03).
///
/// As with layers, the `int` id is display-only; the adapter keeps the ordered
/// `AnnotationId` list so a visibility toggle / delete maps back exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationRowUi {
    pub id: i32,
    pub kind: String,
    pub text: String,
    /// Effective visibility the scene should honour.
    pub visible: bool,
    /// Whether a temporary session override is active for this annotation.
    pub overridden: bool,
    /// Whether this annotation is the current management selection.
    pub selected: bool,
}

/// Annotation management panel snapshot (F09) plus the active tool state (F07).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnnotationPanelState {
    pub rows: Vec<AnnotationRowUi>,
    /// Number of annotations currently hidden by a session override.
    pub hidden_count: usize,
    /// Explicit empty-state text shown when there are no annotations.
    pub empty_label: String,
    /// Whether an annotation creation tool is running (drives confirm/cancel).
    pub tool_active: bool,
    /// Whether the active tool has all its required parameters.
    pub tool_can_confirm: bool,
    /// Selected kind index for the tool selector.
    pub tool_kind_index: i32,
    /// Human-facing step text from the tool state machine; empty when idle.
    pub tool_step_label: String,
    /// Whether the tool's text payload has been supplied (text/leader kinds).
    pub text_supplied: bool,
    /// Whether the active kind needs a text payload at all.
    pub requires_text: bool,
}

impl AnnotationPanelState {
    /// Build the panel state from real management rows plus the tool preview.
    pub fn from_rows(
        rows: &[cad_app::AnnotationRow],
        preview: Option<&cad_app::AnnotationPreview>,
        empty_label: impl Into<String>,
    ) -> Self {
        let (active, can_confirm, kind_index, step, text_supplied, requires_text) = match preview {
            Some(preview) => (
                true,
                preview.can_confirm(),
                preview.kind.index() as i32,
                preview.status_line(),
                preview.text_supplied,
                preview.requires_text,
            ),
            None => (false, false, 0, String::new(), false, false),
        };
        AnnotationPanelState {
            rows: rows
                .iter()
                .map(|row| AnnotationRowUi {
                    // Display-only; the adapter keeps the exact ids in order.
                    id: (row.id.0 & 0xFFFF_FFFF) as i32,
                    kind: row.kind_label.to_string(),
                    text: row.text.clone(),
                    visible: row.visible,
                    overridden: row.overridden,
                    selected: row.selected,
                })
                .collect(),
            hidden_count: rows.iter().filter(|r| r.is_hidden()).count(),
            empty_label: empty_label.into(),
            tool_active: active,
            tool_can_confirm: can_confirm,
            tool_kind_index: kind_index,
            tool_step_label: step,
            text_supplied,
            requires_text,
        }
    }
}

/// Properties-panel snapshot for the current selection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertyPanelState {
    pub rows: Vec<PropertyRowUi>,
    /// Count of selected objects (0 = empty state).
    pub count: usize,
    /// Explicit empty-state text when nothing is selected.
    pub empty_label: String,
    /// Text describing keys whose values differ across a multi-select.
    pub mixed_label: String,
}

impl PropertyPanelState {
    /// Build the panel state from a real [`cad_app::SelectionProperties`].
    pub fn from_properties(
        properties: &cad_app::SelectionProperties,
        empty_label: impl Into<String>,
        mixed_label: impl Fn(&[&'static str]) -> String,
    ) -> Self {
        if properties.empty {
            return PropertyPanelState {
                rows: Vec::new(),
                count: 0,
                empty_label: empty_label.into(),
                mixed_label: String::new(),
            };
        }
        PropertyPanelState {
            rows: properties
                .rows
                .iter()
                .map(|row| PropertyRowUi {
                    key: row.key.to_string(),
                    value: row.value.clone(),
                })
                .collect(),
            count: properties.count,
            empty_label: empty_label.into(),
            mixed_label: if properties.mixed_keys.is_empty() {
                String::new()
            } else {
                mixed_label(&properties.mixed_keys)
            },
        }
    }
}
