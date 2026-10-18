//! Per-kind "draw one entity" conveniences on [`HostController`].
//!
//! Split out of `host/mod.rs` to keep that module inside the code-size limit.
//! Each helper builds the `Command` for one create command and runs it through the
//! one `HostController::execute` funnel, so a host never assembles a transaction
//! itself and every create is one entity / one transaction / one undo step.
//!
//! `create_line`/`create_circle`/`create_polyline`/`create_rectangle` were public
//! methods on `HostController` before this split; they still are, only defined in
//! this module, so their paths are unchanged.

use super::*;

impl HostController {
    /// Draw a LINE through two world points in one transaction (spec F-EDIT).
    pub fn create_line(&mut self, start: Point3, end: Point3) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreateLine,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(vec![start, end]),
        })
    }

    /// Draw a CIRCLE from a centre and an edge point in one transaction.
    pub fn create_circle(&mut self, center: Point3, edge: Point3) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreateCircle,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(vec![center, edge]),
        })
    }

    /// Draw an open LWPOLYLINE through world points in one transaction.
    pub fn create_polyline(&mut self, points: Vec<Point3>) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreatePolyline,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(points),
        })
    }

    /// Draw a closed four-vertex RECTANGLE through two opposite corners in one
    /// transaction. Any corner order is accepted; the stored outline is
    /// canonicalised by the command layer.
    pub fn create_rectangle(
        &mut self,
        corner1: Point3,
        corner2: Point3,
    ) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreateRectangle,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(vec![corner1, corner2]),
        })
    }

    /// Draw a full ELLIPSE by the axis-endpoint method in one transaction.
    ///
    /// `p1`/`p2` are the two endpoints of the first axis and `p3` gives the other
    /// half-axis distance; the command layer stores the longer axis as
    /// `major_axis` (`ratio <= 1`) and refuses a degenerate or non-coplanar
    /// triple.
    pub fn create_ellipse(
        &mut self,
        p1: Point3,
        p2: Point3,
        p3: Point3,
    ) -> CadResult<CommandOutcome> {
        self.execute(Command {
            schema_version: 1,
            id: crate::CommandId::CreateEllipse,
            document: self.document_id,
            viewport: self.viewport_id,
            payload: crate::CommandPayload::Points(vec![p1, p2, p3]),
        })
    }
}
