//! Retained-byte estimates for parameter history rows.
//!
//! Split out of `lib.rs` so the history module stays under the 1000-line gate.
//! These numbers feed [`crate::History`]'s memory budget; they are approximate by
//! construction (the same contract as the other per-batch estimators).

/// Estimated retained bytes of one parameter row.
///
/// Almost all of a row is its name and its expression tree, so the fixed-size
/// value alone would badly under-count a long expression.
pub(crate) fn param_row_bytes(row: &cad_domain::param::ParamRow) -> usize {
    std::mem::size_of::<cad_domain::param::ParamRow>() + row.name.len() + expr_bytes(&row.expr)
}

/// Estimated retained bytes of one expression tree (nodes plus boxed operands).
pub(crate) fn expr_bytes(expr: &cad_domain::param::Expr) -> usize {
    use cad_domain::param::Expr;
    match expr {
        Expr::Num(_) | Expr::Ref(_) | Expr::Const(_) => std::mem::size_of::<Expr>(),
        Expr::Op { lhs, rhs, .. } => {
            std::mem::size_of::<Expr>()
                + std::mem::size_of::<Box<Expr>>()
                + expr_bytes(lhs)
                + expr_bytes(rhs)
        }
        Expr::Neg(inner) => {
            std::mem::size_of::<Expr>() + std::mem::size_of::<Box<Expr>>() + expr_bytes(inner)
        }
        Expr::Pow { base, exp } => {
            std::mem::size_of::<Expr>()
                + 2 * std::mem::size_of::<Box<Expr>>()
                + expr_bytes(base)
                + expr_bytes(exp)
        }
        Expr::Unary { value, .. } => {
            std::mem::size_of::<Expr>() + std::mem::size_of::<Box<Expr>>() + expr_bytes(value)
        }
        Expr::If {
            cond, then_, else_, ..
        } => {
            std::mem::size_of::<Expr>()
                + 3 * std::mem::size_of::<Box<Expr>>()
                + expr_bytes(cond)
                + expr_bytes(then_)
                + expr_bytes(else_)
        }
    }
}
