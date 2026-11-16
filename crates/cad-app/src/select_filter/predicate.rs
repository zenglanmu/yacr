//! The predicate and expression evaluation of the select/filter batch: the
//! [`SelectionQuery`] wrapper, the FILTER logical-tree walk, and the
//! property/predicate comparison (including the anchored `*`/`?` wildcard
//! match).
//!
//! Split out of `select_filter.rs` to keep that file inside the code-size limit.
//! Nothing here is public: [`SelectionQuery`] is the parent module's input type
//! and the `SelectionProperty`/`SelectionPredicate` helpers only evaluate.

use crate::command_payload::{
    FilterExpression, FilterLogic, SelectionOperator, SelectionPredicate, SelectionProperty,
};
use crate::property_value::property_value;
use cad_db::{DbEntity, DrawingDatabase};
use cad_domain::{SemanticGeometry, SpaceId};

use super::types::{type_matches, type_names};

/// One selection query: either one predicate (QSELECT's one-row form) or a
/// FILTER expression tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SelectionQuery {
    Predicate {
        object_type: Option<String>,
        predicate: SelectionPredicate,
    },
    Expression(FilterExpression),
}

impl SelectionQuery {
    pub(super) fn matches(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        match self {
            SelectionQuery::Predicate {
                object_type,
                predicate,
            } => {
                if let Some(object_type) = object_type {
                    if !type_matches(entity, object_type) {
                        return false;
                    }
                }
                predicate.matches(database, entity)
            }
            SelectionQuery::Expression(expression) => {
                expression_matches(expression, database, entity)
            }
        }
    }
}

fn expression_matches(
    expression: &FilterExpression,
    database: &DrawingDatabase,
    entity: &DbEntity,
) -> bool {
    match expression {
        FilterExpression::Predicate(predicate) => predicate.matches(database, entity),
        FilterExpression::Group { logic, members } => match logic {
            FilterLogic::And => members
                .iter()
                .all(|member| expression_matches(member, database, entity)),
            FilterLogic::Or => members
                .iter()
                .any(|member| expression_matches(member, database, entity)),
            // Arity is validated before matching; the `get` calls stay defensive.
            FilterLogic::Xor => match (members.first(), members.get(1)) {
                (Some(first), Some(second)) => {
                    expression_matches(first, database, entity)
                        != expression_matches(second, database, entity)
                }
                _ => false,
            },
            FilterLogic::Not => members
                .first()
                .is_some_and(|member| !expression_matches(member, database, entity)),
        },
    }
}

impl SelectionProperty {
    fn string_value(self, database: &DrawingDatabase, entity: &DbEntity) -> Option<String> {
        match self {
            Self::Type => Some(entity.object.type_key.clone()),
            Self::Layer => database.layer(entity.layer).map(|layer| layer.name.clone()),
            Self::Color | Self::Linetype | Self::Lineweight | Self::Transparency => {
                property_value(database, entity, key(self))
            }
            Self::LinetypeScale => Some(format!(
                "{}",
                cad_db::sanitize_linetype_scale(
                    database.entity_render_attributes(entity.id).linetype_scale
                )
            )),
            Self::Space => Some(space_name(database, &entity.space)),
            Self::Text => match &entity.geometry {
                SemanticGeometry::Text { text, .. } => Some(text.clone()),
                SemanticGeometry::MultiLeader(leader) => Some(leader.text().string.clone()),
                // A table's searchable text is every non-empty cell, joined by
                // one space (the same string FIND searches).
                SemanticGeometry::Table(table) => {
                    let texts = table.texts();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(texts.join(" "))
                    }
                }
                _ => None,
            },
            Self::Name => match &entity.geometry {
                SemanticGeometry::Insert { block, .. } => {
                    database.block_name(*block).map(str::to_string)
                }
                _ => None,
            },
        }
    }

    fn numeric_value(self, database: &DrawingDatabase, entity: &DbEntity) -> Option<f64> {
        match self {
            Self::Lineweight | Self::Transparency => self
                .string_value(database, entity)
                .and_then(|value| value.trim().parse::<f64>().ok()),
            Self::LinetypeScale => Some(cad_db::sanitize_linetype_scale(
                database.entity_render_attributes(entity.id).linetype_scale,
            )),
            _ => None,
        }
    }

    /// The property key `cad_app::property_value` uses, or `None`.
    fn property_key(self) -> Option<&'static str> {
        match self {
            Self::Color => Some("color"),
            Self::Linetype => Some("linetype"),
            Self::Lineweight => Some("lineweight"),
            Self::Transparency => Some("transparency"),
            _ => None,
        }
    }
}

fn key(property: SelectionProperty) -> &'static str {
    property.property_key().unwrap_or("")
}

impl SelectionPredicate {
    fn matches(&self, database: &DrawingDatabase, entity: &DbEntity) -> bool {
        if self.operator == SelectionOperator::SelectAll {
            return true;
        }
        if self.property == SelectionProperty::Type {
            return self.matches_type(entity);
        }
        let Some(candidate) = self.property.string_value(database, entity) else {
            return false;
        };
        self.compare(database, entity, &candidate)
    }

    fn matches_type(&self, entity: &DbEntity) -> bool {
        match self.operator {
            SelectionOperator::Equals => type_matches(entity, &self.value),
            SelectionOperator::NotEqual => !type_matches(entity, &self.value),
            SelectionOperator::WildcardMatch => type_names(entity)
                .iter()
                .any(|name| wildcard_match(&self.value, name)),
            // Relative operators are refused for the type property.
            _ => false,
        }
    }

    fn compare(&self, database: &DrawingDatabase, entity: &DbEntity, candidate: &str) -> bool {
        match self.operator {
            SelectionOperator::Equals | SelectionOperator::NotEqual => {
                let equal = if self.property.is_numeric() {
                    match (
                        self.property.numeric_value(database, entity),
                        self.value.trim().parse::<f64>(),
                    ) {
                        (Some(left), Ok(right)) => (left - right).abs() <= f64::EPSILON * 16.0,
                        _ => candidate.eq_ignore_ascii_case(&self.value),
                    }
                } else {
                    candidate.eq_ignore_ascii_case(&self.value)
                };
                if self.operator == SelectionOperator::Equals {
                    equal
                } else {
                    !equal
                }
            }
            SelectionOperator::WildcardMatch => wildcard_match(&self.value, candidate),
            SelectionOperator::SelectAll => true,
            relative => {
                let Ok(rhs) = self.value.trim().parse::<f64>() else {
                    return false;
                };
                let Some(lhs) = self.property.numeric_value(database, entity) else {
                    return false;
                };
                match relative {
                    SelectionOperator::GreaterThan => lhs > rhs,
                    SelectionOperator::LessThan => lhs < rhs,
                    SelectionOperator::GreaterThanOrEqual => lhs >= rhs,
                    SelectionOperator::LessThanOrEqual => lhs <= rhs,
                    _ => false,
                }
            }
        }
    }
}

fn space_name(database: &DrawingDatabase, space: &SpaceId) -> String {
    match space {
        SpaceId::Model => "Model".to_string(),
        SpaceId::Paper(layout) => database
            .layout(*layout)
            .map(|layout| layout.name.clone())
            .unwrap_or_else(|| "Paper".to_string()),
        SpaceId::Block(block) => format!("Block:{}", block.0),
    }
}

/// Anchored, ASCII case-insensitive `*`/`?` wildcard match.
///
/// This is the `*Wildcard Match*` operator's core grammar. The other AutoCAD
/// wildcard tokens (`#`, `@`, `.`, `[...]`, `~`) are **not** implemented and are
/// literal characters here; that boundary is documented rather than silently
/// guessed.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // Iterative backtracking: `star` remembers the last `*` and `mark` the text
    // position it may still extend.
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(star_index) = star {
            pi = star_index + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}
