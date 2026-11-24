//! The `LAYFRZ` **Viewports** argument form: the per-viewport freeze scope.
//!
//! Split out of `arguments.rs` for the code-size gate (the same split every other
//! grammar in this directory uses). The public paths are unchanged: `commands.rs`
//! re-exports this module, so `session::parse_layfrz_argument` and friends keep
//! resolving.
//!
//! AutoCAD reaches the scope through the command's Settings ▸ Viewports dialog;
//! this build has no in-command dialog, so the scope is typed on the same command
//! line, exactly like `PEDIT CLOSE` or `VPLAYER FREEZE <layers> [<index>]`. The
//! selection remains the whole input (it names the layers); only the viewport
//! scope is typed.

/// AutoCAD's `LAYFRZ VP [<index>,...]`: the per-viewport freeze scope.
///
/// An **argument form of the canonical `LAYFRZ` selector**, not a new command:
/// the selection still names the layers and the application still writes them
/// through `CommandId::LayFrz`, with
/// [`cad_app::SelectionToolPayload::LayFrzViewports`] as the scope. The bare
/// selector keeps its drawing-level freeze.
pub const LAYFRZ_SELECTOR: &str = "LAYFRZ";

/// Parse the argument of a `LAYFRZ VP [...]` selector.
///
/// Grammar: the keyword `VP` (also spelled `VIEWPORTS`, case-insensitive) alone
/// — the empty list, meaning **every** viewport of the active layout — or the
/// keyword plus a comma-separated list of **zero-based** viewport indices in
/// either the bare (`VP 0,1`) or the bracketed (`VP [0,1]`) spelling the
/// `VPLAYER` `[n]` token already uses. Signs, fractions, exponents, empty
/// entries (`0,,1`), an empty bracket pair and any other keyword are outside the
/// grammar (`None`), so the caller refuses the line by name
/// ([`invalid_layfrz_argument_reason`]). Nothing is resolved here: the
/// application validates every index against the active layout
/// (`layfrz.unknown_viewport`), dedupes and applies its current-viewport rule.
pub fn parse_layfrz_argument(argument: &str) -> Option<Vec<usize>> {
    let text = argument.trim();
    let (keyword, rest) = match text.find([' ', '\t']) {
        Some(index) => (&text[..index], text[index..].trim()),
        None => (text, ""),
    };
    if !(keyword.eq_ignore_ascii_case("VP") || keyword.eq_ignore_ascii_case("VIEWPORTS")) {
        return None;
    }
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let list = match rest.strip_prefix('[') {
        Some(inner) => inner.strip_suffix(']')?,
        None => rest,
    };
    let mut indices = Vec::new();
    for part in list.split(',') {
        let token = part.trim();
        if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        indices.push(token.parse::<usize>().ok()?);
    }
    (!indices.is_empty()).then_some(indices)
}

/// The stable rejection reason for a `LAYFRZ` argument outside the grammar.
pub fn invalid_layfrz_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (LAYFRZ without an argument freezes at drawing level, and \
         `LAYFRZ VP [<viewport-index>,...]` freezes in those viewports of the active layout; a \
         bare `LAYFRZ VP` is every viewport)",
        text.trim()
    )
}
