//! `tables/parameters.json` row codec and the element `params` block
//! (ADR-0043 D4/D7).
//!
//! One table row per parameter, in `seq` order:
//!
//! ```json
//! [{"seq":7,"name":"板厚","kind":"length","unit":"mm","expr":{"num":12.5}},
//!  {"seq":8,"name":"孔径","kind":"length","expr":{"op":"div","lhs":{"ref":7},"rhs":{"num":2}}}]
//! ```
//!
//! Every expression is exactly one tagged form:
//!
//! ```json
//! {"num": <finite f64>}
//! {"ref": <u64>}
//! {"const": "pi"}
//! {"op": "add|sub|mul|div", "lhs": <expr>, "rhs": <expr>}
//! ```
//!
//! Strict in both directions, like the element envelope:
//!
//! * every required key is present, an extra key is refused, and a wrong JSON
//!   type is refused;
//! * `unit` is **optional**: absent means `None`, while `"unit": null` is
//!   refused (this writer never emits a null placeholder);
//! * an expression carrying two tags, no tag or an unknown tag word is refused;
//!   the domain constructors ([`cad_domain::param::ParamRow::new`],
//!   [`cad_domain::param::ParamBindings::insert`]) enforce the depth/node
//!   budgets and finite literals;
//! * duplicate `seq`/`name` rows are refused by [`cad_domain::param::ParamTable::insert`],
//!   and dangling references / cycles by
//!   [`cad_domain::param::evaluate_table`];
//! * the row count and the encoded table are both budgeted **before** an
//!   over-budget archive can be accepted: refusal, never truncation.
//!
//! The element `params` block is a flat slot → expression object written only
//! when the element has bindings:
//!
//! ```json
//! {"radius":{"ref":8},"height":{"num":20}}
//! ```
//!
//! Decoding does **not** consult the slot registry: whether a slot is legal for
//! the element's type is the builder's [`cad_domain::param::ParamBindings::resolve`]
//! gate, so the codec stays a shape layer and the database stays the authority.

use cad_domain::param::{
    BinaryOp, Expr, ParamBindings, ParamConst, ParamError, ParamKind, ParamRow, ParamSeq,
    ParamTable, ParamUnit, PARAM_BUDGET_EXCEEDED, PARAM_INVALID_VALUE, PARAM_MAX_ROWS,
};
use cad_domain::ymodel::json;
use serde_json::{Map, Value};

use super::bim::object;
use super::elements::Reader;
use super::{table_entry, SnapshotError, TableSnapshot};
use crate::drawing::{DrawingDatabase, PARAM_MAX_TABLE_BYTES};
use crate::scope::TableKind;

/// The table entry this codec owns.
fn table() -> &'static str {
    table_entry(TableKind::Parameters)
}

/// An over-budget table: `ymodel.invalid_document` whose detail carries the
/// parameter budget marker (not the BIM one — this table has its own vocabulary).
fn budget(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("{PARAM_BUDGET_EXCEEDED}: {detail}"))
}

/// A structurally malformed row/expression (the parameter vocabulary, not BIM's).
fn invalid(detail: impl std::fmt::Display) -> SnapshotError {
    SnapshotError::invalid(format!("{PARAM_INVALID_VALUE}: {detail}"))
}

/// Encode every parameter row, in `seq` order.
///
/// The encoded table is budget-checked here too, so a writer refuses an
/// over-budget document before it can save one.
pub(crate) fn encode_parameters(db: &DrawingDatabase) -> Result<Vec<Value>, SnapshotError> {
    let rows: Vec<Value> = db.parameters().rows().map(encode_row).collect();
    if rows.len() > PARAM_MAX_ROWS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {PARAM_MAX_ROWS}-row budget",
            table(),
            rows.len()
        )));
    }
    let encoded = encoded_len(&rows)?;
    if encoded > PARAM_MAX_TABLE_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the \
             {PARAM_MAX_TABLE_BYTES}-byte budget",
            table()
        )));
    }
    Ok(rows)
}

/// Decode the parameter table. An absent table is an empty document table.
pub(crate) fn decode_parameters(tables: &TableSnapshot) -> Result<ParamTable, SnapshotError> {
    let rows = tables.get(TableKind::Parameters).unwrap_or(&[]);
    if rows.len() > PARAM_MAX_ROWS {
        return Err(budget(format!(
            "{}: {} rows exceeds the {PARAM_MAX_ROWS}-row budget",
            table(),
            rows.len()
        )));
    }
    let mut parameters = ParamTable::new();
    for (index, row) in rows.iter().enumerate() {
        let context = format!("{} row {index}", table());
        let row = decode_row(row, context.clone())?;
        if parameters.get(row.seq).is_some() {
            return Err(invalid(format!(
                "{context}: duplicate parameter sequence {}",
                row.seq.0
            )));
        }
        parameters
            .insert(row)
            .map_err(|error| param_error(&context, error))?;
    }
    // The persisted graph must be a DAG: a dangling reference or a cycle is a
    // malformed document, not a partial table.
    cad_domain::param::evaluate_table(&parameters).map_err(|error| param_error(table(), error))?;
    let encoded = encoded_len(rows)?;
    if encoded > PARAM_MAX_TABLE_BYTES {
        return Err(budget(format!(
            "{}: the table encodes to {encoded} bytes, over the \
             {PARAM_MAX_TABLE_BYTES}-byte budget",
            table()
        )));
    }
    Ok(parameters)
}

/// The exact byte length the writer emits for this table.
fn encoded_len(rows: &[Value]) -> Result<usize, SnapshotError> {
    serde_json::to_vec(rows)
        .map(|bytes| bytes.len())
        .map_err(|error| SnapshotError::invalid(format!("{}: {error}", table())))
}

/// Encode one table row. `unit` is omitted when `None` (never written as null).
fn encode_row(row: &ParamRow) -> Value {
    let mut entries = vec![
        ("seq", Value::from(row.seq.0)),
        ("name", Value::from(row.name.as_str())),
        ("kind", Value::from(row.kind.token())),
    ];
    if let Some(unit) = row.unit {
        entries.push(("unit", Value::from(unit.token())));
    }
    entries.push(("expr", encode_expr(&row.expr)));
    object(entries)
}

/// Encode one expression in its single tagged form.
pub(crate) fn encode_expr(expr: &Expr) -> Value {
    match expr {
        Expr::Num(value) => object(vec![("num", json::encode_f64(*value))]),
        Expr::Ref(seq) => object(vec![("ref", Value::from(seq.0))]),
        Expr::Const(constant) => object(vec![("const", Value::from(constant.token()))]),
        Expr::Op { op, lhs, rhs } => object(vec![
            ("op", Value::from(op.token())),
            ("lhs", encode_expr(lhs)),
            ("rhs", encode_expr(rhs)),
        ]),
        Expr::Neg(inner) => object(vec![("neg", encode_expr(inner))]),
        Expr::Pow { base, exp } => {
            object(vec![("pow", encode_expr(base)), ("exp", encode_expr(exp))])
        }
        Expr::Unary { op, value } => object(vec![
            ("fn", Value::from(op.token())),
            ("arg", encode_expr(value)),
        ]),
        Expr::If {
            cond, then_, else_, ..
        } => object(vec![(
            "if",
            object(vec![
                ("cond", encode_expr(cond)),
                ("then", encode_expr(then_)),
                ("else", encode_expr(else_)),
            ]),
        )]),
    }
}

/// Encode one element's flat slot → expression block.
pub(crate) fn encode_bindings(bindings: &ParamBindings) -> Value {
    let mut map = Map::new();
    for (slot, expr) in bindings.entries() {
        map.insert(slot.to_string(), encode_expr(expr));
    }
    Value::Object(map)
}

/// Decode one element `params` block (a present, non-null object).
///
/// Slot legality is deliberately not checked here: the database's builder gate
/// knows the element's geometry type, this codec does not (the same split as a
/// `bim` block's definition references).
pub(crate) fn decode_bindings(
    value: &Value,
    context: String,
) -> Result<ParamBindings, SnapshotError> {
    let Value::Object(map) = value else {
        return Err(invalid(format!("{context}: expected a JSON object")));
    };
    let mut bindings = ParamBindings::new();
    for (slot, expr) in map {
        let decoded = decode_expr(expr, format!("{context}.{slot}"))?;
        bindings
            .insert(slot.clone(), decoded)
            .map_err(|error| param_error(&context, error))?;
    }
    Ok(bindings)
}

/// Decode one table row strictly.
fn decode_row(value: &Value, context: String) -> Result<ParamRow, SnapshotError> {
    let mut reader = Reader::new(value, context.clone())?;
    let raw = reader.integer("seq")?;
    let seq = u64::try_from(raw)
        .map_err(|_| reader.error(format!("seq: {raw} is not a positive parameter sequence")))?;
    let name = reader.string("name")?;
    let kind_token = reader.string("kind")?;
    let kind = ParamKind::parse(&kind_token).ok_or_else(|| {
        reader.error(format!(
            "kind: unknown parameter kind {kind_token:?} (expected length, number or angle)"
        ))
    })?;
    let unit = match reader.opt_take("unit") {
        None => None,
        Some(Value::Null) => {
            return Err(reader.error("unit: expected a string, not null"));
        }
        Some(Value::String(token)) => Some(ParamUnit::parse(&token).ok_or_else(|| {
            reader.error(format!("unit: unknown unit {token:?} (expected mm or rad)"))
        })?),
        Some(_) => return Err(reader.error("unit: expected a string")),
    };
    let expr_value = reader.take("expr")?;
    let expr = decode_expr(&expr_value, format!("{context}.expr"))?;
    reader.finish()?;
    let row = ParamRow::new(ParamSeq(seq), name, kind, expr)
        .map_err(|error| param_error(&context, error))?;
    let row = match unit {
        Some(unit) => row.with_unit(unit),
        None => row,
    };
    // `with_unit` bypasses `new`'s validation, so the canonical-unit rule is
    // re-checked here instead of trusting the constructor.
    row.validate()
        .map_err(|error| param_error(&context, error))?;
    Ok(row)
}

/// Decode one expression tree, exactly one tagged form.
pub(crate) fn decode_expr(value: &Value, context: String) -> Result<Expr, SnapshotError> {
    let mut reader = Reader::new(value, context)?;
    let num = reader.opt_take("num");
    let reference = reader.opt_take("ref");
    let constant = reader.opt_take("const");
    let op = reader.opt_take("op");
    let neg = reader.opt_take("neg");
    let pow = reader.opt_take("pow");
    let exp = reader.opt_take("exp");
    let function = reader.opt_take("fn");
    let arg = reader.opt_take("arg");
    let conditional = reader.opt_take("if");
    let tags = [
        num.is_some(),
        reference.is_some(),
        constant.is_some(),
        op.is_some(),
        neg.is_some(),
        pow.is_some(),
        function.is_some(),
        conditional.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    // `exp`/`arg` are companions, not tags: they are only legal with `pow`/`fn`
    // and are rejected by `finish()` in every other branch.
    if tags != 1 {
        return Err(reader.error(format!(
            "expression must carry exactly one of num, ref, const, op, neg, pow, fn or if \
             (found {tags})"
        )));
    }
    // Companions are only legal with their tag; `opt_take` removed them from the
    // reader, so this is the check `finish()` can no longer make.
    if pow.is_none() && exp.is_some() {
        return Err(reader.error("exp is only valid with pow"));
    }
    if function.is_none() && arg.is_some() {
        return Err(reader.error("arg is only valid with fn"));
    }
    if let Some(value) = num {
        let number =
            json::decode_f64(&value).map_err(|error| reader.error(format!("num: {error}")))?;
        reader.finish()?;
        return Ok(Expr::num(number));
    }
    if let Some(value) = reference {
        let raw = value
            .as_i64()
            .ok_or_else(|| reader.error("ref: expected a JSON integer"))?;
        let seq = u64::try_from(raw).map_err(|_| {
            reader.error(format!("ref: {raw} is not a positive parameter sequence"))
        })?;
        reader.finish()?;
        return Ok(Expr::reference(ParamSeq(seq)));
    }
    if let Some(value) = constant {
        let Value::String(token) = value else {
            return Err(reader.error("const: expected a string"));
        };
        let constant = ParamConst::parse(&token).ok_or_else(|| {
            reader.error(format!("const: unknown constant {token:?} (expected pi)"))
        })?;
        reader.finish()?;
        return Ok(Expr::Const(constant));
    }
    if let Some(value) = neg {
        let inner = decode_expr(&value, format!("{}.neg", reader.context()))?;
        reader.finish()?;
        return Ok(Expr::negated(inner));
    }
    if let Some(value) = pow {
        let exponent = exp.ok_or_else(|| reader.error("pow: missing the exp operand"))?;
        let base = decode_expr(&value, format!("{}.pow", reader.context()))?;
        let exponent = decode_expr(&exponent, format!("{}.exp", reader.context()))?;
        reader.finish()?;
        return Ok(Expr::pow(base, exponent));
    }
    if let Some(value) = function {
        let Value::String(token) = value else {
            return Err(reader.error("fn: expected a string"));
        };
        let op = cad_domain::param::UnaryOp::parse(&token).ok_or_else(|| {
            reader.error(format!(
                "fn: unknown function {token:?} (expected abs, sqrt, sin, cos, tan, \
                 floor, ceil or round)"
            ))
        })?;
        let argument = arg.ok_or_else(|| reader.error("fn: missing the arg operand"))?;
        let argument = decode_expr(&argument, format!("{}.arg", reader.context()))?;
        reader.finish()?;
        return Ok(Expr::unary(op, argument));
    }
    if let Some(value) = conditional {
        let mut nested = reader.child(&value, "if")?;
        let cond = nested.take("cond")?;
        let then_ = nested.take("then")?;
        let else_ = nested.take("else")?;
        let cond = decode_expr(&cond, format!("{}.if.cond", nested.context()))?;
        let then_ = decode_expr(&then_, format!("{}.if.then", nested.context()))?;
        let else_ = decode_expr(&else_, format!("{}.if.else", nested.context()))?;
        nested.finish()?;
        reader.finish()?;
        return Ok(Expr::if_then(cond, then_, else_));
    }
    let value = op.expect("exactly one tag was present");
    let Value::String(token) = value else {
        return Err(reader.error("op: expected a string"));
    };
    let op = BinaryOp::parse(&token).ok_or_else(|| {
        reader.error(format!(
            "op: unknown operator {token:?} (expected add, sub, mul or div)"
        ))
    })?;
    let lhs_value = reader.take("lhs")?;
    let rhs_value = reader.take("rhs")?;
    let lhs = decode_expr(&lhs_value, format!("{}.lhs", reader.context()))?;
    let rhs = decode_expr(&rhs_value, format!("{}.rhs", reader.context()))?;
    reader.finish()?;
    Ok(Expr::binary(op, lhs, rhs))
}

/// Map one domain refusal into a malformed-document error, keeping the marker.
fn param_error(context: &str, error: ParamError) -> SnapshotError {
    SnapshotError::invalid(format!("{context}: {}: {}", error.code(), error.detail()))
}

#[cfg(test)]
mod tests;
