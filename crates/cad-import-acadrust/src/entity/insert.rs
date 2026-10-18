//! Block lookup and INSERT (including array / MINSERT) geometry expansion.
//!
//! Submodule of [`crate::entity`].

use super::*;

impl<'a> ImporterBuilder<'a> {
    pub(crate) fn block_id(&self, name: &str) -> BlockId {
        self.block_ids
            .get(name)
            .copied()
            .unwrap_or(BlockId(u128::MAX))
    }

    /// The block's insertion base point, defaulting to the origin.
    pub(crate) fn block_base(&self, name: &str) -> Point3 {
        self.block_base_points
            .get(name)
            .copied()
            .unwrap_or_default()
    }

    /// Convert an INSERT (including an array / MINSERT) into expandable
    /// geometry.
    ///
    /// One cell yields a single [`SemanticGeometry::Insert`]; `rows × columns`
    /// cells yield a [`SemanticGeometry::Compound`] of them, one per cell (audit
    /// B31). The block definition itself is never emitted directly, so it cannot
    /// be double-drawn. Unknown block names are reported `Missing` rather than
    /// expanded as an empty success.
    pub(crate) fn insert_semantics(
        &self,
        i: &acadrust::entities::Insert,
    ) -> (SemanticGeometry, Completeness) {
        let name = i.block_name.clone();
        let base = self.block_base(&name);
        let mut completeness = Completeness::Complete;
        if !self.block_ids.contains_key(&name) {
            completeness =
                Completeness::Missing(vec![format!("insert references unknown block '{name}'")]);
        }
        let columns = i.column_count.max(1) as usize;
        let rows = i.row_count.max(1) as usize;
        // A non-finite spacing cannot produce distinct cells; drawing them all on
        // top of each other would hide the state, so report it.
        let spacing_bad = |s: f64| !s.is_finite();
        if columns * rows > 1 && (spacing_bad(i.column_spacing) || spacing_bad(i.row_spacing)) {
            completeness = Completeness::Partial(vec![
                "insert array has a non-finite row/column spacing".into(),
            ]);
        }
        let mut instances = Vec::with_capacity(columns * rows);
        for row in 0..rows {
            for column in 0..columns {
                instances.push(SemanticGeometry::Insert {
                    block: self.block_id(&name),
                    transform: insert_array_transform(
                        i,
                        base,
                        column as f64 * i.column_spacing,
                        row as f64 * i.row_spacing,
                    ),
                });
            }
        }
        let geometry = match instances.len() {
            1 => instances.pop().expect("length checked"),
            _ => SemanticGeometry::Compound(instances),
        };
        (geometry, completeness)
    }
}
