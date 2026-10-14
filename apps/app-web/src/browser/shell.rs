//! Browser touch adapter. The Slint shell and application remain authoritative.
use super::{run_command, with_runtime};
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

pub fn canvas_hit_test(x: f64, y: f64) -> CadResult<bool> {
    with_runtime(|rt| rt.handle.canvas_hit_test([x, y])).ok_or(CadError::Cancelled)?
}

pub fn cancel_draw_capture() -> CadResult<()> {
    with_runtime(|rt| rt.handle.cancel_draw_capture()).ok_or(CadError::Cancelled)?
}

pub fn navigate(dx: f64, dy: f64, zoom: f64) -> CadResult<()> {
    if !dx.is_finite() || !dy.is_finite() || !zoom.is_finite() || !(0.2..=5.0).contains(&zoom) {
        return Err(CadError::InvalidInput("invalid touch navigation".into()));
    }
    with_runtime(|rt| {
        let viewport = rt.viewport();
        let (wpp, document) = {
            let controller = rt.core.controller.borrow();
            let wpp = controller
                .application
                .workspace
                .viewports
                .get(&viewport)
                .ok_or(CadError::Cancelled)?
                .world_per_px();
            (wpp, controller.document_id)
        };
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
            run_command(
                &rt.core,
                Command {
                    schema_version: 1,
                    id,
                    document,
                    viewport,
                    payload: CommandPayload::Points(vec![payload]),
                },
            )?;
        }
        Ok(())
    })
    .ok_or(CadError::Cancelled)?
}
