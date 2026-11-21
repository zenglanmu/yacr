//! The P0 expression grammar, dimension rules and pure evaluation (ADR-0043 D5).

use std::collections::BTreeSet;

use super::{invalid, ParamError, ParamKind, ParamSeq, ParamValues};
use super::{
    PARAM_BUDGET_EXCEEDED, PARAM_MAX_EXPR_DEPTH, PARAM_MAX_EXPR_NODES, PARAM_UNIT_MISMATCH,
    PARAM_UNKNOWN_REFERENCE,
};

// ---------------------------------------------------------------------------
// Expressions.
// ---------------------------------------------------------------------------

/// A binary operator in the P0 closed grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl BinaryOp {
    pub fn token(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
            Self::Div => "div",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Sub),
            "mul" => Some(Self::Mul),
            "div" => Some(Self::Div),
            _ => None,
        }
    }
}

/// A one-argument function in the P1 grammar.
///
/// `Abs`/`Floor`/`Ceil`/`Round` preserve their argument's dimension; `Sqrt`
/// requires a dimensionless argument (there is no `length^0.5` kind); the
/// trigonometric functions accept a dimensionless or `angle` argument and are
/// dimensionless (canonical radians, never converted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Abs,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Floor,
    Ceil,
    Round,
}

impl UnaryOp {
    pub fn token(self) -> &'static str {
        match self {
            Self::Abs => "abs",
            Self::Sqrt => "sqrt",
            Self::Sin => "sin",
            Self::Cos => "cos",
            Self::Tan => "tan",
            Self::Floor => "floor",
            Self::Ceil => "ceil",
            Self::Round => "round",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "abs" => Some(Self::Abs),
            "sqrt" => Some(Self::Sqrt),
            "sin" => Some(Self::Sin),
            "cos" => Some(Self::Cos),
            "tan" => Some(Self::Tan),
            "floor" => Some(Self::Floor),
            "ceil" => Some(Self::Ceil),
            "round" => Some(Self::Round),
            _ => None,
        }
    }
}

/// Named mathematical constants. `pi` is dimensionless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamConst {
    Pi,
}

impl ParamConst {
    pub fn token(self) -> &'static str {
        match self {
            Self::Pi => "pi",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }

    pub fn value(self) -> f64 {
        match self {
            Self::Pi => std::f64::consts::PI,
        }
    }
}

/// The closed expression grammar (ADR-0043 §2: P0 plus the P1 numeric
/// extensions).
///
/// The tagged JSON shape is documented on the encoder in `cad-db`:
///
/// ```json
/// {"num": 1.0}
/// {"ref": 7}
/// {"op": "div", "lhs": {"ref": 7}, "rhs": {"num": 2}}
/// {"const": "pi"}
/// {"neg": {"ref": 7}}
/// {"pow": {"ref": 7}, "exp": {"num": 2}}
/// {"fn": "sqrt", "arg": {"ref": 7}}
/// {"if": {"cond": ..., "then": ..., "else": ...}}
/// ```
///
/// Text templates are a separate, string-valued expression
/// ([`super::TextTemplate`]) because a slot either carries a number or text,
/// never both.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Ref(ParamSeq),
    Const(ParamConst),
    Op {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Neg(Box<Expr>),
    Pow {
        base: Box<Expr>,
        exp: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        value: Box<Expr>,
    },
    If {
        cond: Box<Expr>,
        then_: Box<Expr>,
        else_: Box<Expr>,
    },
}

impl Expr {
    pub fn num(value: f64) -> Self {
        Self::Num(value)
    }

    pub fn reference(seq: ParamSeq) -> Self {
        Self::Ref(seq)
    }

    pub fn binary(op: BinaryOp, lhs: Expr, rhs: Expr) -> Self {
        Self::Op {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    pub fn negated(value: Expr) -> Self {
        Self::Neg(Box::new(value))
    }

    pub fn pow(base: Expr, exp: Expr) -> Self {
        Self::Pow {
            base: Box::new(base),
            exp: Box::new(exp),
        }
    }

    pub fn unary(op: UnaryOp, value: Expr) -> Self {
        Self::Unary {
            op,
            value: Box::new(value),
        }
    }

    pub fn if_then(cond: Expr, then_: Expr, else_: Expr) -> Self {
        Self::If {
            cond: Box::new(cond),
            then_: Box::new(then_),
            else_: Box::new(else_),
        }
    }

    /// Whether this expression uses a P1-only node (it therefore needs the
    /// minor-9 tier; a minor-8 reader would refuse the unknown tag).
    pub fn is_p1(&self) -> bool {
        match self {
            Self::Num(_) | Self::Ref(_) | Self::Const(_) => false,
            Self::Op { lhs, rhs, .. } => lhs.is_p1() || rhs.is_p1(),
            Self::Neg(_) | Self::Pow { .. } | Self::Unary { .. } | Self::If { .. } => true,
        }
    }

    /// Collect every referenced sequence.
    pub fn references(&self, out: &mut BTreeSet<ParamSeq>) {
        match self {
            Self::Num(_) | Self::Const(_) => {}
            Self::Ref(seq) => {
                out.insert(*seq);
            }
            Self::Op { lhs, rhs, .. } => {
                lhs.references(out);
                rhs.references(out);
            }
            Self::Neg(inner) => inner.references(out),
            Self::Pow { base, exp } => {
                base.references(out);
                exp.references(out);
            }
            Self::Unary { value, .. } => value.references(out),
            Self::If {
                cond, then_, else_, ..
            } => {
                cond.references(out);
                then_.references(out);
                else_.references(out);
            }
        }
    }

    fn node_count(&self) -> usize {
        match self {
            Self::Num(_) | Self::Const(_) | Self::Ref(_) => 1,
            Self::Op { lhs, rhs, .. } => 1 + lhs.node_count() + rhs.node_count(),
            Self::Neg(inner) => 1 + inner.node_count(),
            Self::Pow { base, exp } => 1 + base.node_count() + exp.node_count(),
            Self::Unary { value, .. } => 1 + value.node_count(),
            Self::If {
                cond, then_, else_, ..
            } => 1 + cond.node_count() + then_.node_count() + else_.node_count(),
        }
    }

    fn depth(&self) -> usize {
        match self {
            Self::Num(_) | Self::Const(_) | Self::Ref(_) => 1,
            Self::Op { lhs, rhs, .. } => 1 + lhs.depth().max(rhs.depth()),
            Self::Neg(inner) => 1 + inner.depth(),
            Self::Pow { base, exp } => 1 + base.depth().max(exp.depth()),
            Self::Unary { value, .. } => 1 + value.depth(),
            Self::If {
                cond, then_, else_, ..
            } => 1 + cond.depth().max(then_.depth()).max(else_.depth()),
        }
    }

    /// Enforce the per-expression budgets and literal validity.
    pub fn validate_shape(&self) -> Result<(), ParamError> {
        let nodes = self.node_count();
        if nodes > PARAM_MAX_EXPR_NODES {
            return Err(ParamError::new(
                PARAM_BUDGET_EXCEEDED,
                format!(
                    "expression has {nodes} nodes, over the {PARAM_MAX_EXPR_NODES}-node budget"
                ),
            ));
        }
        let depth = self.depth();
        if depth > PARAM_MAX_EXPR_DEPTH {
            return Err(ParamError::new(
                PARAM_BUDGET_EXCEEDED,
                format!("expression is {depth} deep, over the {PARAM_MAX_EXPR_DEPTH}-level budget"),
            ));
        }
        self.check_finite()
    }

    fn check_finite(&self) -> Result<(), ParamError> {
        match self {
            Self::Num(value) if !value.is_finite() => Err(invalid(format!(
                "expression holds the non-finite literal {value}"
            ))),
            Self::Num(_) | Self::Const(_) | Self::Ref(_) => Ok(()),
            Self::Op { lhs, rhs, .. } => {
                lhs.check_finite()?;
                rhs.check_finite()
            }
            Self::Neg(inner) => inner.check_finite(),
            Self::Pow { base, exp } => {
                base.check_finite()?;
                exp.check_finite()
            }
            Self::Unary { value, .. } => value.check_finite(),
            Self::If {
                cond, then_, else_, ..
            } => {
                cond.check_finite()?;
                then_.check_finite()?;
                else_.check_finite()
            }
        }
    }
}
// ---------------------------------------------------------------------------
// Dimension rules.
// ---------------------------------------------------------------------------

/// The dimension of an expression.
///
/// `Free` means "adopts the surrounding kind": a numeric literal, `pi`, or a
/// `number`-kind parameter carries no unit of its own, exactly like the ADR
/// example where a `{"num": 12.5}` in a length row means 12.5 mm. A reference to
/// a `length`/`angle` parameter is `Fixed` to that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Free,
    Fixed(ParamKind),
}

impl Dimension {
    fn kind(self) -> Option<ParamKind> {
        match self {
            Self::Free => None,
            Self::Fixed(kind) => Some(kind),
        }
    }
}

/// The dimension of an expression (see [`Dimension`]).
///
/// `Add`/`Sub` require compatible dimensions (a free side adopts the fixed one).
/// `Mul` accepts one free side; `Div` accepts a free divisor and yields free only
/// for two fixed operands of the **same** kind (`length / length`), refusing
/// `length / angle` and `number / length`. Anything else is
/// [`PARAM_UNIT_MISMATCH`]; no conversion is attempted.
pub fn dimension(expr: &Expr, values: &ParamValues) -> Result<Dimension, ParamError> {
    match expr {
        Expr::Num(_) | Expr::Const(_) => Ok(Dimension::Free),
        // A `number`-kind parameter is dimensionless, exactly like a literal. It
        // can scale a length and combine with one, but it never *becomes* a
        // length through a reference.
        Expr::Ref(seq) => values
            .get(*seq)
            .map(|value| match value.kind {
                ParamKind::Number => Dimension::Free,
                kind => Dimension::Fixed(kind),
            })
            .ok_or_else(|| unknown_reference(*seq)),
        Expr::Op { op, lhs, rhs } => {
            let lhs = dimension(lhs, values)?;
            let rhs = dimension(rhs, values)?;
            match op {
                BinaryOp::Add | BinaryOp::Sub => match (lhs, rhs) {
                    (Dimension::Free, Dimension::Free) => Ok(Dimension::Free),
                    (Dimension::Fixed(kind), Dimension::Free)
                    | (Dimension::Free, Dimension::Fixed(kind)) => Ok(Dimension::Fixed(kind)),
                    (Dimension::Fixed(a), Dimension::Fixed(b)) if a == b => Ok(Dimension::Fixed(a)),
                    _ => Err(unit_mismatch(*op, lhs.kind(), rhs.kind())),
                },
                BinaryOp::Mul => match (lhs, rhs) {
                    (Dimension::Free, Dimension::Free) => Ok(Dimension::Free),
                    (Dimension::Fixed(kind), Dimension::Free)
                    | (Dimension::Free, Dimension::Fixed(kind)) => Ok(Dimension::Fixed(kind)),
                    (Dimension::Fixed(a), Dimension::Fixed(b)) => {
                        Err(unit_mismatch(*op, Some(a), Some(b)))
                    }
                },
                BinaryOp::Div => match (lhs, rhs) {
                    (Dimension::Free, Dimension::Free) => Ok(Dimension::Free),
                    (Dimension::Fixed(a), Dimension::Fixed(b)) if a == b => Ok(Dimension::Free),
                    (Dimension::Fixed(kind), Dimension::Free) => Ok(Dimension::Fixed(kind)),
                    (Dimension::Fixed(a), Dimension::Fixed(b)) => {
                        Err(unit_mismatch(*op, Some(a), Some(b)))
                    }
                    (Dimension::Free, Dimension::Fixed(kind)) => {
                        Err(unit_mismatch(*op, None, Some(kind)))
                    }
                },
            }
        }
        Expr::Neg(inner) => dimension(inner, values),
        // There is no area/volume kind: a power is dimensionless-only, so
        // `pow(length, 2)` is refused instead of silently producing a length.
        Expr::Pow { base, exp } => {
            for (operand, label) in [(base, "base"), (exp, "exponent")] {
                if dimension(operand, values)? != Dimension::Free {
                    return Err(ParamError::new(
                        PARAM_UNIT_MISMATCH,
                        format!("pow {label} must be dimensionless"),
                    ));
                }
            }
            Ok(Dimension::Free)
        }
        Expr::Unary { op, value } => {
            let inner = dimension(value, values)?;
            match op {
                UnaryOp::Abs | UnaryOp::Floor | UnaryOp::Ceil | UnaryOp::Round => Ok(inner),
                UnaryOp::Sqrt => {
                    if inner == Dimension::Free {
                        Ok(Dimension::Free)
                    } else {
                        Err(ParamError::new(
                            PARAM_UNIT_MISMATCH,
                            "sqrt takes a dimensionless argument".to_string(),
                        ))
                    }
                }
                UnaryOp::Sin | UnaryOp::Cos | UnaryOp::Tan => match inner {
                    Dimension::Free | Dimension::Fixed(ParamKind::Angle) => Ok(Dimension::Free),
                    _ => Err(ParamError::new(
                        PARAM_UNIT_MISMATCH,
                        format!("{:?} takes an angle or a number", op),
                    )),
                },
            }
        }
        Expr::If {
            cond, then_, else_, ..
        } => {
            if dimension(cond, values)? != Dimension::Free {
                return Err(ParamError::new(
                    PARAM_UNIT_MISMATCH,
                    "an if condition must be dimensionless".to_string(),
                ));
            }
            let a = dimension(then_, values)?;
            let b = dimension(else_, values)?;
            match (a, b) {
                (Dimension::Free, Dimension::Free) => Ok(Dimension::Free),
                (Dimension::Free, Dimension::Fixed(kind))
                | (Dimension::Fixed(kind), Dimension::Free) => Ok(Dimension::Fixed(kind)),
                (Dimension::Fixed(a), Dimension::Fixed(b)) if a == b => Ok(Dimension::Fixed(a)),
                _ => Err(ParamError::new(
                    PARAM_UNIT_MISMATCH,
                    "if branches have different dimensions".to_string(),
                )),
            }
        }
    }
}

fn unit_mismatch(op: BinaryOp, lhs: Option<ParamKind>, rhs: Option<ParamKind>) -> ParamError {
    ParamError::new(
        PARAM_UNIT_MISMATCH,
        format!(
            "{:?} cannot combine {} and {}",
            op,
            lhs.map_or("number", ParamKind::token),
            rhs.map_or("number", ParamKind::token)
        ),
    )
}

pub(crate) fn unknown_reference(seq: ParamSeq) -> ParamError {
    ParamError::new(
        PARAM_UNKNOWN_REFERENCE,
        format!(
            "the expression references parameter sequence {}, which does not exist",
            seq.0
        ),
    )
}

/// Evaluate one expression in the context of already-evaluated parameters.
///
/// `expected` is the declared kind of the row or slot. A dimensionless
/// expression is accepted and interpreted in `expected` (a bare `12.5` in a
/// length row means 12.5 mm, as in the ADR example); a dimensioned expression
/// must match exactly.
pub fn resolve(expr: &Expr, expected: ParamKind, values: &ParamValues) -> Result<f64, ParamError> {
    expr.validate_shape()?;
    if let Dimension::Fixed(found) = dimension(expr, values)? {
        if found != expected {
            return Err(ParamError::new(
                PARAM_UNIT_MISMATCH,
                format!(
                    "expression is {} but the target is {}",
                    found.token(),
                    expected.token()
                ),
            ));
        }
    }
    let value = eval(expr, values, 0)?;
    if !value.is_finite() {
        return Err(invalid(format!("the expression evaluated to {value}")));
    }
    Ok(value)
}

fn eval(expr: &Expr, values: &ParamValues, depth: usize) -> Result<f64, ParamError> {
    if depth > PARAM_MAX_EXPR_DEPTH {
        return Err(ParamError::new(
            PARAM_BUDGET_EXCEEDED,
            format!("expression evaluation exceeded {PARAM_MAX_EXPR_DEPTH} levels"),
        ));
    }
    match expr {
        Expr::Num(value) => Ok(*value),
        Expr::Const(constant) => Ok(constant.value()),
        Expr::Ref(seq) => values
            .get(*seq)
            .map(|value| value.value)
            .ok_or_else(|| unknown_reference(*seq)),
        Expr::Op { op, lhs, rhs } => {
            let lhs = eval(lhs, values, depth + 1)?;
            let rhs = eval(rhs, values, depth + 1)?;
            let value = match op {
                BinaryOp::Add => lhs + rhs,
                BinaryOp::Sub => lhs - rhs,
                BinaryOp::Mul => lhs * rhs,
                BinaryOp::Div => {
                    if rhs == 0.0 {
                        return Err(invalid("the expression divides by zero"));
                    }
                    lhs / rhs
                }
            };
            finite(value)
        }
        Expr::Neg(inner) => finite(-eval(inner, values, depth + 1)?),
        Expr::Pow { base, exp } => {
            let base = eval(base, values, depth + 1)?;
            let exp = eval(exp, values, depth + 1)?;
            finite(base.powf(exp))
        }
        Expr::Unary { op, value } => {
            let value = eval(value, values, depth + 1)?;
            let result = match op {
                UnaryOp::Abs => value.abs(),
                UnaryOp::Sqrt => {
                    if value < 0.0 {
                        return Err(invalid(format!("sqrt of the negative value {value}")));
                    }
                    value.sqrt()
                }
                UnaryOp::Sin => value.sin(),
                UnaryOp::Cos => value.cos(),
                UnaryOp::Tan => value.tan(),
                UnaryOp::Floor => value.floor(),
                UnaryOp::Ceil => value.ceil(),
                UnaryOp::Round => value.round(),
            };
            finite(result)
        }
        Expr::If {
            cond, then_, else_, ..
        } => {
            let cond = eval(cond, values, depth + 1)?;
            // Only the taken branch is evaluated: an unused `1/0` must not turn
            // a valid conditional into a failure.
            if cond != 0.0 {
                eval(then_, values, depth + 1)
            } else {
                eval(else_, values, depth + 1)
            }
        }
    }
}

fn finite(value: f64) -> Result<f64, ParamError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid(format!("the expression evaluated to {value}")))
    }
}
