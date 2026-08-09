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
    /// 1-based source line number of the transaction's header line.
    pub line: usize,
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
}

/// An explicit numeric amount and commodity on a posting leg.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PostingAmount {
    /// The numeric value.
    pub value: Decimal,
    /// The commodity code (e.g. `"AUD"`).
    pub currency: String,
}
