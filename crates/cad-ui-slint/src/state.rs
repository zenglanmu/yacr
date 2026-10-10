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
                kind: preview.kind,
                step_label: preview.status_line(),
                unit_label: unit_label.into(),
            },
            None => MeasurementUiState {
                active: false,
                can_confirm: false,
                kind: MeasurementToolKind::Distance,
                step_label: String::new(),
                unit_label: unit_label.into(),
            },
        }
    }

    /// Combobox index for [`MeasurementToolKind::ALL`].
    pub fn kind_index(&self) -> i32 {
        self.kind.index() as i32
    }
}

/// View/observation state pushed into the shell (F13/F14).
///
/// `is_3d` drives the 2D/3D affordance and the adapter's orbit-by-drag gate;
/// `perspective` drives the projection toggle's pressed state. `standard_view`
/// is the named view the authoritative camera currently matches, if any, so the
/// observation drawer can mark the active row without a second camera copy.
/// Every field is a derivation of the application viewport, never invented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewStateUi {
    pub is_3d: bool,
    pub perspective: bool,
    /// The standard view matching the active camera, or `None` for a free orbit.
    pub standard_view: Option<cad_app::StandardView>,
}

/// The standard view a camera currently matches, if any (F13).
///
/// Compares the camera's view direction, up axis and projection family against
/// every [`cad_app::StandardView`] within a small tolerance. `None` means the
/// view is a free orbit (or an orientation that is not a named standard view);
/// it is never guessed. Pure, so the observation drawer's active row is tested
/// without a window.
pub fn standard_view_for_camera(camera: &cad_app::Camera) -> Option<cad_app::StandardView> {
    use cad_app::camera::{dot3, normalize3, ViewBasis};
    const TOL: f64 = 1e-3;
    let actual = camera.view_basis().ok()?;
    for view in cad_app::StandardView::ALL {
        let offset = normalize3(view.eye_offset())?;
        // The camera looks from `target + offset * distance`, so its forward
        // direction is the negated, normalised offset.
        let forward = cad_domain::Point3 {
            x: -offset.x,
            y: -offset.y,
            z: -offset.z,
        };
        let Ok(expected) = ViewBasis::from_forward_up(forward, view.up_hint()) else {
            continue;
        };
        if dot3(actual.forward, expected.forward) < 1.0 - TOL
            || dot3(actual.up, expected.up) < 1.0 - TOL
        {
            continue;
        }
        // Projection family must agree: the plan view is orthographic, the rest
        // are 3D perspective views.
        if view.is_plan() != camera.projection.is_orthographic() {
            continue;
        }
        return Some(view);
    }
    None
}

/// 3D observation drawer snapshot (F13), derived from [`ViewStateUi`].
///
/// Consolidates the observation mode, projection, active standard view and the
/// orbit interaction model into one pushable state. `orbit_available` and the
/// fit reason are explicit: a control the build cannot dispatch shows why rather
/// than faking a command. All text comes from the active catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct View3dPanelState {
    pub is_3d: bool,
    pub perspective: bool,
    /// Index into [`cad_app::StandardView::ALL`] the camera matches, or `None`.
    pub standard_view_index: Option<i32>,
    /// Catalog labels for the standard-view selector, in `StandardView::ALL` order.
    pub standard_view_labels: Vec<String>,
    /// Whether drag-to-orbit is available (only in 3D mode).
    pub orbit_available: bool,
    /// Human-facing orbit status: the drag hint in 3D, the explicit reason in 2D.
    pub orbit_status: String,
    /// Explicit reason the 3D zoom-to-fit control cannot dispatch.
    pub fit_reason: String,
}

impl View3dPanelState {
    /// Derive the drawer state from the pushed view state and the catalog.
    pub fn from_view_state(state: &ViewStateUi, messages: &MessageSource) -> Self {
        let standard_view_index = state.standard_view.map(|view| view.index() as i32);
        View3dPanelState {
            is_3d: state.is_3d,
            perspective: state.perspective,
            standard_view_index,
            standard_view_labels: crate::status::standard_view_labels(messages),
            orbit_available: state.is_3d,
            orbit_status: if state.is_3d {
                messages.text("view.orbit.drag", &[])
            } else {
                messages.text("view.orbit.needs_3d", &[])
            },
            fit_reason: messages.text("view.fit3d_unavailable", &[]),
        }
    }
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

/// UI-facing snapshot of the asynchronous open progress panel (F01).
///
/// Derived from [`cad_app::ImportProgressSnapshot`]. It carries **only** real
/// values: `percent` is `Some` only when the importer reported an entity total
/// (otherwise the shell must show an indeterminate bar), and no byte text is
/// produced when the byte count is unknown (so a UI never renders "0 bytes").
/// The panel is hidden while idle and after a successful open; cancelled and
/// failed terminals are explicit.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportProgressUiState {
    /// Whether the progress panel should be shown.
    pub visible: bool,
    /// Localized phase label, or the explicit terminal text.
    pub phase_label: String,
    /// Completion in `0.0..=1.0`, only when the real entity total is known.
    pub percent: Option<f32>,
    /// Localized "entities N/M" or indeterminate text, plus bytes when known.
    pub progress_text: String,
    /// Whether the cancel affordance should be enabled.
    pub cancellable: bool,
    /// Whether this snapshot is a terminal cancelled/failed state.
    pub terminal: bool,
    /// The raw controller snapshot, kept so a live locale switch can re-derive
    /// the labels without the host re-polling. `None` when idle.
    pub source: Option<cad_app::ImportProgressSnapshot>,
}

impl ImportProgressUiState {
    /// Derive the panel state from a controller snapshot and the active catalog.
    ///
    /// `None` means the controller is idle: the panel is hidden and no label is
    /// fabricated.
    pub fn from_snapshot(
        snapshot: Option<&cad_app::ImportProgressSnapshot>,
        messages: &MessageSource,
    ) -> Self {
        let Some(snapshot) = snapshot else {
            return ImportProgressUiState::default();
        };
        match &snapshot.terminal {
            // A successful open replaces the document; the progress panel is
            // done and hidden (the document itself is the feedback).
            Some(cad_app::ImportTerminal::Opened { .. }) => ImportProgressUiState {
                visible: false,
                ..ImportProgressUiState::default()
            },
            // Cancelled/failed terminals are explicit and stay visible until a
            // new open supersedes them.
            Some(cad_app::ImportTerminal::Cancelled) => ImportProgressUiState {
                visible: true,
                phase_label: messages.text("import.terminal.cancelled", &[]),
                percent: None,
                progress_text: String::new(),
                cancellable: false,
                terminal: true,
                source: Some(snapshot.clone()),
            },
            Some(cad_app::ImportTerminal::Failed { error }) => ImportProgressUiState {
                visible: true,
                phase_label: messages
                    .text("import.terminal.failed", &[("error", &error.to_string())]),
                percent: None,
                progress_text: String::new(),
                cancellable: false,
                terminal: true,
                source: Some(snapshot.clone()),
            },
            // Running (with or without a first tick yet).
            None => {
                let phase_key = snapshot.phase_key().unwrap_or("unknown");
                let phase_label = messages.text(&format!("import.phase.{phase_key}"), &[]);
                // A percent exists only for a known, positive total; a known
                // zero total is indeterminate rather than a division by zero.
                let percent = snapshot
                    .entities_total
                    .filter(|total| *total > 0)
                    .map(|total| {
                        (snapshot.entities_done.min(total) as f32 / total as f32).clamp(0.0, 1.0)
                    });
                let base = match snapshot.entities_total {
                    Some(total) => messages.text(
                        "import.progress.count",
                        &[
                            ("done", &snapshot.entities_done.to_string()),
                            ("total", &total.to_string()),
                        ],
                    ),
                    None => messages.text(
                        "import.progress.indeterminate",
                        &[("done", &snapshot.entities_done.to_string())],
                    ),
                };
                let progress_text = match snapshot.bytes {
                    Some(bytes) => {
                        let separator = messages.text("import.detail.separator", &[]);
                        let bytes_text = messages
                            .text("import.progress.bytes", &[("bytes", &bytes.to_string())]);
                        format!("{base}{separator}{bytes_text}")
                    }
                    None => base,
                };
                ImportProgressUiState {
                    visible: true,
                    phase_label,
                    percent,
                    progress_text,
                    cancellable: snapshot.cancellable,
                    terminal: false,
                    source: Some(snapshot.clone()),
                }
            }
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
    /// Real viewport scale text (e.g. `1:100`) pushed by the host, if any.
    ///
    /// `None` means the host has not pushed a scale for this layout, so the
    /// panel shows an explicit "scale unavailable" reason instead of inventing a
    /// ratio. The representation descriptors do not carry a scale, so
    /// [`LayoutPanelState::from_descriptors`] always starts `None`.
    pub viewport_scale: Option<String>,
}

/// True while no application command changes a layout's viewport scale.
///
/// `cad-app::CommandId` has no viewport-scale variant, so the layout panel
/// surfaces the scale read-only and explains why. Flipping this when such a
/// command lands makes the panel interactive without a shell change.
pub const LAYOUT_SCALE_CONTROL_WIRED: bool = false;

/// Layout-panel snapshot derived from the database's real layout table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutPanelState {
    pub rows: Vec<LayoutRowUi>,
    /// Index of the active row in `rows`, or `None` for model space.
    pub active_index: Option<i32>,
    /// Explicit empty-state text shown when the drawing has no paper layouts.
    pub empty_label: String,
    /// Whether the host can dispatch a viewport-scale change from the panel.
    ///
    /// There is no `CommandId` for viewport scale, so this stays false and the
    /// control renders disabled with an explicit reason. Kept as a field so a
    /// future command can enable it without touching the shell.
    pub scale_control_available: bool,
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
                // The descriptors carry no scale; a host pushes it explicitly.
                viewport_scale: None,
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
            scale_control_available: LAYOUT_SCALE_CONTROL_WIRED,
        }
    }

    /// Attach a real viewport scale to the layout row at `index`.
    ///
    /// Hosts map a layout's per-viewport transform to a display string (for
    /// example `1:100`) and call this before pushing the state. Returns false
    /// when `index` is out of range, so a bad index can never invent a row.
    pub fn set_viewport_scale(&mut self, index: usize, scale: impl Into<String>) -> bool {
        match self.rows.get_mut(index) {
            Some(row) => {
                row.viewport_scale = Some(scale.into());
                true
            }
            None => false,
        }
    }
}

/// One read-only property row pushed into the shell (audit F05/U03).
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyRowUi {
    pub key: String,
    pub value: String,
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
