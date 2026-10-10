//! In-app links the import page builds.

/// Settings with the currency registry open.
#[cfg(target_arch = "wasm32")]
pub(crate) const CURRENCIES_HREF: &str = "/settings?section=currencies";

/// Upper-case hex digits for percent-encoding.
const HEX: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
];

/// Percent-encodes a query value, keeping only RFC 3986 unreserved bytes.
///
/// # Arguments
///
/// * `raw` - The value.
///
/// # Returns
///
/// The encoded value.
#[must_use]
pub(crate) fn encode_component(raw: &str) -> String {
    raw.bytes()
        .fold(String::with_capacity(raw.len()), |mut out, byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                out.push(char::from(byte));
            } else {
                out.push('%');
                for nibble in [byte.checked_shr(4).unwrap_or(0), byte & 0x0F] {
                    out.push(HEX.get(usize::from(nibble)).copied().unwrap_or('0'));
                }
            }
            out
        })
}

/// The import page with a History row highlighted.
///
/// # Arguments
///
/// * `batch` - The batch ID.
/// * `arm_discard` - Whether the row opens with its discard armed.
///
/// # Returns
///
/// `/import?batch=<id>`, plus `&discard=1` when armed.
#[must_use]
pub(crate) fn history_href(batch: &str, arm_discard: bool) -> String {
    let arm = if arm_discard { "&discard=1" } else { "" };
    format!("/import?batch={}{arm}", encode_component(batch))
}

/// Settings → Backup with the backup at `path` highlighted.
///
/// # Arguments
///
/// * `path` - The snapshot path a discard recorded.
///
/// # Returns
///
/// `/settings?section=backup&backup=<path>`.
#[must_use]
pub(crate) fn backup_href(path: &str) -> String {
    format!("/settings?section=backup&backup={}", encode_component(path))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("batch-0001", "batch-0001")]
    #[case("Everyday & Bills", "Everyday%20%26%20Bills")]
    #[case("/backups/a b.sqlite", "%2Fbackups%2Fa%20b.sqlite")]
    #[case("Caf\u{e9}", "Caf%C3%A9")]
    fn encode_component_cases(#[case] raw: &str, #[case] expected: &str) {
        assert_eq!(encode_component(raw), expected);
    }

    #[rstest]
    #[case("batch-0001", false, "/import?batch=batch-0001")]
    #[case("batch-0001", true, "/import?batch=batch-0001&discard=1")]
    fn history_href_cases(#[case] batch: &str, #[case] arm: bool, #[case] expected: &str) {
        assert_eq!(history_href(batch, arm), expected);
    }

    #[test]
    fn backup_href_encodes_the_path() {
        assert_eq!(
            backup_href("/backups/ledger_x/20261010-020000000.pre-discard.sqlite"),
            "/settings?section=backup&backup=%2Fbackups%2Fledger_x%2F20261010-020000000.pre-discard.sqlite"
        );
    }
}
