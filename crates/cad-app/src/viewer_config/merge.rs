use super::*;

fn join_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// The only map-typed field in the protocol; new keys are legal here.
fn is_map_path(path: &str) -> bool {
    path == "ui.commandOverrides"
}

/// Capabilities and component visibility a user preference may never re-enable.
fn is_clamped_path(path: &str) -> bool {
    if path == "features" || path.starts_with("features.") {
        return true;
    }
    // A command-override visibility leaf is capability-bearing too: a user may
    // hide a command but must never re-enable one a preset/feature disabled.
    if path == "ui.commandOverrides" || path.starts_with("ui.commandOverrides.") {
        return true;
    }
    // `ui.components.<name>.visible`
    let parts: Vec<&str> = path.split('.').collect();
    parts.len() == 4 && parts[0] == "ui" && parts[1] == "components" && parts[3] == "visible"
}

fn path_allowed(path: &str, allowed: &[String]) -> bool {
    allowed
        .iter()
        .any(|a| path == a || path.starts_with(&format!("{a}.")))
}

fn path_or_descendant_allowed(child: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|a| {
        child == a || child.starts_with(&format!("{a}.")) || a.starts_with(&format!("{child}."))
    })
}

/// Project a preference patch onto the currently-allowed paths. Used by hosts
/// to persist only what a later load may legitimately re-apply.
pub(super) fn project_allowed(patch: &Value, path: &str, allowed: &[String]) -> Value {
    match patch {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                let child = join_path(path, key.as_str());
                let projected = project_allowed(value, &child, allowed);
                let keep = if projected.is_object() {
                    projected
                        .as_object()
                        .map(|m| !m.is_empty())
                        .unwrap_or(false)
                } else {
                    path_allowed(&child, allowed)
                };
                if keep {
                    out.insert(key.clone(), projected);
                }
            }
            Value::Object(out)
        }
        leaf => leaf.clone(),
    }
}

/// Reject `null` anywhere in a value tree (config has no null sentinel).
fn ensure_no_null(value: &Value, path: &str) -> Result<(), ConfigError> {
    match value {
        Value::Null => Err(ConfigError::new(path, "null is not a configuration value")),
        Value::Object(map) => {
            for (key, child) in map {
                ensure_no_null(child, &join_path(path, key))?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                ensure_no_null(child, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn type_error(path: &str) -> ConfigError {
    ConfigError::new(path, "configuration value has wrong type")
}

/// Deep-merge a preference patch, creating missing object nodes.
///
/// Unlike the schema-aware [`merge`], the preference accumulator has no fixed
/// shape: it only needs to remember what the host allowed. Arrays replace, null
/// is rejected; a type clash between the existing preference and the patch is
/// rejected rather than silently overwritten.
pub(super) fn deep_merge_create(target: &mut Value, patch: &Value) -> Result<(), ConfigError> {
    match (&mut *target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                match target.get_mut(key) {
                    Some(slot) => deep_merge_create(slot, value)?,
                    None => {
                        ensure_no_null(value, key)?;
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
            Ok(())
        }
        (_, Value::Null) => Err(ConfigError::new(
            "preference",
            "null is not a configuration value",
        )),
        (slot, leaf) => {
            ensure_no_null(leaf, "")?;
            let valid = matches!(
                (&*slot, leaf),
                (Value::Bool(_), Value::Bool(_))
                    | (Value::String(_), Value::String(_))
                    | (Value::Number(_), Value::Number(_))
                    | (Value::Array(_), Value::Array(_))
                    | (Value::Object(_), Value::Object(_))
            );
            if !valid {
                return Err(type_error("preference"));
            }
            *slot = leaf.clone();
            Ok(())
        }
    }
}

/// Structured merge for host updates: objects merge key-wise, arrays replace,
/// explicit `false` survives and `null` is rejected. New keys are only legal
/// inside the command-override map; everything else must exist already.
pub(super) fn merge(target: &mut Value, patch: Value, path: &str) -> Result<(), ConfigError> {
    if let Value::Object(patch) = patch {
        let target = target
            .as_object_mut()
            .ok_or_else(|| ConfigError::new(path, "expected object"))?;
        for (key, value) in patch {
            let child = join_path(path, key.as_str());
            match target.get_mut(&key) {
                Some(slot) => merge(slot, value, &child)?,
                None if is_map_path(path) => {
                    ensure_no_null(&value, &child)?;
                    if !value.is_object() {
                        return Err(type_error(&child));
                    }
                    target.insert(key, value);
                }
                None => {
                    return Err(ConfigError::new(child, "unsupported configuration field"));
                }
            }
        }
        Ok(())
    } else {
        if patch.is_null() {
            return Err(ConfigError::new(path, "null is not a configuration value"));
        }
        let valid = matches!(
            (&*target, &patch),
            (Value::Bool(_), Value::Bool(_))
                | (Value::String(_), Value::String(_))
                | (Value::Number(_), Value::Number(_))
                | (Value::Array(_), Value::Array(_))
                | (Value::Object(_), Value::Object(_))
        );
        if !valid {
            return Err(type_error(path));
        }
        *target = patch;
        Ok(())
    }
}

/// Apply a host-allowed user preference onto a config value derived from the
/// host value. `strict` rejects disallowed paths (an explicit API call); the
/// lenient mode used when re-applying after `set_config` skips paths the host
/// has since revoked.
pub(super) fn apply_preference(
    target: &mut Value,
    patch: &Value,
    path: &str,
    allowed: &[String],
    strict: bool,
) -> Result<(), ConfigError> {
    match patch {
        Value::Object(map) => {
            let target = target
                .as_object_mut()
                .ok_or_else(|| ConfigError::new(path, "expected object"))?;
            for (key, value) in map {
                let child = join_path(path, key.as_str());
                if let Some(slot) = target.get_mut(key) {
                    apply_preference(slot, value, &child, allowed, strict)?;
                } else if is_map_path(path) {
                    if path_or_descendant_allowed(&child, allowed) {
                        ensure_no_null(value, &child)?;
                        if !value.is_object() {
                            return Err(type_error(&child));
                        }
                        // A new command-override entry is capability-bearing: a
                        // user preference may create it but must never set its
                        // `visible` to true when the host has not enabled it.
                        let value = if let Value::Object(mut entry) = value.clone() {
                            if entry.contains_key("visible") {
                                entry.insert("visible".into(), Value::Bool(false));
                            }
                            Value::Object(entry)
                        } else {
                            value.clone()
                        };
                        target.insert(key.clone(), value);
                    } else if strict {
                        return Err(ConfigError::new(
                            child,
                            "user preference path is not allowed by host customization",
                        ));
                    }
                } else if strict {
                    return Err(ConfigError::new(child, "unsupported configuration field"));
                }
            }
            Ok(())
        }
        Value::Null => Err(ConfigError::new(path, "null is not a configuration value")),
        leaf => {
            if !path_allowed(path, allowed) {
                if strict {
                    return Err(ConfigError::new(
                        path,
                        "user preference path is not allowed by host customization",
                    ));
                }
                return Ok(());
            }
            // A nullable schema leaf (`interaction.snapEnabled`, an
            // `Option<bool>`) serializes as `null` when the host set no value;
            // a null slot means "absent", so an allowed preference may fill it.
            // Every other target/leaf pairing must keep its type.
            let type_ok = target.is_null()
                || matches!(
                    (&*target, leaf),
                    (Value::Bool(_), Value::Bool(_))
                        | (Value::String(_), Value::String(_))
                        | (Value::Number(_), Value::Number(_))
                        | (Value::Array(_), Value::Array(_))
                        | (Value::Object(_), Value::Object(_))
                );
            if !type_ok {
                return Err(type_error(path));
            }
            if is_clamped_path(path) {
                if let (Value::Bool(host), Value::Bool(pref)) = (&*target, leaf) {
                    // Clamp, never error: a user may turn a capability off, but
                    // may not switch on one the host/preset disabled.
                    *target = Value::Bool(*host && *pref);
                    return Ok(());
                }
            }
            *target = leaf.clone();
            Ok(())
        }
    }
}

/// Resolve a dotted path inside a serialized config value. Numeric segments
/// index arrays. Returns `None` when the path does not exist.
fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    if path.is_empty() {
        return Some(current);
    }
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Validate one host-allowed path against the schema. A command-override path
/// names a whitelisted command id (which itself may contain dots).
pub(super) fn allowed_path_is_known(config: &Value, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if let Some(rest) = path.strip_prefix("ui.commandOverrides.") {
        return COMMAND_IDS
            .iter()
            .any(|id| rest == *id || rest.starts_with(&format!("{id}.")));
    }
    lookup_path(config, path).is_some()
}
