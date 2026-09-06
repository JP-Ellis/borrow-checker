//! Advisory warnings raised by write paths that accept their input anyway.
//!
//! The project's governing principle is "warn, don't block": guardrails inform
//! rather than gatekeep, and hard errors are reserved for genuinely
//! unrepresentable states. A [`Warning`] is what that principle produces — the
//! write happened, and something about it is worth saying.

use bc_models::AccountId;
use jiff::civil::Date;

/// Something worth telling the user about a write that nonetheless succeeded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Warning {
    /// A posting's commodity is outside its account's non-empty declared list.
    ///
    /// Compared by code, not by id: a [`bc_models::Amount`] carries a
    /// [`bc_models::CommodityCode`] and no id, so the account's declared ids are
    /// resolved to codes to compare. `commodities.code` is not unique across
    /// exchanges, so an account declaring one exchange's BTC accepts a posting
    /// coded `BTC` from any exchange. The posting carries nothing finer, so no
    /// stricter comparison is available.
    CommodityOutsideAccountList {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
        /// The commodity code the posting used.
        commodity_code: String,
    },
    /// A transaction dated before its account's declared opening date.
    PostingBeforeAccountOpened {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
        /// The transaction's value date.
        date: Date,
        /// The account's declared opening date.
        opened_on: Date,
    },
    /// A transaction dated after its account's declared closing date.
    PostingAfterAccountClosed {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
        /// The transaction's value date.
        date: Date,
        /// The account's declared closing date.
        closed_on: Date,
    },
    /// A posting written into an archived account.
    PostingIntoArchivedAccount {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
    },
}

impl std::fmt::Display for Warning {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::CommodityOutsideAccountList {
                ref account_path,
                ref commodity_code,
                ..
            } => write!(
                f,
                "{account_path} does not declare {commodity_code} among the commodities it holds"
            ),
            Self::PostingBeforeAccountOpened {
                ref account_path,
                date,
                opened_on,
                ..
            } => write!(
                f,
                "{account_path} is dated {date} but the account opened on {opened_on}"
            ),
            Self::PostingAfterAccountClosed {
                ref account_path,
                date,
                closed_on,
                ..
            } => write!(
                f,
                "{account_path} is dated {date} but the account closed on {closed_on}"
            ),
            Self::PostingIntoArchivedAccount {
                ref account_path, ..
            } => write!(f, "{account_path} is archived"),
        }
    }
}

/// A value paired with the warnings raised while producing it.
///
/// The write succeeded either way; `warnings` is advisory.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Warned<T> {
    /// The value produced.
    pub value: T,
    /// Warnings raised while producing it. Empty is the common case.
    pub warnings: Vec<Warning>,
}

impl<T> Warned<T> {
    /// Pairs a value with its warnings.
    #[inline]
    #[must_use]
    pub const fn new(value: T, warnings: Vec<Warning>) -> Self {
        Self { value, warnings }
    }

    /// Wraps a value that raised no warnings.
    #[inline]
    #[must_use]
    pub const fn clean(value: T) -> Self {
        Self {
            value,
            warnings: Vec::new(),
        }
    }

    /// Discards the warnings and returns the value.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> T {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    use super::Warned;
    use super::Warning;

    #[test]
    fn clean_carries_no_warnings() {
        let warned = Warned::clean(7_u32);
        assert_eq!(warned.value, 7);
        assert!(warned.warnings.is_empty());
    }

    #[test]
    fn into_inner_discards_warnings() {
        let warned = Warned::new(
            7_u32,
            vec![Warning::PostingIntoArchivedAccount {
                account_id: bc_models::AccountId::new(),
                account_path: "Assets:BankA:Checking".to_owned(),
            }],
        );
        assert_eq!(warned.into_inner(), 7);
    }

    #[test]
    fn warning_display_names_the_account_and_the_dates() {
        let warning = Warning::PostingBeforeAccountOpened {
            account_id: bc_models::AccountId::new(),
            account_path: "Assets:BankA:Checking".to_owned(),
            date: date(2019, 5, 1),
            opened_on: date(2020, 1, 1),
        };
        let rendered = warning.to_string();
        assert!(rendered.contains("Assets:BankA:Checking"), "{rendered}");
        assert!(rendered.contains("2019-05-01"), "{rendered}");
        assert!(rendered.contains("2020-01-01"), "{rendered}");
    }
}
