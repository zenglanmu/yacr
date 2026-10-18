use super::mtext::{skip_param_code, TAB_STEP};

/// Strip MTEXT/TEXT formatting so the raw glyph text can be shaped.
///
/// Handles:
///
/// - `\P`/`\p` line breaks, `\~` non-breaking space and `\\` escapes;
/// - `%%d`/`%%p`/`%%c` symbols and brace grouping;
/// - `\S<num><sep><den>;` stacked fractions, rendered as two lines (see below);
/// - `\t` and `^I` tabs, expanded to `TAB_STEP` spaces;
/// - no-argument toggles `\L`/`\l` (underline), `\O`/`\o` (overline) and
///   `\K`/`\k` (strikethrough), which affect styling we do not render;
/// - parameter codes `\H`, `\W`, `\A`, `\Q`, `\C`, `\f`, `\F`, … terminated by
///   `;`, consumed without emitting text.
///
/// Unknown escapes are dropped. This is an approximation, not a full MTEXT
/// layout engine: stacked fractions use the normal line spacing with no
/// fraction bar or vertical scaling, tabs do not align to true tab stops, and
/// MTEXT columns are ignored.
pub fn sanitize_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // DXF caret notation: `^I` is a TAB control character.
        if c == '^' && i + 1 < chars.len() && matches!(chars[i + 1], 'I' | 'i') {
            for _ in 0..TAB_STEP {
                out.push(' ');
            }
            i += 2;
            continue;
        }
        if c == '\\' {
            if i + 1 >= chars.len() {
                i += 1;
                continue;
            }
            let n = chars[i + 1];
            match n {
                'P' | 'p' => {
                    out.push('\n');
                    i += 2;
                }
                '~' => {
                    out.push(' ');
                    i += 2;
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                }
                't' => {
                    // `\t` is a tab; expand to a fixed run of spaces.
                    for _ in 0..TAB_STEP {
                        out.push(' ');
                    }
                    i += 2;
                }
                'S' => {
                    i = push_stacked_fraction(&chars, i, &mut out);
                }
                // No-argument style toggles. These must not scan for `;`, or
                // they would swallow the rest of the run.
                'L' | 'l' | 'O' | 'o' | 'K' | 'k' => {
                    i += 2;
                }
                // Parameter codes: `\H<val>;`, `\W<val>;`, `\A<val>;`,
                // `\Q<val>;`, `\C<val>;`, `\f<name|…>;`, etc. Consume the value
                // and its terminating `;` without emitting text.
                'H' | 'W' | 'A' | 'Q' | 'C' | 'f' | 'F' => {
                    i = skip_param_code(&chars, i + 2);
                }
                _ if n.is_ascii_alphabetic() => {
                    i = skip_param_code(&chars, i + 2);
                }
                _ => i += 2,
            }
            continue;
        }
        if c == '{' || c == '}' {
            i += 1;
            continue;
        }
        if c == '%' && i + 2 < chars.len() && chars[i + 1] == '%' {
            match chars[i + 2].to_ascii_lowercase() {
                'd' => {
                    out.push('°');
                    i += 3;
                    continue;
                }
                'p' => {
                    out.push('±');
                    i += 3;
                    continue;
                }
                'c' => {
                    out.push('⌀');
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Render an MTEXT stacked fraction starting at the `\` of `\S`.
///
/// The separator is the first of `^` (centre), `/` (horizontal bar) or `#`
/// (diagonal). All three are rendered the same way here: numerator and
/// denominator on consecutive lines, in the order written, using the normal
/// line spacing. There is no fraction bar, no vertical scaling and no reduced
/// font size — a deliberate, documented approximation. Returns the index just
/// past the consumed `\S…;` code.
fn push_stacked_fraction(chars: &[char], backslash: usize, out: &mut String) -> usize {
    let end = skip_param_code(chars, backslash + 2);
    let content_end = if end > backslash + 2 && chars.get(end - 1) == Some(&';') {
        end - 1
    } else {
        end
    };
    let content: String = chars[backslash + 2..content_end].iter().collect();
    match content
        .char_indices()
        .find(|(_, ch)| matches!(ch, '^' | '/' | '#'))
    {
        Some((idx, sep)) => {
            out.push_str(&sanitize_text(&content[..idx]));
            out.push('\n');
            out.push_str(&sanitize_text(&content[idx + sep.len_utf8()..]));
        }
        None => out.push_str(&sanitize_text(&content)),
    }
    end
}
