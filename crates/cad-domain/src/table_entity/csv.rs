//! CSV export of one table's cell grid (`TABLEEXPORT`).
//!
//! The serializer is a **pure derivation of the stored grid**, never of the
//! display: every stored cell is data, including the covered cells of a merge
//! range (they are the fields of the spanned columns). What a merged range
//! changes is *where* its text appears — the anchor cell — and every other field
//! in the range is written empty, which is the shape a spreadsheet shows for a
//! merged cell.
//!
//! Format (documented because it is a file ABI):
//!
//! * one record per table row, top row first, one field per table column, left
//!   column first — no header line (the column *names* are column titles, not
//!   data rows, and a table may have no header row at all in this build);
//! * `,` between fields and `\r\n` after every record, including the last
//!   (RFC 4180's line ending);
//! * a field containing `,`, `"`, `\r` or `\n` is quoted with `"`, and an inner
//!   `"` is doubled — the RFC 4180 escape, so an Excel/LibreOffice reader reads
//!   the same cells back;
//! * a cell's text is written **verbatim**: this build has no value-format model,
//!   so nothing is reformatted, rounded or re-localized.
//!
//! The output is UTF-8 text with no BOM: a BOM would become part of the first
//! field for a plain RFC 4180 reader.

use crate::TableGeometry;

impl TableGeometry {
    /// The whole grid as RFC 4180 CSV text, one record per row.
    ///
    /// A table with no rows or no columns has no grid and produces an empty
    /// string; the command layer refuses such a table before writing a file, so
    /// an empty file is never produced from a refusal.
    pub fn to_csv(&self) -> String {
        let column_count = self.columns().len();
        if column_count == 0 || self.rows().is_empty() {
            return String::new();
        }
        let covered = super::edit::merge_coverage(self.rows(), column_count);
        let mut out = String::new();
        for (row_index, row) in self.rows().iter().enumerate() {
            for column_index in 0..column_count {
                if column_index > 0 {
                    out.push(',');
                }
                if covered.contains(&(row_index, column_index)) {
                    // A covered cell draws no text of its own, so the field is
                    // empty even when the source stored content there.
                    continue;
                }
                if let Some(cell) = row.cells.get(column_index) {
                    push_field(&mut out, &cell.text);
                }
            }
            out.push_str("\r\n");
        }
        out
    }
}

/// Append one RFC 4180 field, quoting only when the value needs it.
fn push_field(out: &mut String, value: &str) {
    if !value.contains([',', '"', '\r', '\n']) {
        out.push_str(value);
        return;
    }
    out.push('"');
    for character in value.chars() {
        if character == '"' {
            out.push('"');
        }
        out.push(character);
    }
    out.push('"');
}
