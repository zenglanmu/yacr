//! The `SECTIONPLANE [ON|OFF|TOGGLE|CLEAR|LIST|SET ...]` typed grammar.
//!
//! A bare selector toggles the stored plane (the menu action); `LIST` reports it
//! without writing. `SET` takes an origin and a non-zero normal; the app
//! normalizes and validates it.

use cad_domain::Point3;

/// The `SECTIONPLANE` selector.
pub const SECTIONPLANE_SELECTOR: &str = "SECTIONPLANE";

/// One admitted `SECTIONPLANE` argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SectionArgument {
    Set { origin: Point3, normal: Point3 },
    On,
    Off,
    Toggle,
    Clear,
    List,
}

/// Parse the argument of a `SECTIONPLANE <option>` line.
pub fn parse_section_argument(argument: &str) -> Option<SectionArgument> {
    let tokens: Vec<&str> = argument.split_whitespace().collect();
    match tokens.as_slice() {
        [word] if word.eq_ignore_ascii_case("ON") => Some(SectionArgument::On),
        [word] if word.eq_ignore_ascii_case("OFF") => Some(SectionArgument::Off),
        [word] if word.eq_ignore_ascii_case("TOGGLE") => Some(SectionArgument::Toggle),
        [word] if word.eq_ignore_ascii_case("CLEAR") => Some(SectionArgument::Clear),
        [word] if word.eq_ignore_ascii_case("LIST") => Some(SectionArgument::List),
        [set, ox, oy, oz, nx, ny, nz] if set.eq_ignore_ascii_case("SET") => {
            let number = |raw: &str| raw.parse::<f64>().ok().filter(|value| value.is_finite());
            Some(SectionArgument::Set {
                origin: Point3 {
                    x: number(ox)?,
                    y: number(oy)?,
                    z: number(oz)?,
                },
                normal: Point3 {
                    x: number(nx)?,
                    y: number(ny)?,
                    z: number(nz)?,
                },
            })
        }
        _ => None,
    }
}

/// The stable rejection reason for a `SECTIONPLANE` line outside the grammar.
pub fn invalid_section_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (SECTIONPLANE takes ON, OFF, TOGGLE, CLEAR, LIST or \
         SET <ox> <oy> <oz> <nx> <ny> <nz>; bare SECTIONPLANE toggles)",
        text.trim()
    )
}
