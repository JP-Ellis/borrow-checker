// The dashboard's metadata line.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim, which is why this header uses `//` rather than `//!`.

use bc_ipc::AccountDetail;

use crate::format::kind_label;

/// The account's kind, default commodity, and open and close dates.
///
/// Parts the account lacks are left out; the kind is always present.
///
/// # Arguments
///
/// * `detail` - The account.
///
/// # Returns
///
/// The parts joined by ` · `, such as `deposit · AUD · opened 2026-01-05`.
#[must_use]
pub(crate) fn meta_text(detail: &AccountDetail) -> String {
    let mut parts = vec![kind_label(detail.kind).to_owned()];
    parts.extend(detail.commodity_codes.first().cloned());
    parts.extend(detail.opened_on.map(|d| format!("opened {d}")));
    parts.extend(detail.closed_on.map(|d| format!("closed {d}")));
    parts.join(" \u{b7} ")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountDetail;
    use bc_ipc::AccountKindDto;
    use bc_ipc::AccountType;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    use super::meta_text;

    fn detail(kind: AccountKindDto, commodities: &[&str]) -> AccountDetail {
        AccountDetail::builder()
            .id("a1")
            .name("Cheque")
            .path("Assets :: Cheque")
            .account_type(AccountType::Asset)
            .kind(kind)
            .commodity_codes(commodities.iter().map(|c| (*c).to_owned()).collect())
            .tags(vec![])
            .event_count(0)
            .build()
    }

    #[test]
    fn every_part_in_order() {
        let mut d = detail(AccountKindDto::DepositAccount, &["AUD", "USD"]);
        d.opened_on = Some(date(2026, 1, 5));
        d.closed_on = Some(date(2026, 10, 31));
        assert_eq!(
            meta_text(&d),
            "deposit \u{b7} AUD \u{b7} opened 2026-01-05 \u{b7} closed 2026-10-31"
        );
    }

    #[test]
    fn missing_parts_are_skipped() {
        let mut d = detail(AccountKindDto::Group, &[]);
        assert_eq!(meta_text(&d), "group");
        d.closed_on = Some(date(2026, 10, 31));
        assert_eq!(meta_text(&d), "group \u{b7} closed 2026-10-31");
    }
}
