//! Internal AST for the Beancount file format.

use bc_sdk::Date;
use rust_decimal::Decimal;

/// A top-level directive in a Beancount file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Directive {
    /// A `YYYY-MM-DD * "Payee" "Narration"` transaction.
    Transaction(Transaction),
    /// An `open <Account> <Currency>` directive.
    Open {
        /// The date on which the account was opened.
        date: Date,
        /// The colon-separated account path (e.g. `"Assets:Bank"`).
        account: String,
        /// The optional currency constraint for this account.
        currency: Option<String>,
    },
    /// A `close <Account>` directive.
    Close {
        /// The date on which the account was closed.
        date: Date,
        /// The colon-separated account path.
        account: String,
    },
    /// A `commodity <Code>` directive.
    Commodity {
        /// The date from which this commodity is valid.
        date: Date,
        /// The commodity code (e.g. `"AUD"`).
        code: String,
    },
    /// A `balance <Account> <Amount> <Currency>` assertion.
    Balance {
        /// The date of the balance assertion.
        date: Date,
        /// The account being asserted.
        account: String,
        /// The asserted balance amount.
        amount: Decimal,
        /// The commodity code of the asserted balance.
        currency: String,
    },
    /// A Fava `custom "budget"` directive.
    Budget(Budget),
    /// A `custom "budget"` line that does not read as a budget.
    ///
    /// Carried rather than failing the parse: the importer only wants
    /// transactions, so it warns and continues, while `budgets()` turns the
    /// carrier into an error.
    MalformedBudget {
        /// 1-based source line number of the directive.
        line: usize,
        /// What was wrong with the line.
        reason: String,
    },
    /// An `include "path"` directive naming another file to splice in.
    Include {
        /// The path exactly as written in the source, still unresolved.
        path: String,
        /// 1-based source line number of the directive.
        line: usize,
    },
    /// A directive whose leading keyword the parser does not recognise.
    ///
    /// Carried rather than discarded so the importer can warn about it: a
    /// silently dropped directive is indistinguishable from an absent one.
    Unknown {
        /// The unrecognised keyword as written.
        keyword: String,
        /// 1-based source line number of the directive.
        line: usize,
    },
    /// Any other directive or comment (skipped by the importer).
    Other,
}

/// A typed value on a `key: value` metadata line.
///
/// Beancount's own value grammar is wider than the seven types the host
/// holds. A currency code and a `#`-prefixed tag each land as text, which is
/// what they are once they leave beancount's own semantics.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MetaValue {
    /// A quoted string, a currency code, a tag, or anything unrecognised.
    Text(String),
    /// A bare decimal.
    Number(Decimal),
    /// `TRUE` or `FALSE`.
    Boolean(bool),
    /// A `YYYY-MM-DD` date.
    Date(Date),
    /// A decimal paired with a currency code.
    Amount(PostingAmount),
    /// A colon-separated account path.
    Account(String),
}

/// One `key: value` metadata line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MetaEntry {
    /// The key, exactly as written.
    pub key: String,
    /// The typed value.
    pub value: MetaValue,
}

/// A Beancount transaction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Transaction {
    /// The transaction date.
    pub date: Date,
    /// The transaction flag.
    pub flag: TxFlag,
    /// Payee string (first quoted string if two are present; absent if only one).
    pub payee: Option<String>,
    /// Narration (second quoted string, or the only one if there is just one).
    pub narration: String,
    /// The `#`-prefixed tags on the transaction header, in source order.
    pub tags: Vec<String>,
    /// The posting legs for this transaction.
    pub postings: Vec<Posting>,
    /// The transaction's own `key: value` metadata lines, in source order.
    pub metadata: Vec<MetaEntry>,
    /// 1-based source line number of the transaction's header line.
    pub line: usize,
}

/// A Fava `YYYY-MM-DD custom "budget" <Account> "<period>" <amount>
/// <Currency>` line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Budget {
    /// The directive date.
    pub date: Date,
    /// The colon-separated account path.
    pub account: String,
    /// The period word.
    pub period: BudgetPeriod,
    /// The evaluated amount.
    pub amount: Decimal,
    /// The commodity code.
    pub currency: String,
    /// 1-based source line number.
    pub line: usize,
}

/// The period word of a Fava budget directive, in Fava's own vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetPeriod {
    /// `"daily"`.
    Daily,
    /// `"weekly"`.
    Weekly,
    /// `"monthly"`.
    Monthly,
    /// `"quarterly"`.
    Quarterly,
    /// `"yearly"`.
    Yearly,
}

impl BudgetPeriod {
    /// Parses one of Fava's period words.
    ///
    /// # Arguments
    ///
    /// * `word` - The quoted period text, without its quotes.
    ///
    /// # Returns
    ///
    /// The matching variant, or `None` if `word` is not one of Fava's five
    /// period words.
    pub(crate) fn parse(word: &str) -> Option<Self> {
        match word {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "quarterly" => Some(Self::Quarterly),
            "yearly" => Some(Self::Yearly),
            _ => None,
        }
    }
}

/// The flag on a Beancount transaction header line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxFlag {
    /// `*` — complete.
    Complete,
    /// `!` — incomplete.
    Incomplete,
}

/// A single posting leg in a Beancount transaction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Posting {
    /// The account path (e.g. `"Assets:Bank"`).
    pub account: String,
    /// The explicit amount, or `None` if the posting elides it (Beancount
    /// derives the elided amount so the transaction balances).
    pub amount: Option<PostingAmount>,
    /// This leg's own `key: value` metadata lines, in source order.
    pub metadata: Vec<MetaEntry>,
}

/// An explicit numeric amount and commodity on a posting leg.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PostingAmount {
    /// The numeric value.
    pub value: Decimal,
    /// The commodity code (e.g. `"AUD"`).
    pub currency: String,
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[rstest::rstest]
    #[case("daily", Some(BudgetPeriod::Daily))]
    #[case("yearly", Some(BudgetPeriod::Yearly))]
    #[case("fortnightly", None)]
    fn budget_period_parses_fava_words(#[case] word: &str, #[case] expected: Option<BudgetPeriod>) {
        assert_eq!(BudgetPeriod::parse(word), expected);
    }
}
