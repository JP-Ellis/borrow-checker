//! Internal AST for the Beancount file format.

use jiff::civil::Date;
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
    /// Any other directive or comment (skipped by the importer).
    Other,
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
    /// The posting legs for this transaction.
    pub postings: Vec<Posting>,
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
    /// The numeric amount.
    pub amount: Decimal,
    /// The commodity code (e.g. `"AUD"`).
    pub currency: String,
}
