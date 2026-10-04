//! The typed query that `resolve` produces and the engine runs.

use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;

/// A typed query, evaluated per leg (spec §2).
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches ResolvedExpr exhaustively"
)]
pub enum ResolvedExpr {
    /// Holds when any alternative does.
    Or(Vec<ResolvedExpr>),
    /// Holds when every conjunct does.
    And(Vec<ResolvedExpr>),
    /// Holds when the inner expression does not, on this leg.
    Not(Box<ResolvedExpr>),
    /// Holds on every leg when some leg of the transaction satisfies the
    /// inner expression.
    Any(Box<ResolvedExpr>),
    /// A single predicate.
    Pred(Pred),
}

/// One typed predicate.
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches Pred exhaustively"
)]
pub enum Pred {
    /// The transaction description.
    Description(TextMatch),
    /// The leg's account.
    Account {
        /// The account id.
        id: String,
        /// Whether descendants also match.
        subtree: bool,
    },
    /// A tag on the leg or its transaction.
    Tag(TagPred),
    /// The transaction's reconciliation or balance status.
    Status(Status),
    /// The transaction date.
    Date(DateRange),
    /// The leg's amount magnitude.
    Amount(AmountPred),
    /// The leg's commodity code.
    Commodity(String),
    /// A metadata value on the leg or its transaction.
    Meta {
        /// The normalised key.
        key: String,
        /// What its value must satisfy.
        pred: MetaPred,
    },
}

/// A case-insensitive text test; the needle is ASCII-lowercased.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches TextMatch exhaustively"
)]
pub enum TextMatch {
    /// The text contains the needle.
    Contains(String),
    /// The text equals the needle.
    Equals(String),
}

/// A tag test.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches TagPred exhaustively"
)]
pub enum TagPred {
    /// Any tag at all.
    Any,
    /// One tag.
    Tag {
        /// The tag id.
        id: String,
        /// Whether descendant tags also match.
        subtree: bool,
    },
}

/// A status word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches Status exhaustively"
)]
pub enum Status {
    /// Not confirmed against a statement.
    Unreconciled,
    /// Flagged for attention.
    Flagged,
    /// Confirmed against a statement.
    Reconciled,
    /// Weights sum to zero.
    Balanced,
    /// Weights do not sum to zero.
    Unbalanced,
}

impl Status {
    /// Whether this is a reconciliation state (the rest are balance states).
    #[must_use]
    pub const fn is_reconciliation(self) -> bool {
        matches!(self, Self::Unreconciled | Self::Flagged | Self::Reconciled)
    }
}

/// One end of a numeric range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bound {
    /// The limit.
    pub value: Decimal,
    /// Whether the limit itself is inside the range.
    pub inclusive: bool,
}

impl Bound {
    /// Creates a bound.
    ///
    /// # Arguments
    ///
    /// * `value` - The limit.
    /// * `inclusive` - Whether the limit is inside the range.
    #[must_use]
    pub const fn new(value: Decimal, inclusive: bool) -> Self {
        Self { value, inclusive }
    }
}

/// A decimal range; a missing end is unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NumRange {
    /// The lower end.
    pub lo: Option<Bound>,
    /// The upper end.
    pub hi: Option<Bound>,
}

impl NumRange {
    /// Creates a range.
    ///
    /// # Arguments
    ///
    /// * `lo` - The lower end.
    /// * `hi` - The upper end.
    #[must_use]
    pub const fn new(lo: Option<Bound>, hi: Option<Bound>) -> Self {
        Self { lo, hi }
    }

    /// Whether `value` lies in the range, compared exactly.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to test.
    #[must_use]
    pub fn contains(&self, value: Decimal) -> bool {
        let above = self.lo.is_none_or(|b| {
            if b.inclusive {
                value >= b.value
            } else {
                value > b.value
            }
        });
        let below = self.hi.is_none_or(|b| {
            if b.inclusive {
                value <= b.value
            } else {
                value < b.value
            }
        });
        above && below
    }

    /// Whether no value can lie in the range.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match (self.lo, self.hi) {
            (Some(lo), Some(hi)) => {
                lo.value > hi.value || (lo.value == hi.value && !(lo.inclusive && hi.inclusive))
            }
            _ => false,
        }
    }
}

/// An amount test: a range plus an optional commodity.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmountPred {
    /// The range the value must fall in.
    pub range: NumRange,
    /// The commodity the amount must be in, when a marker was given.
    pub commodity: Option<String>,
}

impl AmountPred {
    /// Creates an amount test.
    ///
    /// # Arguments
    ///
    /// * `range` - The range the value must fall in.
    /// * `commodity` - The commodity the amount must be in, if any.
    #[must_use]
    pub const fn new(range: NumRange, commodity: Option<String>) -> Self {
        Self { range, commodity }
    }
}

/// Dates in `[from, until)`; a missing end is unbounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DateRange {
    /// The first included date.
    pub from: Option<Date>,
    /// The first excluded date.
    pub until: Option<Date>,
}

impl DateRange {
    /// Creates a date range.
    ///
    /// # Arguments
    ///
    /// * `from` - The first included date.
    /// * `until` - The first excluded date.
    #[must_use]
    pub const fn new(from: Option<Date>, until: Option<Date>) -> Self {
        Self { from, until }
    }
}

/// Instants in `[from, until)`; a missing end is unbounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimeRange {
    /// The first included instant.
    pub from: Option<Timestamp>,
    /// The first excluded instant.
    pub until: Option<Timestamp>,
}

impl TimeRange {
    /// Creates a time range.
    ///
    /// # Arguments
    ///
    /// * `from` - The first included instant.
    /// * `until` - The first excluded instant.
    #[must_use]
    pub const fn new(from: Option<Timestamp>, until: Option<Timestamp>) -> Self {
        Self { from, until }
    }
}

/// A metadata value test, by the key's registered type.
#[derive(Clone, Debug, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches MetaPred exhaustively"
)]
pub enum MetaPred {
    /// The key is present, whatever its value.
    Exists,
    /// A text key.
    Text(TextMatch),
    /// A number key; signed.
    Number(NumRange),
    /// An amount key; signed.
    Amount(AmountPred),
    /// A boolean key.
    Boolean(bool),
    /// A date key.
    Date(DateRange),
    /// A timestamp key.
    Timestamp(TimeRange),
    /// An account key, matched on its stored path.
    Account {
        /// The path segments.
        path: Vec<String>,
        /// Whether paths beneath also match.
        subtree: bool,
    },
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    /// `[lo, hi]` with inclusivity flags.
    fn range(lo: Option<(Decimal, bool)>, hi: Option<(Decimal, bool)>) -> NumRange {
        NumRange::new(
            lo.map(|(v, i)| Bound::new(v, i)),
            hi.map(|(v, i)| Bound::new(v, i)),
        )
    }

    #[rstest]
    #[case(range(Some((dec!(100), true)), None), dec!(100), true)]
    #[case(range(Some((dec!(100), false)), None), dec!(100), false)]
    #[case(range(None, Some((dec!(20), true))), dec!(20.00), true)]
    #[case(range(None, Some((dec!(20), false))), dec!(19.99), true)]
    #[case(range(Some((dec!(150), true)), Some((dec!(150), true))), dec!(150.00), true)]
    #[case(range(None, None), dec!(-5), true)]
    fn contains(#[case] r: NumRange, #[case] value: Decimal, #[case] expected: bool) {
        assert_eq!(r.contains(value), expected);
    }

    #[rstest]
    #[case(range(Some((dec!(200), true)), Some((dec!(100), true))), true)]
    #[case(range(Some((dec!(100), true)), Some((dec!(100), false))), true)]
    #[case(range(Some((dec!(100), true)), Some((dec!(100), true))), false)]
    #[case(range(Some((dec!(100), true)), None), false)]
    fn is_empty(#[case] r: NumRange, #[case] expected: bool) {
        assert_eq!(r.is_empty(), expected);
    }
}
