// Links into Settings → Accounts, and the query that opens a draft or
// focuses a row there.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim, which is why this header uses `//` rather than `//!`.

use bc_ipc::AccountType;

use crate::pages::import::links::encode_component;

/// Settings with the accounts table open.
pub(crate) const ACCOUNTS_HREF: &str = "/settings?section=accounts";

/// The accounts table with a new draft row.
///
/// # Arguments
///
/// * `account_type` - The type the draft starts with.
/// * `parent_id` - The parent the draft starts under, if any.
///
/// # Returns
///
/// `/settings?section=accounts&new=<type>`, plus `&parent=<id>` with a parent.
#[must_use]
pub(crate) fn new_account_href(account_type: AccountType, parent_id: Option<&str>) -> String {
    let parent = parent_id.map_or_default(|id| format!("&parent={}", encode_component(id)));
    format!("{ACCOUNTS_HREF}&new={}{parent}", account_type.label())
}

/// The accounts table with the row `id` focused.
///
/// # Returns
///
/// `/settings?section=accounts&focus=<id>`.
#[must_use]
pub(crate) fn focus_account_href(id: &str) -> String {
    format!("{ACCOUNTS_HREF}&focus={}", encode_component(id))
}

/// What the accounts table does on its first load, from the query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryRequest {
    /// A draft row to open, with its starting type and parent.
    pub draft: Option<NewDraft>,
    /// The saved account whose row to scroll to and focus.
    pub focus: Option<String>,
}

/// The starting values of a draft row opened from a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewDraft {
    /// The type to start with; `None` when `new=` names no type.
    pub account_type: Option<AccountType>,
    /// The parent to start under.
    pub parent_id: Option<String>,
}

impl EntryRequest {
    /// Reads the request from the `new`, `parent` and `focus` query values.
    ///
    /// Any `new=` opens a draft, an empty one included, typed when it names
    /// an account type. `parent=` applies only with `new=`. An empty
    /// `parent=` or `focus=` counts as absent.
    ///
    /// # Arguments
    ///
    /// * `new` - The `?new=` value.
    /// * `parent` - The `?parent=` value.
    /// * `focus` - The `?focus=` value.
    #[must_use]
    pub(crate) fn from_query(new: Option<&str>, parent: Option<&str>, focus: Option<&str>) -> Self {
        let present = |v: Option<&str>| v.filter(|s| !s.is_empty()).map(str::to_owned);
        Self {
            draft: new.map(|t| NewDraft {
                account_type: parse_account_type(t),
                parent_id: present(parent),
            }),
            focus: present(focus),
        }
    }
}

/// Parses an account type from its [`AccountType::label`].
#[must_use]
fn parse_account_type(label: &str) -> Option<AccountType> {
    [
        AccountType::Asset,
        AccountType::Liability,
        AccountType::Equity,
        AccountType::Income,
        AccountType::Expense,
    ]
    .into_iter()
    .find(|t| t.label() == label)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountType;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::EntryRequest;
    use super::NewDraft;
    use super::focus_account_href;
    use super::new_account_href;

    #[rstest]
    #[case(
        AccountType::Liability,
        None,
        "/settings?section=accounts&new=liability"
    )]
    #[case(
        AccountType::Asset,
        Some("acct 1"),
        "/settings?section=accounts&new=asset&parent=acct%201"
    )]
    fn new_account_hrefs(
        #[case] account_type: AccountType,
        #[case] parent: Option<&str>,
        #[case] expected: &str,
    ) {
        assert_eq!(new_account_href(account_type, parent), expected);
    }

    #[test]
    fn focus_href_encodes_the_id() {
        assert_eq!(
            focus_account_href("a/b"),
            "/settings?section=accounts&focus=a%2Fb"
        );
    }

    #[rstest]
    #[case(None, None, None, EntryRequest::default())]
    #[case(
        Some("expense"),
        Some("p1"),
        None,
        EntryRequest {
            draft: Some(NewDraft { account_type: Some(AccountType::Expense), parent_id: Some("p1".into()) }),
            focus: None,
        }
    )]
    #[case(
        Some("Expense"),
        Some(""),
        None,
        EntryRequest {
            draft: Some(NewDraft { account_type: None, parent_id: None }),
            focus: None,
        }
    )]
    #[case(None, Some("p1"), Some("a1"), EntryRequest { draft: None, focus: Some("a1".into()) })]
    #[case(None, None, Some(""), EntryRequest::default())]
    fn entry_requests(
        #[case] new: Option<&str>,
        #[case] parent: Option<&str>,
        #[case] focus: Option<&str>,
        #[case] expected: EntryRequest,
    ) {
        assert_eq!(EntryRequest::from_query(new, parent, focus), expected);
    }
}
