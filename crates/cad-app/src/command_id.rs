//! The command catalogue: every command the application can execute.
//!
//! `command_id.rs` used to hold the enum directly; it is split so the enum (and
//! its long per-command contracts) can grow past the 1000-line code gate while
//! the sibling contract modules stay declared here. The public path is
//! unchanged: `crate::CommandId` / `crate::AppMode` are re-exported from
//! `ids.rs`.

mod app_mode;
mod capabilities;
mod ids;
pub use app_mode::AppMode;
pub use ids::CommandId;

pub mod area_object;
pub mod block_define;
pub mod boundary;
pub mod break_at_point;
pub mod construction_line;
pub mod dimension_chain;
pub mod dimension_extra;
pub mod draw_variants;
pub mod gradient;
pub mod grip_stretch;
pub mod hatch;
pub mod hatch_generate_boundary;
pub mod hatch_to_back;
pub mod hatchedit;
pub mod massprop;
pub mod match_prop;
pub mod measuregeom;
pub mod oops;
pub mod overkill;
pub mod pedit;
pub mod point_style;
pub mod reverse;
pub mod select_filter;
pub mod text_commands;
pub mod texttofront;
pub mod zoom;
