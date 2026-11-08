//! Synthetic invoices with many lines, for the measurements and budgets of the time and
//! memory a validation takes on large documents.

/// The invoice `base` (UBL, with at least one `cac:InvoiceLine`) with exactly `count`
/// invoice lines: its own lines, repeated in their order. `None` when `base` has no
/// invoice line.
#[must_use]
pub fn with_lines(base: &str, count: usize) -> Option<String> {
    const OPEN: &str = "<cac:InvoiceLine>";
    const CLOSE: &str = "</cac:InvoiceLine>";
    let start = base.find(OPEN)?;
    let end = base.rfind(CLOSE)? + CLOSE.len();
    let mut lines = Vec::new();
    let mut rest = base.get(start..end)?;
    while let Some(at) = rest.find(CLOSE) {
        let (line, after) = rest.split_at(at + CLOSE.len());
        lines.push(line);
        rest = after;
    }
    let mut out = String::from(&base[..start]);
    for line in lines.iter().cycle().take(count) {
        out.push_str(line);
    }
    out.push_str(&base[end..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lines_are_repeated_in_order() {
        let base = "<I><a/><cac:InvoiceLine>1</cac:InvoiceLine>\n<cac:InvoiceLine>2</cac:InvoiceLine><b/></I>";
        assert_eq!(
            super::with_lines(base, 3).unwrap(),
            "<I><a/><cac:InvoiceLine>1</cac:InvoiceLine>\n<cac:InvoiceLine>2</cac:InvoiceLine><cac:InvoiceLine>1</cac:InvoiceLine><b/></I>"
        );
        assert_eq!(super::with_lines(base, 0).unwrap(), "<I><a/><b/></I>");
        assert_eq!(super::with_lines("<I/>", 3), None);
    }
}
