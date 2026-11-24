//! The legacy `QLEADER` / `LEADER` creation input (split out of
//! `payload_extra.rs` for the code-size gate).
//!
//! Both names create one `MULTILEADER` through the ordinary `MLEADER` payload;
//! this value is the typed input the session parses and the application maps onto
//! that one creation path. The fields beyond the two points and the text are the
//! shared `CONTENT` kind plus `QLEADER`'s own `ARROW`/`MAXPOINTS` settings.

use cad_domain::Point3;

/// The input of a `QLEADER` / `LEADER` line (both map to one `MULTILEADER`).
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderCommand {
    /// The arrowhead point.
    pub arrow: Point3,
    /// The landing point (where the leader line ends and the text begins).
    pub landing: Point3,
    /// The annotation text. Empty is a real case: a block-content leader takes
    /// none and a text-content one becomes a content-less leader (`172 = 0`).
    pub text: String,
    /// The annotation content kind (`CONTENT MTEXT|BLOCK <name>|TOLERANCE`), or
    /// `None` for the text-driven default. It is the same
    /// [`MLeaderContentSpec`](crate::MLeaderContentSpec) the `MLEADER` payload
    /// carries, so block/tolerance leaders follow one set of domain rules.
    pub content: Option<crate::MLeaderContentSpec>,
    /// `QLEADER ARROW ON|OFF`: `Some(false)` stores no arrowhead on the created
    /// leader line (`LEADER_LINE 40 = 0` with the per-line override bit);
    /// `None`/`Some(true)` draw the resolved style arrowhead.
    pub arrowhead: Option<bool>,
    /// `QLEADER MAXPOINTS <n>`: the requested maximum vertex count. The creation
    /// has exactly two vertices, so a request above that is the named
    /// `mleader.option_max_points` refusal and nothing is written.
    pub max_points: Option<u32>,
}
