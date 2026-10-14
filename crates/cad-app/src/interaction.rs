//! Host- and UI-independent requests describing user interaction intent.
//!
//! These requests are separate from [`crate::Command`]: beginning a tool or
//! capturing a point does not itself create or commit a drawing transaction.

use cad_domain::{CadError, CadResult, Point3};

pub use crate::{DrawToolKind, MeasurementToolKind};

/// A point supplied either in world coordinates or in logical canvas space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointInput {
    World(Point3),
    CanvasLocal {
        logical_position: [f64; 2],
        metrics_revision: u64,
    },
}

impl PointInput {
    /// Reject non-finite coordinates before they enter the interaction protocol.
    pub fn validate(self) -> CadResult<Self> {
        let valid = match self {
            Self::World(point) => point.x.is_finite() && point.y.is_finite() && point.z.is_finite(),
            Self::CanvasLocal {
                logical_position, ..
            } => logical_position.into_iter().all(f64::is_finite),
        };
        if valid {
            Ok(self)
        } else {
            Err(CadError::InvalidInput(
                "interaction point must be finite".into(),
            ))
        }
    }
}

/// Revision-tagged dimensions needed to interpret logical canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportMetrics {
    logical_size: [f64; 2],
    physical_size: [u32; 2],
    scale_factor: f64,
    revision: u64,
}

impl ViewportMetrics {
    /// The three size/scale values are independently validated host-clipped
    /// measurements. Logical and physical dimensions may differ slightly from
    /// a scale-factor conversion because the host may round or clip them.
    pub fn new(
        logical_size: [f64; 2],
        physical_size: [u32; 2],
        scale_factor: f64,
        revision: u64,
    ) -> CadResult<Self> {
        let valid_logical = logical_size
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0);
        let valid_physical = physical_size.into_iter().all(|value| value > 0);
        if !valid_logical || !valid_physical || !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(CadError::InvalidInput(
                "viewport metrics require positive finite dimensions and scale".into(),
            ));
        }
        Ok(Self {
            logical_size,
            physical_size,
            scale_factor,
            revision,
        })
    }

    pub fn logical_size(self) -> [f64; 2] {
        self.logical_size
    }

    pub fn physical_size(self) -> [u32; 2] {
        self.physical_size
    }

    pub fn scale_factor(self) -> f64 {
        self.scale_factor
    }

    pub fn revision(self) -> u64 {
        self.revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// Identity of one pointer stream, stable across its phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointerId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Mouse,
    Touch,
    Pen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Replace,
    Add,
    Toggle,
    Remove,
}

/// UI-agnostic interaction intent. None of these variants imply execution.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionRequest {
    BeginDraw(DrawToolKind),
    BeginMeasurement(MeasurementToolKind),
    /// Change the retained measurement algorithm choice without starting capture.
    SetMeasurementKind(MeasurementToolKind),
    /// Replace any current capture with navigation pan mode; no zero-delta pan is emitted.
    BeginPan,
    /// Clear only the live cursor marker, preserving captured points and tool lifecycle.
    ClearCursor,
    SelectAt {
        point: PointInput,
        mode: SelectionMode,
    },
    CapturePoint(PointInput),
    MoveCursor(PointInput),
    Confirm,
    Cancel,
    Pointer {
        id: PointerId,
        kind: PointerKind,
        phase: PointerPhase,
        button: Option<PointerButton>,
        position: PointInput,
        modifiers: Modifiers,
    },
    Scroll {
        delta_logical: [f64; 2],
        position: PointInput,
        modifiers: Modifiers,
    },
}

impl InteractionRequest {
    /// Validate numeric input without assigning behavior to the request.
    pub fn validate(&self) -> CadResult<()> {
        match self {
            Self::CapturePoint(point) | Self::MoveCursor(point) => point.validate().map(|_| ()),
            Self::SelectAt { point, .. } => point.validate().map(|_| ()),
            Self::Pointer { position, .. } => position.validate().map(|_| ()),
            Self::Scroll {
                delta_logical,
                position,
                ..
            } => {
                if !delta_logical.iter().all(|value| value.is_finite()) {
                    return Err(CadError::InvalidInput("scroll delta must be finite".into()));
                }
                position.validate().map(|_| ())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_metrics_reject_invalid_dimensions_or_scale() {
        assert!(ViewportMetrics::new([640.0, 480.0], [1280, 960], 2.0, 7).is_ok());
        assert!(ViewportMetrics::new([0.0, 480.0], [1280, 960], 2.0, 7).is_err());
        assert!(ViewportMetrics::new([640.0, f64::NAN], [1280, 960], 2.0, 7).is_err());
        assert!(ViewportMetrics::new([640.0, 480.0], [1280, 0], 2.0, 7).is_err());
        assert!(ViewportMetrics::new([640.0, 480.0], [1280, 960], f64::INFINITY, 7).is_err());
    }

    #[test]
    fn synthetic_interaction_rejects_non_finite_coordinates() {
        let request = InteractionRequest::CapturePoint(PointInput::CanvasLocal {
            logical_position: [12.0, f64::NAN],
            metrics_revision: 3,
        });
        assert!(request.validate().is_err());
    }

    #[test]
    fn synthetic_pointer_identity_and_move_without_button_are_preserved() {
        let request = InteractionRequest::Pointer {
            id: PointerId(17),
            kind: PointerKind::Touch,
            phase: PointerPhase::Move,
            button: None,
            position: PointInput::CanvasLocal {
                logical_position: [8.0, 21.0],
                metrics_revision: 4,
            },
            modifiers: Modifiers::default(),
        };
        assert!(request.validate().is_ok());
        assert!(matches!(
            request,
            InteractionRequest::Pointer {
                id: PointerId(17),
                kind: PointerKind::Touch,
                phase: PointerPhase::Move,
                button: None,
                ..
            }
        ));
    }

    #[test]
    fn synthetic_selection_has_a_validated_target() {
        let request = InteractionRequest::SelectAt {
            point: PointInput::CanvasLocal {
                logical_position: [f64::INFINITY, 2.0],
                metrics_revision: 1,
            },
            mode: SelectionMode::Replace,
        };
        assert!(request.validate().is_err());
    }
}
