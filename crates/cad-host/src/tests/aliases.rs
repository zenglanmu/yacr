//! Durable runtime-alias contracts: the root `commandAliases` object is loaded
//! before the strict preference projection, re-emitted on every write, and a
//! failed write rolls the in-memory set back.

use super::*;
use crate::aliases::{command_aliases_from_preference, COMMAND_ALIASES_PREFERENCE_KEY};
use std::cell::Cell;

struct SeededConfig {
    preference: String,
    persisted: RefCell<Vec<String>>,
    fail: Cell<bool>,
}

impl SeededConfig {
    fn new(preference: impl Into<String>) -> Rc<Self> {
        Rc::new(Self {
            preference: preference.into(),
            persisted: RefCell::new(Vec::new()),
            fail: Cell::new(false),
        })
    }

    fn last_persisted(&self) -> serde_json::Value {
        let text = self
            .persisted
            .borrow()
            .last()
            .cloned()
            .expect("a preference write happened");
        serde_json::from_str(&text).expect("the persisted preference is JSON")
    }
}

impl ConfigStore for SeededConfig {
    fn load_config(&self) -> ConfigLoad {
        ConfigLoad::Absent
    }

    fn load_preference(&self) -> ConfigLoad {
        ConfigLoad::Text {
            path: "/test/preference.json".into(),
            text: self.preference.clone(),
        }
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        if self.fail.get() {
            return Err(CadError::Unsupported("the disk is full".into()));
        }
        self.persisted.borrow_mut().push(text.to_string());
        Ok(())
    }
}

#[test]
fn the_alias_object_is_loaded_before_the_strict_projection_and_re_emitted() {
    let config = SeededConfig::new(format!(
        r#"{{"{COMMAND_ALIASES_PREFERENCE_KEY}":{{"QQ":"FIT"}},"ui":{{"recovery":{{"intervalSeconds":30}}}}}}"#
    ));
    let core = core_with_config(config.clone());
    core.load_startup();
    assert_eq!(
        core.command_aliases(),
        vec![("QQ".to_string(), "FIT".to_string())]
    );
    // The startup re-persist keeps the root leaf (the projection cannot carry
    // it) and applies the ordinary setting beside it.
    let persisted = config.last_persisted();
    assert_eq!(persisted["commandAliases"]["QQ"], "FIT");
    assert_eq!(persisted["ui"]["recovery"]["intervalSeconds"], 30);
}

#[test]
fn replace_persists_the_set_and_rolls_back_on_a_failed_write() {
    let config = SeededConfig::new("{}");
    let core = core_with_config(config.clone());
    core.load_startup();

    core.replace_command_aliases(&[("QQ".to_string(), "FIT".to_string())])
        .expect("the durable write succeeds");
    assert_eq!(
        config.last_persisted()["commandAliases"]["QQ"],
        "FIT",
        "the root object is written"
    );

    config.fail.set(true);
    let error = core
        .replace_command_aliases(&[
            ("QQ".to_string(), "FIT".to_string()),
            ("ZZ".to_string(), "ZOOM".to_string()),
        ])
        .expect_err("a failing store refuses the write");
    assert!(
        error.to_string().contains("settings.not_persisted"),
        "{error}"
    );
    assert_eq!(
        core.command_aliases(),
        vec![("QQ".to_string(), "FIT".to_string())],
        "the in-memory set is rolled back"
    );
}

#[test]
fn a_malformed_persisted_alias_object_is_untrusted_as_a_whole() {
    for text in [
        r#"{"commandAliases":{"QQ":7}}"#,
        r#"{"commandAliases":{"":"FIT"}}"#,
        r#"{"commandAliases":["QQ","FIT"]}"#,
        r#"{"commandAliases":{"QQ":" "}}"#,
    ] {
        assert!(
            command_aliases_from_preference(text).is_empty(),
            "a malformed row drops the whole set: {text}"
        );
    }
    assert_eq!(
        command_aliases_from_preference(r#"{"commandAliases":{"qq":"FIT"}}"#),
        vec![("QQ".to_string(), "FIT".to_string())],
        "the alias spelling is stored uppercased"
    );
}
