//! Session-only bounded multiline style store (`MLSTYLE`).
//!
//! The store is display/session state: it feeds `MLINE`'s spacing and is never
//! written to the drawing, history or preferences. The cap is
//! [`MULTILINE_STYLE_CAP`] entries; creating one more is an explicit
//! `mlstyle.capacity` refusal, never an eviction that silently changes what a
//! saved style means.
//!
//! Full contract: [`crate::command_id::draw2::MlStyleContract`].

/// The largest number of named multiline styles the session keeps.
pub const MULTILINE_STYLE_CAP: usize = 16;

/// The longest accepted style name, in characters.
const MAX_STYLE_NAME_CHARS: usize = 64;

/// The spacing of the default `STANDARD` style, in drawing units.
pub const DEFAULT_MULTILINE_SPACING: f64 = 1.0;

/// One named, session-only multiline style.
#[derive(Debug, Clone, PartialEq)]
pub struct MultilineStyle {
    pub name: String,
    pub spacing: f64,
}

/// The bounded style table plus the current style's index.
#[derive(Debug, Clone)]
pub struct MultilineStyleStore {
    styles: Vec<MultilineStyle>,
    current: usize,
}

impl Default for MultilineStyleStore {
    fn default() -> Self {
        Self {
            styles: vec![MultilineStyle {
                name: "STANDARD".into(),
                spacing: DEFAULT_MULTILINE_SPACING,
            }],
            current: 0,
        }
    }
}

impl MultilineStyleStore {
    pub fn styles(&self) -> &[MultilineStyle] {
        &self.styles
    }

    pub fn current(&self) -> &MultilineStyle {
        // `current` always indexes `styles`: every mutation keeps it in range.
        &self.styles[self.current]
    }

    /// The current style name (the `MLSTYLE LIST` marker).
    pub fn current_name(&self) -> &str {
        &self.current().name
    }

    /// The spacing `MLINE` uses when its payload supplies none.
    pub fn current_spacing(&self) -> f64 {
        self.current().spacing
    }

    /// Create or update one style and make it current.
    ///
    /// The name is trimmed; an existing name matches case-insensitively and
    /// keeps its stored spelling. Every refusal leaves the store untouched.
    pub fn set(&mut self, name: &str, spacing: f64) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_STYLE_NAME_CHARS
            || name.chars().any(|c| c == '\n' || c == '\r')
        {
            return Err(format!(
                "mlstyle.invalid_name: the style name must be a non-empty single line of at \
                 most {MAX_STYLE_NAME_CHARS} characters"
            ));
        }
        if !spacing.is_finite() || spacing <= 0.0 {
            return Err(format!(
                "mlstyle.invalid_spacing: spacing must be finite and > 0, got {spacing}"
            ));
        }
        if let Some(index) = self
            .styles
            .iter()
            .position(|style| style.name.eq_ignore_ascii_case(name))
        {
            self.styles[index].spacing = spacing;
            self.current = index;
            return Ok(());
        }
        if self.styles.len() >= MULTILINE_STYLE_CAP {
            return Err(format!(
                "mlstyle.capacity: the session already holds the maximum of \
                 {MULTILINE_STYLE_CAP} multiline styles"
            ));
        }
        self.styles.push(MultilineStyle {
            name: name.to_string(),
            spacing,
        });
        self.current = self.styles.len() - 1;
        Ok(())
    }
}
