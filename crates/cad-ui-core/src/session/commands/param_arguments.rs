//! The `-PARAMETERS` command-line grammar (ADR-0043 D9; no solver is
//! implemented, so only the user-parameter table exists).
//!
//! `PARAMETERS` (no dash) is the chrome-local Parameters Manager *panel*; the
//! dashed spelling is the **command-line** form. Its bare name is the read-only
//! list ([`crate::CommandId::ParamList`]); the typed forms below are the one
//! structural/editor surface the panel itself drives:
//!
//! ```text
//! -PARAMETERS                          list every user parameter
//! -PARAMETERS ?                        the same list (explicit report form)
//! -PARAMETERS NEW <name> <kind> <expr> create a row at the next sequence
//! -PARAMETERS SET <target> <expr>      replace one row's expression
//! -PARAMETERS RENAME <target> <name>   rename one row (references survive)
//! -PARAMETERS DELETE <target>          delete one unreferenced row
//! ```
//!
//! * `<name>` is one token; a name with spaces is quoted (`"My width"`). Names
//!   are compared **exactly** by the parameter table, so the command-line tail
//!   is forwarded with the typed spelling (the React chrome treats it as a
//!   case-preserving tail) and the host never case-folds a name.
//! * `<kind>` is `length`, `number` or `angle`, case-insensitive: the same three
//!   tokens the parameter table stores.
//! * `<target>` is an exact parameter name first, then a numeric sequence.
//! * `<expr>` is a number or an arithmetic expression over **parameter names**,
//!   `pi`, the operators `+ - * / ^` and parentheses. It is parsed into the
//!   domain's own [`Expr`] tree here; dimensions, references, cycles and budgets
//!   stay the domain/database call (one merged write path), never a second
//!   grammar. Functions (`sqrt`, `if`, …) are stored forms a document may carry
//!   but are not admitted on this typed line: a function name is refused as an
//!   unknown parameter by name.
//!
//! Every form performs exactly one `PARAM` op, so one command is one transaction,
//! one revision and one undo step. Nothing is defaulted: a missing operand, an
//! unknown keyword, an empty target or an unparsable expression is a stable
//! refusal and writes nothing.
//!
//! The grammar is split from `session/point_style.rs` (the argument dispatch)
//! because it is a pure parser here and a session read there; every SQL-free
//! contract test can exercise it directly.

use cad_domain::param::{BinaryOp, Expr, ParamConst, ParamKind, ParamSeq};

/// The `-PARAMETERS` selector (the dashed command-line spelling).
pub const DASH_PARAMETERS_SELECTOR: &str = "-PARAMETERS";

/// One parameter target of a write form.
///
/// A name that the current document knows is resolved to its sequence during
/// parsing (exact match); an unknown token that is not a positive integer stays a
/// [`ParamTarget::Name`], so the session can refuse it with the precise
/// `param.unknown_name` reason instead of guessing a sequence. A token that *is*
/// a positive integer is accepted as a sequence even when no such row exists; the
/// application then refuses it with `param.unknown_reference`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamTarget {
    /// A resolved or explicitly typed sequence.
    Seq(u64),
    /// An unresolved name (the session refuses it by name).
    Name(String),
}

/// One `-PARAMETERS` typed line.
#[derive(Debug, Clone, PartialEq)]
pub enum DashParametersOps {
    /// The read-only list form (`?`/`LIST`).
    List,
    /// `NEW <name> <kind> <expr>`.
    Create {
        name: String,
        kind: ParamKind,
        expr: Expr,
    },
    /// `SET <target> <expr>`.
    SetValue { target: ParamTarget, expr: Expr },
    /// `RENAME <target> <name>`.
    Rename { target: ParamTarget, name: String },
    /// `DELETE <target>`.
    Delete { target: ParamTarget },
}

/// A refusal carrying one stable `param.*` marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamArgumentError {
    code: &'static str,
    detail: String,
}

impl ParamArgumentError {
    fn invalid(detail: impl Into<String>) -> Self {
        Self {
            code: "param.invalid_value",
            detail: detail.into(),
        }
    }

    fn unknown_reference(detail: impl Into<String>) -> Self {
        Self {
            code: "param.unknown_reference",
            detail: detail.into(),
        }
    }

    /// The stable marker (`param.invalid_value` / `param.unknown_reference`).
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Human-readable context. Not part of the contract.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Parse the argument of a `-PARAMETERS` line.
///
/// `resolve` maps an exact parameter name to its stored sequence; it is the
/// caller's read of the authoritative table (the session's projection), so this
/// parser stays pure and a test can supply a fixed table.
pub fn parse_dash_parameters_argument(
    argument: &str,
    resolve: impl Fn(&str) -> Option<u64>,
) -> Result<DashParametersOps, ParamArgumentError> {
    let text = argument.trim();
    if text.is_empty() {
        return Err(ParamArgumentError::invalid(
            "-PARAMETERS needs ?/LIST or one option: NEW, SET, RENAME or DELETE",
        ));
    }
    let (keyword, rest) = take_token(text)
        .ok_or_else(|| ParamArgumentError::invalid("missing -PARAMETERS option"))?;
    match keyword.to_ascii_uppercase().as_str() {
        "?" | "LIST" => {
            if !rest.trim().is_empty() {
                return Err(ParamArgumentError::invalid(
                    "the -PARAMETERS list form takes no further arguments",
                ));
            }
            Ok(DashParametersOps::List)
        }
        "NEW" | "N" => {
            let (name, rest) = take_token(rest).ok_or_else(|| {
                ParamArgumentError::invalid("NEW needs a name, a kind and an expression")
            })?;
            let (kind, rest) = take_token(rest).ok_or_else(|| {
                ParamArgumentError::invalid("NEW needs a kind after the name (length|number|angle)")
            })?;
            let kind = parse_kind(&kind)?;
            let expr_text = rest.trim();
            if expr_text.is_empty() {
                return Err(ParamArgumentError::invalid(
                    "NEW needs an expression after the kind",
                ));
            }
            Ok(DashParametersOps::Create {
                name,
                kind,
                expr: parse_expression(expr_text, &resolve)?,
            })
        }
        "SET" | "S" => {
            let (target, rest) = take_token(rest).ok_or_else(|| {
                ParamArgumentError::invalid("SET needs a target and an expression")
            })?;
            let expr_text = rest.trim();
            if expr_text.is_empty() {
                return Err(ParamArgumentError::invalid(
                    "SET needs an expression after the target",
                ));
            }
            Ok(DashParametersOps::SetValue {
                target: parse_target(&target, &resolve),
                expr: parse_expression(expr_text, &resolve)?,
            })
        }
        "RENAME" | "R" => {
            let (target, rest) = take_token(rest).ok_or_else(|| {
                ParamArgumentError::invalid("RENAME needs a target and a new name")
            })?;
            let name = take_name(rest)?;
            Ok(DashParametersOps::Rename {
                target: parse_target(&target, &resolve),
                name,
            })
        }
        "DELETE" | "D" => {
            let (target, rest) = take_token(rest)
                .ok_or_else(|| ParamArgumentError::invalid("DELETE needs a target"))?;
            if !rest.trim().is_empty() {
                return Err(ParamArgumentError::invalid(
                    "DELETE takes exactly one target",
                ));
            }
            Ok(DashParametersOps::Delete {
                target: parse_target(&target, &resolve),
            })
        }
        other => Err(ParamArgumentError::invalid(format!(
            "unknown -PARAMETERS option {other:?} (expected NEW, SET, RENAME or DELETE)"
        ))),
    }
}

/// The stable refusal for a `-PARAMETERS` argument outside the grammar.
pub fn invalid_dash_parameters_argument_reason(text: &str) -> String {
    format!(
        "unsupported CAD command: {} (-PARAMETERS takes ?/LIST or one option: \
         NEW <name> <length|number|angle> <expr>, SET <target> <expr>, RENAME <target> <name> \
         or DELETE <target>; <target> is an exact name or a sequence and <expr> is a number or \
         an arithmetic expression over parameter names)",
        text.trim()
    )
}

/// One token: `"quoted text with spaces"` or a bare whitespace-delimited run.
fn take_token(text: &str) -> Option<(String, &str)> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(after) = trimmed.strip_prefix('"') {
        let end = after.find('"')?;
        return Some((after[..end].to_string(), &after[end + 1..]));
    }
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    Some((trimmed[..end].to_string(), &trimmed[end..]))
}

/// One rename target name: a single quoted token, or the whole remainder.
fn take_name(text: &str) -> Result<String, ParamArgumentError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParamArgumentError::invalid("RENAME needs a new name"));
    }
    if trimmed.starts_with('"') {
        let (name, rest) = take_token(trimmed)
            .ok_or_else(|| ParamArgumentError::invalid("RENAME has an unterminated quoted name"))?;
        if !rest.trim().is_empty() {
            return Err(ParamArgumentError::invalid(
                "RENAME takes exactly one new name (quote a name with spaces)",
            ));
        }
        return Ok(name);
    }
    Ok(trimmed.to_string())
}

fn parse_kind(token: &str) -> Result<ParamKind, ParamArgumentError> {
    ParamKind::parse(&token.to_ascii_lowercase()).ok_or_else(|| {
        ParamArgumentError::invalid(format!(
            "unknown parameter kind {token:?} (expected length, number or angle)"
        ))
    })
}

fn parse_target(token: &str, resolve: &impl Fn(&str) -> Option<u64>) -> ParamTarget {
    if let Some(seq) = resolve(token) {
        return ParamTarget::Seq(seq);
    }
    match token.parse::<u64>() {
        Ok(seq) if seq >= 1 => ParamTarget::Seq(seq),
        _ => ParamTarget::Name(token.to_string()),
    }
}

// ---------------------------------------------------------------------------
// The expression text grammar.
// ---------------------------------------------------------------------------

/// Parse one arithmetic expression into the domain's [`Expr`] tree.
///
/// Grammar (precedence low → high):
///
/// ```text
/// expr    := term (('+' | '-') term)*
/// term    := unary (('*' | '/') unary)*
/// unary   := ('+' | '-') unary | power
/// power   := primary ('^' unary)?          (right-associative)
/// primary := number | name | 'pi' | '(' expr ')'
/// ```
///
/// A name resolves through `resolve` to a stored sequence; an unknown name is a
/// `param.unknown_reference` refusal (never a silent `0`). Numbers must be
/// finite. The tree is validated structurally by `Expr::validate_shape` when the
/// application builds the row; dimensions, cycles and reference budgets stay the
/// domain/database call.
pub fn parse_expression(
    text: &str,
    resolve: &impl Fn(&str) -> Option<u64>,
) -> Result<Expr, ParamArgumentError> {
    let tokens = tokenize_expression(text)?;
    let mut parser = ExpressionParser {
        tokens: &tokens,
        position: 0,
        resolve,
    };
    let expr = parser.parse_sum()?;
    if parser.position != tokens.len() {
        return Err(ParamArgumentError::invalid(format!(
            "unexpected token {:?} after the expression",
            tokens[parser.position]
        )));
    }
    expr.validate_shape().map_err(|error| {
        ParamArgumentError::invalid(format!("{}: {}", error.code(), error.detail()))
    })?;
    Ok(expr)
}

#[derive(Debug, Clone, PartialEq)]
enum ExpressionToken {
    Number(f64),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Open,
    Close,
}

impl std::fmt::Display for ExpressionToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::Name(name) => formatter.write_str(name),
            Self::Plus => formatter.write_str("+"),
            Self::Minus => formatter.write_str("-"),
            Self::Star => formatter.write_str("*"),
            Self::Slash => formatter.write_str("/"),
            Self::Caret => formatter.write_str("^"),
            Self::Open => formatter.write_str("("),
            Self::Close => formatter.write_str(")"),
        }
    }
}

fn tokenize_expression(text: &str) -> Result<Vec<ExpressionToken>, ParamArgumentError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() {
            index += 1;
            continue;
        }
        match current {
            '+' => {
                tokens.push(ExpressionToken::Plus);
                index += 1;
            }
            '-' => {
                tokens.push(ExpressionToken::Minus);
                index += 1;
            }
            '*' => {
                tokens.push(ExpressionToken::Star);
                index += 1;
            }
            '/' => {
                tokens.push(ExpressionToken::Slash);
                index += 1;
            }
            '^' => {
                tokens.push(ExpressionToken::Caret);
                index += 1;
            }
            '(' => {
                tokens.push(ExpressionToken::Open);
                index += 1;
            }
            ')' => {
                tokens.push(ExpressionToken::Close);
                index += 1;
            }
            _ if current.is_ascii_digit() || current == '.' => {
                let start = index;
                while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.')
                {
                    index += 1;
                }
                if index < chars.len() && (chars[index] == 'e' || chars[index] == 'E') {
                    index += 1;
                    if index < chars.len() && (chars[index] == '+' || chars[index] == '-') {
                        index += 1;
                    }
                    while index < chars.len() && chars[index].is_ascii_digit() {
                        index += 1;
                    }
                }
                let literal: String = chars[start..index].iter().collect();
                let value = literal.parse::<f64>().map_err(|_| {
                    ParamArgumentError::invalid(format!("{literal:?} is not a number"))
                })?;
                if !value.is_finite() {
                    return Err(ParamArgumentError::invalid(format!(
                        "{literal:?} is not a finite number"
                    )));
                }
                tokens.push(ExpressionToken::Number(value));
            }
            _ if current.is_ascii_alphabetic() || current == '_' => {
                let start = index;
                while index < chars.len()
                    && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                tokens.push(ExpressionToken::Name(chars[start..index].iter().collect()));
            }
            other => {
                return Err(ParamArgumentError::invalid(format!(
                    "unexpected character {other:?} in the expression"
                )));
            }
        }
    }
    if tokens.is_empty() {
        return Err(ParamArgumentError::invalid("the expression is empty"));
    }
    Ok(tokens)
}

struct ExpressionParser<'a, F: Fn(&str) -> Option<u64>> {
    tokens: &'a [ExpressionToken],
    position: usize,
    resolve: &'a F,
}

impl<F: Fn(&str) -> Option<u64>> ExpressionParser<'_, F> {
    fn peek(&self) -> Option<&ExpressionToken> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<&ExpressionToken> {
        let token = self.tokens.get(self.position);
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn parse_sum(&mut self) -> Result<Expr, ParamArgumentError> {
        let mut lhs = self.parse_product()?;
        while let Some(token) = self.peek() {
            let op = match token {
                ExpressionToken::Plus => BinaryOp::Add,
                ExpressionToken::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_product()?;
            lhs = Expr::Op {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_product(&mut self) -> Result<Expr, ParamArgumentError> {
        let mut lhs = self.parse_unary()?;
        while let Some(token) = self.peek() {
            let op = match token {
                ExpressionToken::Star => BinaryOp::Mul,
                ExpressionToken::Slash => BinaryOp::Div,
                _ => break,
            };
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Op {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParamArgumentError> {
        match self.peek() {
            Some(ExpressionToken::Minus) => {
                self.advance();
                Ok(Expr::Neg(Box::new(self.parse_unary()?)))
            }
            Some(ExpressionToken::Plus) => {
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<Expr, ParamArgumentError> {
        let base = self.parse_primary()?;
        if matches!(self.peek(), Some(ExpressionToken::Caret)) {
            self.advance();
            let exp = self.parse_unary()?;
            return Ok(Expr::Pow {
                base: Box::new(base),
                exp: Box::new(exp),
            });
        }
        Ok(base)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParamArgumentError> {
        let Some(token) = self.advance().cloned() else {
            return Err(ParamArgumentError::invalid(
                "the expression ends where a value is expected",
            ));
        };
        match token {
            ExpressionToken::Number(value) => Ok(Expr::Num(value)),
            ExpressionToken::Name(name) => {
                if name.eq_ignore_ascii_case("pi") {
                    return Ok(Expr::Const(ParamConst::Pi));
                }
                match (self.resolve)(&name) {
                    Some(seq) => Ok(Expr::Ref(ParamSeq(seq))),
                    None => Err(ParamArgumentError::unknown_reference(format!(
                        "the expression names {name:?}, which is not a parameter of this document \
                         (functions such as sqrt/if are stored forms, not typed expressions)"
                    ))),
                }
            }
            ExpressionToken::Open => {
                let inner = self.parse_sum()?;
                match self.advance() {
                    Some(ExpressionToken::Close) => Ok(inner),
                    _ => Err(ParamArgumentError::invalid("a '(' is not closed by ')'")),
                }
            }
            other => Err(ParamArgumentError::invalid(format!(
                "unexpected token {other} where a value is expected"
            ))),
        }
    }
}
