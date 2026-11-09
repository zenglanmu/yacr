//! `MSTRETCH`: the STRETCH planner over several crossing windows.
//!
//! The windows are logical-pixel rectangles, exactly like `STRETCH`'s single
//! `rect_logical`; each is mapped through the active viewport's orthographic
//! plan view. The edit reuses [`Application::stretch_windows`] and therefore the
//! one enclosure/vertex-move planner: an entity fully inside one window moves
//! whole, an entity hit by any window is partial, and a vertex/endpoint moves
//! when it lies inside any window. One transaction and one undo step cover the
//! whole command; every refusal happens before the transaction opens.
//!
//! `MSTRETCH` has no command-line argument grammar in this build (a rectangle
//! list needs a delimiter the one-argument selector grammar does not define);
//! hosts/IPC supply [`crate::CommandPayload::MStretch`].

use super::*;

use crate::app_drawing::stretch_window::StretchWindow;
use crate::select_box::Rect;

fn refusal(code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{code}: {detail}"))
}

impl Application {
    /// Apply the STRETCH planner to the union of the payload's crossing windows.
    pub(crate) fn mstretch_entities(
        &mut self,
        session: &SessionState,
        command: &Command,
    ) -> CadResult<CommandOutcome> {
        let CommandPayload::MStretch {
            rects_logical,
            base,
            delta,
        } = &command.payload
        else {
            return Err(refusal(
                "mstretch.payload",
                "MSTRETCH needs an MStretch { rects_logical, base, delta } payload",
            ));
        };
        if rects_logical.is_empty() {
            return Err(refusal(
                "mstretch.windows_empty",
                "MSTRETCH needs at least one crossing window",
            ));
        }
        if !rects_logical
            .iter()
            .flatten()
            .flatten()
            .all(|value| value.is_finite())
        {
            return Err(refusal(
                "mstretch.invalid_window",
                "MSTRETCH window corners must be finite",
            ));
        }
        if !is_finite_point(*base) || !is_finite_point(*delta) {
            return Err(refusal(
                "mstretch.invalid_point",
                "MSTRETCH base and displacement points must be finite",
            ));
        }
        if session.active_space != SpaceId::Model {
            return Err(CadError::Unsupported(
                "MSTRETCH currently supports model space only".into(),
            ));
        }
        let viewport = self
            .workspace
            .viewports
            .get(&command.viewport)
            .ok_or_else(|| CadError::InvalidInput("unknown viewport".into()))?;
        let size = viewport.logical_size;
        if !size.iter().all(|value| value.is_finite() && *value > 0.0) {
            return Err(refusal(
                "mstretch.invalid_viewport",
                "MSTRETCH viewport size must be finite and positive",
            ));
        }
        let mut rects = Vec::with_capacity(rects_logical.len());
        for rect_logical in rects_logical {
            let first = viewport
                .screen_to_world(rect_logical[0], size)
                .ok_or_else(|| {
                    CadError::Unsupported("MSTRETCH requires the orthographic plan view".into())
                })?;
            let second = viewport
                .screen_to_world(rect_logical[1], size)
                .ok_or_else(|| {
                    CadError::Unsupported("MSTRETCH requires the orthographic plan view".into())
                })?;
            let rect = Rect::from_corners(first, second);
            if !rect.width().is_finite()
                || !rect.height().is_finite()
                || rect.width() <= super::stretch::stretch_tolerance()
                || rect.height() <= super::stretch::stretch_tolerance()
            {
                return Err(refusal(
                    "mstretch.degenerate_window",
                    "every MSTRETCH crossing window needs non-zero width and height",
                ));
            }
            rects.push(rect);
        }
        let translation = Point3 {
            x: delta.x - base.x,
            y: delta.y - base.y,
            z: delta.z - base.z,
        };
        let length = (translation.x * translation.x
            + translation.y * translation.y
            + translation.z * translation.z)
            .sqrt();
        if !is_finite_point(translation) || !length.is_finite() || length <= 1e-12 {
            return Err(refusal(
                "mstretch.zero_displacement",
                "MSTRETCH displacement must be non-zero and finite (nothing to do)",
            ));
        }
        self.stretch_windows(
            session,
            command,
            rects.into_iter().map(StretchWindow::Rect).collect(),
            translation,
        )
    }
}
