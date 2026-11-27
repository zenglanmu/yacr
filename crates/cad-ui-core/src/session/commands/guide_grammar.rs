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
        "unsupported CAD command: {} (GUIDE takes LIST, CLEAR, DELETE <n> or POINT <x> <y> <z>; \
         bare GUIDE lists)",
        text.trim()
    )
}
