//! The typed `SHADOWS`/`FOG` payload row.
//!
//! Both selectors write the one per-document display record
//! (`tables/display.json`, `.ymodel` minor 22): the Shadows sun (a shading
//! direction, **not** a cast-shadow map) and the Fog distance band. A bare
//! selector reports the current record and writes nothing.
//!
//! The typed grammar lives in `cad-ui-core`
//! (`SHADOWS [ON|OFF|SUN <azimuth> <elevation>]`,
//! `FOG [ON|OFF|RANGE <start> <end>|COLOR <r> <g> <b>]`), and the application
//! re-validates every value so a direct payload caller gets the same refusal.

/// One typed display-settings write (or a bare-selector report).
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayCommand {
    /// `SHADOWS ON|OFF`.
    Shadows(bool),
    /// `SHADOWS SUN <azimuth> <elevation>`: angles in degrees.
    Sun {
        /// Degrees clockwise from world `+Y`, in `[0, 360)`.
        azimuth: f64,
        /// Degrees above the horizon, in `[0, 90]`.
        elevation: f64,
    },
    /// A bare `SHADOWS`: report the record, write nothing.
    ReportShadows,
    /// `FOG ON|OFF`.
    Fog(bool),
    /// `FOG RANGE <start> <end>`: model-unit distances.
    FogRange {
        /// Distance where the fog begins.
        start: f64,
        /// Distance where geometry is fully fogged.
        end: f64,
    },
    /// `FOG COLOR <r> <g> <b>`: sRGB bytes.
    FogColor([u8; 3]),
    /// A bare `FOG`: report the record, write nothing.
    ReportFog,
}
