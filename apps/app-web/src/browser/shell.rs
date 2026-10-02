//! Browser touch adapter. The Slint shell and application remain authoritative.
use super::{state_push, sync_view_camera, with_runtime};
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::{CadError, CadResult, Point3};

pub fn geometry() -> CadResult<Vec<f64>> {
    with_runtime(|rt| {
        let (rect, state) = rt.handle.shell_geometry()?;
        Ok(rect
            .into_iter()
            .chain(state.map(|v| f64::from(u8::from(v))))
            .collect())
    })
    .ok_or(CadError::Cancelled)?
}

pub fn pick(x: f64, y: f64) -> CadResult<()> {
    if !x.is_finite() || !y.is_finite() {
        return Err(CadError::InvalidInput("invalid touch pick".into()));
    }
    with_runtime(|rt| rt.handle.touch_pick(x, y)).ok_or(CadError::Cancelled)?
}

pub fn navigate(dx: f64, dy: f64, zoom: f64) -> CadResult<()> {
    if !dx.is_finite() || !dy.is_finite() || !zoom.is_finite() || !(0.2..=5.0).contains(&zoom) {
        return Err(CadError::InvalidInput("invalid touch navigation".into()));
    }
    with_runtime(|rt| {
        let mut controller = rt.controller.borrow_mut();
        let wpp = controller
            .application
            .workspace
            .viewports
            .get(&rt.viewport)
            .ok_or(CadError::Cancelled)?
            .world_per_px();
        let document = controller.document_id;
        for (id, payload) in [
            (
                CommandId::Pan,
                Point3 {
                    x: dx * wpp,
                    y: -dy * wpp,
                    z: 0.0,
                },
            ),
            (
                CommandId::Zoom,
                Point3 {
                    x: zoom,
                    y: 0.0,
                    z: 0.0,
                },
            ),
        ] {
            controller.execute(Command {
                schema_version: 1,
                id,
                document,
                viewport: rt.viewport,
                payload: CommandPayload::Points(vec![payload]),
            })?;
        }
        sync_view_camera(&controller, &rt.view, &rt.viewport);
        drop(controller);
        let slot = state_push::view_slot(&rt.view);
        state_push::push_panel_state(&rt.controller, &rt.handle, &slot);
        Ok(())
    })
    .ok_or(CadError::Cancelled)?
}
