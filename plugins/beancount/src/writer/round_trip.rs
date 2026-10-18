//! Property test: the importer reads back what the working-level writer wrote.

#![cfg_attr(coverage_nightly, coverage(off))]

use bc_sdk::Amount;
use bc_sdk::Cost;
use bc_sdk::Date;
use bc_sdk::Directive;
use bc_sdk::ExportLevel;
use bc_sdk::ExportPosting;
use bc_sdk::ExportSnapshot;
use bc_sdk::ExportTransaction;
use bc_sdk::MetaEntry;
use bc_sdk::MetaValue;
use bc_sdk::NativePosting;
use bc_sdk::NativeTransaction;
use bc_sdk::Quote;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use bc_sdk::Reconciliation;
use proptest::collection::btree_set;
use proptest::collection::vec;
use proptest::prelude::*;
use rust_decimal::Decimal;

use crate::parse_text;
use crate::writer::write;

// MARK: Strategies

/// Text with quotes, backslashes, non-ASCII characters, line breaks and the
/// punctuation the line reader treats specially.
fn text() -> impl Strategy<Value = String> {
    const POOL: &[char] = &[
        'a', 'Z', '7', ' ', ' ', '"', '\\', 'é', '日', '🦀', ';', '#', '-', ':', ',', '{', '}',
        '@', '\n', '\r',
    ];
    vec(proptest::sample::select(POOL), 0..10).prop_map(|chars| chars.into_iter().collect())
}

/// A cost label the importer's cost-block reader keeps intact.
fn label() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 \"\\\\é日\r\n-]{0,8}"
}

/// A decimal of scale 0 to 4.
fn decimal() -> impl Strategy<Value = Decimal> {
    (-1_000_000_000_i64..1_000_000_000, 0_u32..5)
        .prop_map(|(mantissa, scale)| Decimal::new(mantissa, scale))
}

/// A non-negative decimal, for prices and costs.
fn magnitude() -> impl Strategy<Value = Decimal> {
    (0_i64..1_000_000_000, 0_u32..5).prop_map(|(mantissa, scale)| Decimal::new(mantissa, scale))
}

fn date() -> impl Strategy<Value = Date> {
    (1900_i32..2100, 1_u8..=12, 1_u8..=28).prop_map(|(y, m, d)| Date::new(y, m, d))
}

/// A commodity of two or more characters.
fn commodity() -> impl Strategy<Value = String> {
    "[A-Z][A-Z0-9'._-]{0,5}[A-Z0-9]"
}

fn account() -> impl Strategy<Value = String> {
    vec("[A-Z0-9][A-Za-z0-9-]{0,6}", 1..4).prop_map(|segments| segments.join(":"))
}

/// An account path the reader types as an account: it holds a `:` and starts
/// with a capital letter.
fn colon_account() -> impl Strategy<Value = String> {
    (
        "[A-Z][A-Za-z0-9-]{0,6}",
        vec("[A-Z0-9][A-Za-z0-9-]{0,6}", 1..3),
    )
        .prop_map(|(root, rest)| format!("{root}:{}", rest.join(":")))
}

fn tag() -> impl Strategy<Value = String> {
    "[A-Za-z0-9_/.-]{1,8}"
}

fn amount() -> impl Strategy<Value = Amount> {
    (decimal(), commodity()).prop_map(|(value, code)| Amount::new(value, code))
}

fn quote() -> impl Strategy<Value = Quote> {
    (magnitude(), commodity(), any::<bool>()).prop_map(|(value, code, total)| {
        let figure = Amount::new(value, code);
        if total {
            Quote::Total(figure)
        } else {
            Quote::PerUnit(figure)
        }
    })
}

fn cost() -> impl Strategy<Value = Cost> {
    (
        quote(),
        proptest::option::of(date()),
        proptest::option::of(label()),
    )
        .prop_map(|(basis, date, label)| {
            Cost::builder()
                .basis(basis)
                .maybe_date(date)
                .maybe_label(label)
                .build()
        })
}

fn meta_value() -> impl Strategy<Value = MetaValue> {
    prop_oneof![
        text().prop_map(MetaValue::Text),
        decimal().prop_map(MetaValue::Number),
        any::<bool>().prop_map(MetaValue::Boolean),
        date().prop_map(MetaValue::Date),
        amount().prop_map(MetaValue::Amount),
        colon_account().prop_map(MetaValue::Account),
    ]
}

/// Metadata with unique keys that are neither `bc` nor start `bc-`.
fn metadata() -> impl Strategy<Value = Vec<MetaEntry>> {
    btree_set("[a-z][a-z0-9_-]{0,6}", 0..5)
        .prop_map(|keys| {
            keys.into_iter()
                .filter(|key| key != "bc" && !key.starts_with("bc-"))
                .collect::<Vec<_>>()
        })
        .prop_flat_map(|keys| {
            let count = keys.len();
            vec(meta_value(), count).prop_map(move |values| {
                keys.iter()
                    .cloned()
                    .zip(values)
                    .map(|(key, value)| MetaEntry::new(key, value))
                    .collect()
            })
        })
}

/// A leg's amount and the annotations that depend on it.
fn leg_money() -> impl Strategy<Value = (Option<Amount>, Option<Quote>, Option<Cost>)> {
    proptest::option::of((
        amount(),
        proptest::option::of(quote()),
        proptest::option::of(cost()),
    ))
    .prop_map(|leg| match leg {
        Some((amount, price, cost)) => (Some(amount), price, cost),
        None => (None, None, None),
    })
}

fn posting() -> impl Strategy<Value = ExportPosting> {
    (
        proptest::option::of(text()),
        account(),
        leg_money(),
        vec(tag(), 0..3),
        metadata(),
        proptest::option::of(date()),
        proptest::option::of(date()),
    )
        .prop_map(
            |(handle, account, (amount, price, cost), tags, metadata, from, until)| {
                ExportPosting::builder()
                    .maybe_handle(handle)
                    .account(account)
                    .maybe_amount(amount)
                    .maybe_price(price)
                    .maybe_cost(cost)
                    .tags(tags)
                    .metadata(metadata)
                    .maybe_spread_from(from)
                    .maybe_spread_until(until)
                    .build()
            },
        )
}

fn reconciliation() -> impl Strategy<Value = Reconciliation> {
    prop_oneof![
        Just(Reconciliation::Unreconciled),
        Just(Reconciliation::Flagged),
        Just(Reconciliation::Reconciled),
    ]
}

fn transaction() -> impl Strategy<Value = ExportTransaction> {
    (
        text(),
        date(),
        text(),
        reconciliation(),
        vec(tag(), 0..3),
        proptest::option::of(text()),
        metadata(),
        vec(posting(), 1..4),
    )
        .prop_map(
            |(handle, date, description, reconciliation, tags, payee, mut metadata, postings)| {
                if let Some(name) = payee
                    && !metadata.iter().any(|entry| entry.key == "payee")
                {
                    metadata.insert(0, MetaEntry::text("payee", name));
                }
                ExportTransaction::builder()
                    .handle(handle)
                    .date(date)
                    .description(description)
                    .reconciliation(reconciliation)
                    .tags(tags)
                    .metadata(metadata)
                    .postings(postings)
                    .build()
            },
        )
}

// MARK: Expectation

/// `text` as the writer spells it: each line break becomes a space.
fn flat(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// `entries` with every text value flattened.
fn flat_meta(entries: &[MetaEntry]) -> Vec<MetaEntry> {
    entries
        .iter()
        .map(|entry| match entry.value {
            MetaValue::Text(ref text) => MetaEntry::text(entry.key.clone(), flat(text)),
            _ => entry.clone(),
        })
        .collect()
}

/// `cost` with its label flattened.
fn flat_cost(cost: &Cost) -> Cost {
    Cost::builder()
        .basis(cost.basis.clone())
        .maybe_date(cost.date.clone())
        .maybe_label(cost.label.as_deref().map(flat))
        .build()
}

/// What the importer should read back for `leg`.
fn expected_posting(leg: &ExportPosting) -> RawPosting {
    let carries_native = leg.handle.is_some()
        || !leg.tags.is_empty()
        || leg.spread_from.is_some()
        || leg.spread_until.is_some();
    let native = carries_native.then(|| {
        NativePosting::builder()
            .maybe_handle(leg.handle.as_deref().map(flat))
            .maybe_spread_from(leg.spread_from.clone())
            .maybe_spread_until(leg.spread_until.clone())
            .build()
    });
    RawPosting::builder()
        .account(leg.account.clone())
        .maybe_amount(leg.amount.clone())
        .maybe_price(leg.price.clone())
        .maybe_cost(leg.cost.as_ref().map(flat_cost))
        .tags(leg.tags.clone())
        .metadata(flat_meta(&leg.metadata))
        .maybe_native(native)
        .build()
}

/// What the importer should read back for `tx`, with no source location.
fn expected_transaction(tx: &ExportTransaction) -> RawTransaction {
    RawTransaction::builder()
        .date(tx.date.clone())
        .description(flat(&tx.description))
        .tags(tx.tags.clone())
        .metadata(flat_meta(&tx.metadata))
        .postings(tx.postings.iter().map(expected_posting).collect())
        .native(
            NativeTransaction::builder()
                .handle(flat(&tx.handle))
                .reconciliation(tx.reconciliation)
                .build(),
        )
        .build()
}

// MARK: Property

proptest! {
    #[test]
    fn write_then_parse_is_identity(
        transactions in vec(transaction(), 0..4)
    ) {
        let snapshot = ExportSnapshot::builder()
            .level(ExportLevel::Working)
            .transactions(transactions.clone())
            .build();
        let files = write(&snapshot).expect("generated names are spellable");
        let contents = &files.first().expect("one file").contents;

        let parsed = parse_text(contents).expect("written text parses");
        let actual: Vec<Directive> = parsed
            .into_iter()
            .map(|directive| match directive {
                Directive::Transaction(mut tx) => {
                    tx.source_location = None;
                    Directive::Transaction(tx)
                }
                other => other,
            })
            .collect();
        let expected: Vec<Directive> = transactions
            .iter()
            .map(|tx| Directive::from(expected_transaction(tx)))
            .collect();

        // `Decimal` equality ignores scale, so compare the printed form too.
        prop_assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "text:\n{}", contents);
        prop_assert_eq!(actual, expected);
    }
}
