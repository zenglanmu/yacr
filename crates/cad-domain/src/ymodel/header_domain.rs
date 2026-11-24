//! The `application_domain` scalar of `document.json` (ADR-0053 D2).
//!
//! Split out of `header.rs` (at the file-size gate) with the rules unchanged:
//! the key is written only when the document names a domain, and a present value
//! this build does not model is refused as a malformed field rather than being
//! normalized to the drafting profile.

use serde_json::Value;

use super::header::HeaderError;
use super::json::ObjectReader;
use crate::ApplicationDomain;

/// The JSON value for a named domain, or `None` when nothing is written.
pub(super) fn encode(domain: Option<ApplicationDomain>) -> Option<Value> {
    domain.map(|value| Value::String(value.as_str().to_string()))
}

/// Read `application_domain` from an open document-header reader.
pub(super) fn decode(reader: &mut ObjectReader) -> Result<Option<ApplicationDomain>, HeaderError> {
    match reader.raw_if_present("application_domain") {
        None => Ok(None),
        Some(Value::String(token)) => {
            ApplicationDomain::from_token(&token)
                .map(Some)
                .ok_or(HeaderError::InvalidField {
                    field: "application_domain",
                    detail: format!("unknown domain token {token:?}"),
                })
        }
        Some(_) => Err(HeaderError::InvalidField {
            field: "application_domain",
            detail: "expected a string".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_value(value: Value) -> Result<Option<ApplicationDomain>, HeaderError> {
        let mut reader = ObjectReader::new(&value, "document").expect("an object");
        decode(&mut reader)
    }

    #[test]
    fn absent_stays_unwritten_and_known_tokens_round_trip() {
        assert!(encode(None).is_none());
        for domain in ApplicationDomain::ALL {
            let value = encode(Some(domain)).expect("written");
            assert_eq!(value, Value::String(domain.as_str().to_string()));
            assert_eq!(
                decode_value(serde_json::json!({ "application_domain": value })).expect("read"),
                Some(domain)
            );
        }
        assert_eq!(decode_value(serde_json::json!({})).expect("absent"), None);
    }

    #[test]
    fn an_unknown_or_non_string_token_is_refused() {
        for value in [
            serde_json::json!({"application_domain": "revit"}),
            serde_json::json!({"application_domain": true}),
        ] {
            assert!(matches!(
                decode_value(value),
                Err(HeaderError::InvalidField {
                    field: "application_domain",
                    ..
                })
            ));
        }
    }
}
