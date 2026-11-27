//! The `GUIDE [LIST|CLEAR|DELETE <n>|POINT <x> <y> <z>]` typed grammar.
//!
//! Bare `GUIDE` reports the table; `LIST` is the same report, `CLEAR` removes
//! everything, `DELETE <n>` removes the 1-based row and `POINT` appends a point
//! marker. Nothing else is admitted.

/// The `GUIDE` selector.
pub const GUIDE_SELECTOR: &str = "GUIDE";

/// One admitted `GUIDE` argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GuideArgument {
    List,
    Clear,
    /// 1-based row index.
    Delete(usize),
    /// World point for a point marker.
    Point(cad_domain::Point3),
    /// Shared sRGB colour.
    StyleColor([u8; 3]),
    /// Shared line dash switch.
    StyleDash(bool),
}

/// Parse the argument of a `GUIDE <option>` line.
pub fn parse_guide_argument(argument: &str) -> Option<GuideArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    match tokens.as_slice() {
        [option] if option.eq_ignore_ascii_case("LIST") => Some(GuideArgument::List),
        [option] if option.eq_ignore_ascii_case("CLEAR") => Some(GuideArgument::Clear),
        [option, index] if option.eq_ignore_ascii_case("DELETE") => {
            let index = index.parse::<usize>().ok()?;
            (index > 0).then_some(GuideArgument::Delete(index))
        }
        [style, kind, r, g, b]
            if style.eq_ignore_ascii_case("STYLE") && kind.eq_ignore_ascii_case("COLOR") =>
        {
            Some(GuideArgument::StyleColor([
                r.parse::<u8>().ok()?,
                g.parse::<u8>().ok()?,
                b.parse::<u8>().ok()?,
            ]))
        }
        [style, kind, value]
            if style.eq_ignore_ascii_case("STYLE") && kind.eq_ignore_ascii_case("DASH") =>
        {
            if value.eq_ignore_ascii_case("ON") || *value == "1" {
                Some(GuideArgument::StyleDash(true))
            } else if value.eq_ignore_ascii_case("OFF") || *value == "0" {
                Some(GuideArgument::StyleDash(false))
            } else {
                None
            }
        }
        [option, x, y, z] if option.eq_ignore_ascii_case("POINT") => {
            let point = cad_domain::Point3 {
                x: x.parse::<f64>().ok().filter(|value| value.is_finite())?,
                y: y.parse::<f64>().ok().filter(|value| value.is_finite())?,
                z: z.parse::<f64>().ok().filter(|value| value.is_finite())?,
            };
            Some(GuideArgument::Point(point))
        }
        _ => None,
    }
}

/// The stable rejection reason for a `GUIDE` line outside the grammar.
pub fn invalid_guide_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (GUIDE takes LIST, CLEAR, DELETE <n>, POINT <x> <y> <z> or \
         STYLE COLOR <r> <g> <b>|DASH ON|OFF; bare GUIDE lists)",
        text.trim()
    )
}
