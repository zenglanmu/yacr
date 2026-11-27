//! Persisted scenes (`tables/scenes.json`, `.ymodel` minor 20; a row with the
//! `tags` snapshot makes the writer stamp minor 23, a row with `display` minor
//! 24).
//!
//! A scene is SketchUp's page: a named snapshot of **presentation state** that a
//! click restores. This cut captures exactly the state this build can restore
//! and render:
//!
//! * the camera, 2D/3D mode, saved 2D camera, work plane and active space — the
//!   same payload a named view stores ([`crate::NamedView`]);
//! * the **visual style** token (`WIREFRAME`/`SHADED`), the one real display
//!   style this build implements (`VSCURRENT`);
//! * the **visible tags/layers** at save time (name + effective visibility,
//!   applied on restore as session overrides; the layer table itself is never
//!   rewritten by a scene);
//! * the document **Shadows/Fog record** (`tables/display.json`), applied on
//!   restore as the same one-revision display write the `SHADOWS`/`FOG`
//!   commands use when the saved record differs from the current one.
//!
//! It deliberately does **not** capture sections or soft edges: those have no
//! scene application path yet, and storing a flag the renderer ignores would be
//! a lie. The scene row is a named view plus the style key and optional tag and
//! display snapshots, so it reuses the view's strict codec and camera
//! validation. A row whose `tags` key is present makes the writer stamp the
//! archive at minor 23, a row with `display` at minor 24; a row without either
//! stays a minor-20 row and restores only the view/style.
//!
//! # Bounds and names
//!
//! [`SCENE_CAP`] = **32** rows. Saving a 33rd is refused by name rather than
//! evicting one; names are trimmed, non-empty, at most
//! [`crate::MAX_NAMED_VIEW_NAME_CHARS`] characters, unique case-insensitively
//! (exact match first, then a unique case-insensitive one), and saving an
//! existing name replaces it and moves it last. A scene name and a view name are
//! independent namespaces.

use std::fmt;

use serde_json::{Map, Value};

use crate::display::DisplaySettings;
use crate::view::{NamedView, NamedViewError};

/// How many scenes one document retains.
pub const SCENE_CAP: usize = 32;

/// How many tag rows one scene may capture.
pub const SCENE_TAG_CAP: usize = 1024;

/// The longest captured tag name (layer names are short in practice).
pub const SCENE_TAG_NAME_MAX_CHARS: usize = 255;

/// How many named views one document retains (re-exported for scene caps/tests).
pub use crate::view::NAMED_VIEW_CAP;

/// Stable refusal markers.
pub mod markers {
    /// The scene table holds more rows than [`super::SCENE_CAP`].
    pub const SCENE_TABLE_TOO_LARGE: &str = "scene.table_too_large";
    /// Saving a new name would exceed [`super::SCENE_CAP`].
    pub const SCENE_CAP_EXCEEDED: &str = "scene.cap_exceeded";
    /// Two rows share a name (case-insensitively).
    pub const SCENE_DUPLICATE_NAME: &str = "scene.duplicate_name";
    /// There is no scene of that name.
    pub const SCENE_UNKNOWN: &str = "scene.unknown";
    /// The style token is not one this build implements.
    pub const SCENE_STYLE_INVALID: &str = "scene.style_invalid";
    /// The row's view half is malformed (delegated to the view codec).
    pub const SCENE_INVALID_VIEW: &str = "scene.invalid_view";
    /// The captured tag snapshot is malformed or over budget.
    pub const SCENE_TAGS_INVALID: &str = "scene.tags_invalid";
    /// The captured Shadows/Fog record is malformed or out of range.
    pub const SCENE_DISPLAY_INVALID: &str = "scene.display_invalid";
}

/// The visual style a scene restores.
///
/// The two values this build implements, spelled exactly like the app's
/// `VisualStyleToken`; another style is never silently mapped onto one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SceneVisualStyle {
    /// 3D body wireframe: edges only.
    Wireframe,
    /// 3D body shaded display: filled mesh plus stored edges (the default).
    #[default]
    Shaded,
}

impl SceneVisualStyle {
    /// The stable token spelling stored in the archive.
    pub fn token(self) -> &'static str {
        match self {
            Self::Wireframe => "WIREFRAME",
            Self::Shaded => "SHADED",
        }
    }

    /// Decode a stored token (exact spelling; the writer emits uppercase).
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "WIREFRAME" => Some(Self::Wireframe),
            "SHADED" => Some(Self::Shaded),
            _ => None,
        }
    }
}

/// One scene-table row failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    /// A stable marker plus a detail, ready for a `CadError`/snapshot error.
    Invalid(&'static str, String),
}

impl SceneError {
    /// The stable marker.
    pub fn marker(&self) -> &'static str {
        match self {
            SceneError::Invalid(marker, _) => marker,
        }
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneError::Invalid(marker, detail) => write!(formatter, "{marker}: {detail}"),
        }
    }
}

impl std::error::Error for SceneError {}

impl From<NamedViewError> for SceneError {
    fn from(error: NamedViewError) -> Self {
        SceneError::Invalid(markers::SCENE_INVALID_VIEW, error.to_string())
    }
}

/// One tag/layer visibility captured with a scene.
///
/// The name is the layer's own name (the stable user-facing identity); restore
/// resolves it against the **current** layer table, so a renamed or deleted
/// layer is simply not matched and nothing is fabricated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneTag {
    /// Layer name as stored in the layer table.
    pub name: String,
    /// Effective visibility at save time (database flag plus the session override).
    pub visible: bool,
}

/// One persisted scene: a named view plus the display state it restores.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The camera/space half (its `name` is the scene name).
    pub view: NamedView,
    /// The display style restored with the view.
    pub style: SceneVisualStyle,
    /// The visible tags/layers captured at save time, or `None` for a scene
    /// written by a build without tag capture (minor 20..22).
    pub tags: Option<Vec<SceneTag>>,
    /// The document Shadows/Fog record at save time, or `None` for a scene
    /// written before scene display capture (minor 20..23).
    pub display: Option<DisplaySettings>,
}

impl Scene {
    /// A scene named `name` over `view` (the view's own name is overwritten).
    pub fn named(name: impl Into<String>, view: NamedView) -> Self {
        Scene {
            view: NamedView {
                name: name.into(),
                ..view
            },
            style: SceneVisualStyle::default(),
            tags: None,
            display: None,
        }
    }

    /// The stored name.
    pub fn name(&self) -> &str {
        &self.view.name
    }

    /// Whether `name` resolves to this row (case-insensitive, trimmed).
    pub fn has_name(&self, name: &str) -> bool {
        self.view.name.eq_ignore_ascii_case(name.trim())
    }

    /// The row's single validation point (delegates the camera/space half and
    /// checks the tag snapshot).
    pub fn validate(&self) -> Result<(), SceneError> {
        self.view
            .validate()
            .map_err(|error| SceneError::Invalid(markers::SCENE_INVALID_VIEW, error.to_string()))?;
        if let Some(display) = &self.display {
            display.validate().map_err(|error| {
                SceneError::Invalid(markers::SCENE_DISPLAY_INVALID, error.to_string())
            })?;
        }
        if let Some(tags) = &self.tags {
            if tags.len() > SCENE_TAG_CAP {
                return Err(SceneError::Invalid(
                    markers::SCENE_TAGS_INVALID,
                    format!(
                        "tags: {} rows, over the {SCENE_TAG_CAP}-row budget",
                        tags.len()
                    ),
                ));
            }
            let mut seen = Vec::with_capacity(tags.len());
            for tag in tags {
                let name = tag.name.trim();
                if name.is_empty() {
                    return Err(SceneError::Invalid(
                        markers::SCENE_TAGS_INVALID,
                        "tags: a captured tag needs a non-empty name".into(),
                    ));
                }
                if name.chars().count() > SCENE_TAG_NAME_MAX_CHARS {
                    return Err(SceneError::Invalid(
                        markers::SCENE_TAGS_INVALID,
                        format!(
                            "tags: `{name}` is longer than {SCENE_TAG_NAME_MAX_CHARS} characters"
                        ),
                    ));
                }
                if tag.name != name {
                    return Err(SceneError::Invalid(
                        markers::SCENE_TAGS_INVALID,
                        format!("tags: `{}` has untrimmed whitespace", tag.name),
                    ));
                }
                let key = name.to_ascii_lowercase();
                if seen.contains(&key) {
                    return Err(SceneError::Invalid(
                        markers::SCENE_TAGS_INVALID,
                        format!("tags: duplicate tag name `{name}`"),
                    ));
                }
                seen.push(key);
            }
        }
        Ok(())
    }

    /// Encode one row: the named-view object plus a `style` key.
    pub fn to_json(&self) -> Value {
        let Value::Object(mut row) = self.view.to_json() else {
            // `NamedView::to_json` is an object by construction; this arm is
            // unreachable and exists so the codec never fabricates a shape.
            let mut row = Map::new();
            row.insert(
                "note".to_string(),
                Value::String("scene.encode: named view is not an object".into()),
            );
            return Value::Object(row);
        };
        row.insert(
            "style".to_string(),
            Value::String(self.style.token().into()),
        );
        if let Some(tags) = &self.tags {
            let rows = tags
                .iter()
                .map(|tag| {
                    let mut row = Map::new();
                    row.insert("name".to_string(), Value::String(tag.name.clone()));
                    row.insert("visible".to_string(), Value::Bool(tag.visible));
                    Value::Object(row)
                })
                .collect();
            row.insert("tags".to_string(), Value::Array(rows));
        }
        if let Some(display) = &self.display {
            row.insert("display".to_string(), display.to_json());
        }
        Value::Object(row)
    }

    /// Decode one row strictly: `style` is lifted out, then the named-view codec
    /// (which refuses every other unknown field) decodes the rest.
    pub fn from_json(value: &Value) -> Result<Self, SceneError> {
        let Value::Object(map) = value else {
            return Err(SceneError::Invalid(
                markers::SCENE_INVALID_VIEW,
                "row: expected a JSON object".into(),
            ));
        };
        let mut map = map.clone();
        let style = match map.remove("style") {
            Some(Value::String(token)) => {
                SceneVisualStyle::from_token(&token).ok_or_else(|| {
                    SceneError::Invalid(
                        markers::SCENE_STYLE_INVALID,
                        format!("style: {token:?} is not WIREFRAME or SHADED"),
                    )
                })?
            }
            Some(_) => {
                return Err(SceneError::Invalid(
                    markers::SCENE_STYLE_INVALID,
                    "style: expected a string".into(),
                ))
            }
            None => {
                return Err(SceneError::Invalid(
                    markers::SCENE_STYLE_INVALID,
                    "style: a scene row needs a style token".into(),
                ))
            }
        };
        let tags = match map.remove("tags") {
            Some(Value::Array(rows)) => {
                let mut tags = Vec::with_capacity(rows.len());
                for (index, row) in rows.iter().enumerate() {
                    let Value::Object(row) = row else {
                        return Err(SceneError::Invalid(
                            markers::SCENE_TAGS_INVALID,
                            format!("tags[{index}]: expected an object"),
                        ));
                    };
                    let name = match row.get("name") {
                        Some(Value::String(name)) => name.trim().to_string(),
                        _ => {
                            return Err(SceneError::Invalid(
                                markers::SCENE_TAGS_INVALID,
                                format!("tags[{index}].name: expected a string"),
                            ))
                        }
                    };
                    let visible = match row.get("visible") {
                        Some(Value::Bool(visible)) => *visible,
                        _ => {
                            return Err(SceneError::Invalid(
                                markers::SCENE_TAGS_INVALID,
                                format!("tags[{index}].visible: expected a boolean"),
                            ))
                        }
                    };
                    if row.len() != 2 {
                        return Err(SceneError::Invalid(
                            markers::SCENE_TAGS_INVALID,
                            format!("tags[{index}]: unknown key"),
                        ));
                    }
                    tags.push(SceneTag { name, visible });
                }
                Some(tags)
            }
            Some(_) => {
                return Err(SceneError::Invalid(
                    markers::SCENE_TAGS_INVALID,
                    "tags: expected an array".into(),
                ))
            }
            None => None,
        };
        let display = match map.remove("display") {
            Some(value) => Some(DisplaySettings::from_json(&value).map_err(|error| {
                SceneError::Invalid(markers::SCENE_DISPLAY_INVALID, error.to_string())
            })?),
            None => None,
        };
        let view = NamedView::from_json(&Value::Object(map))?;
        let scene = Scene {
            view,
            style,
            tags,
            display,
        };
        scene.validate()?;
        Ok(scene)
    }
}

/// A bounded scene table, oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneTable {
    scenes: Vec<Scene>,
}

impl SceneTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many scenes are retained.
    pub fn len(&self) -> usize {
        self.scenes.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    /// The retained scenes in save order, oldest first.
    pub fn scenes(&self) -> impl Iterator<Item = &Scene> + '_ {
        self.scenes.iter()
    }

    /// Resolve a name: exact first, then a unique case-insensitive match.
    pub fn resolve(&self, name: &str) -> Option<&Scene> {
        let name = name.trim();
        if let Some(found) = self.scenes.iter().find(|scene| scene.name() == name) {
            return Some(found);
        }
        let mut matches = self.scenes.iter().filter(|scene| scene.has_name(name));
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// Save (or replace, case-insensitively) one scene and return whether an
    /// existing row was replaced. A replacement moves to the newest position.
    pub fn save(&mut self, scene: Scene) -> Result<bool, SceneError> {
        scene.validate()?;
        let name = scene.view.name.trim().to_string();
        if let Some(index) = self
            .scenes
            .iter()
            .position(|candidate| candidate.has_name(&name))
        {
            self.scenes.remove(index);
            self.scenes.push(Scene {
                view: NamedView { name, ..scene.view },
                style: scene.style,
                tags: scene.tags,
                display: scene.display,
            });
            return Ok(true);
        }
        if self.scenes.len() >= SCENE_CAP {
            return Err(SceneError::Invalid(
                markers::SCENE_CAP_EXCEEDED,
                format!(
                    "`{name}` would be scene {}, but the table holds at most {SCENE_CAP} rows",
                    self.scenes.len() + 1
                ),
            ));
        }
        self.scenes.push(Scene {
            view: NamedView { name, ..scene.view },
            style: scene.style,
            tags: scene.tags,
            display: scene.display,
        });
        Ok(false)
    }

    /// Remove one row, returning it when it existed.
    pub fn remove(&mut self, name: &str) -> Option<Scene> {
        let index = self.scenes.iter().position(|scene| scene.has_name(name))?;
        Some(self.scenes.remove(index))
    }

    /// Encode every row, oldest first.
    pub fn to_json(&self) -> Vec<Value> {
        self.scenes.iter().map(Scene::to_json).collect()
    }

    /// Decode a row table strictly (duplicates, cap and every row validated).
    pub fn from_json(rows: &[Value]) -> Result<Self, SceneError> {
        if rows.len() > SCENE_CAP {
            return Err(SceneError::Invalid(
                markers::SCENE_TABLE_TOO_LARGE,
                format!(
                    "tables/scenes.json holds {} rows, over the {SCENE_CAP}-row budget",
                    rows.len()
                ),
            ));
        }
        let mut scenes = Vec::with_capacity(rows.len());
        let mut names: Vec<String> = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let scene = Scene::from_json(row).map_err(|error| match error {
                SceneError::Invalid(marker, detail) => {
                    SceneError::Invalid(marker, format!("row {index}: {detail}"))
                }
            })?;
            let name = scene.view.name.to_ascii_lowercase();
            if names.contains(&name) {
                return Err(SceneError::Invalid(
                    markers::SCENE_DUPLICATE_NAME,
                    format!("row {index}: duplicate scene name {:?}", scene.view.name),
                ));
            }
            names.push(name);
            scenes.push(scene);
        }
        Ok(SceneTable { scenes })
    }
}
