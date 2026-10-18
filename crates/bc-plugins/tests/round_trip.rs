//! End-to-end property: a working export, one edit to its text, and an apply
//! change exactly that edit in the database.
//!
//! Each case seeds a fresh database with generated transactions covering
//! every spelled feature: costs, prices, elided legs, spreads, posting tags,
//! each metadata value type that Beancount spells, the three reconciliation
//! states, and line breaks in the description, text values and cost labels,
//! which the writer flattens. Every transaction the edit does not touch must
//! apply as a skip. The beancount plugin is loaded once for every case.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]
#![expect(clippy::expect_used, reason = "test code — panics are acceptable")]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use bc_core::AccountPath;
use bc_core::AccountService;
use bc_core::CommodityService;
use bc_core::ExportLevel;
use bc_core::Exporter;
use bc_core::ImportConfig;
use bc_core::Importer;
use bc_core::MetadataService;
use bc_core::PathSpec;
use bc_core::TagService;
use bc_core::TransactionService;
use bc_core::roundtrip::RoundTripService;
use bc_core::roundtrip::Sidecar;
use bc_core::search::TransactionQuery;
use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Commodity;
use bc_models::Cost;
use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaType;
use bc_models::MetaValue;
use bc_models::Metadata;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Quote;
use bc_models::Reconciliation;
use bc_models::TagId;
use bc_models::TagPath;
use bc_models::Transaction;
use bc_models::TransactionId;
use bc_plugins::PluginRegistry;
use jiff::Timestamp;
use jiff::civil::date;
use proptest::collection::btree_set;
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::Config;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestRunner;
use rust_decimal::Decimal;
use serde_json::Value;

// MARK: Ledger

/// The tags a transaction or leg may carry.
const TAGS: [&str; 4] = ["trip", "work", "home", "review"];

/// The metadata keys a transaction or leg may carry, one per spelled type.
const KEYS: [(&str, MetaType); 6] = [
    ("receipt", MetaType::Text),
    ("items", MetaType::Number),
    ("gst", MetaType::Boolean),
    ("due", MetaType::Date),
    ("budgeted", MetaType::Amount),
    ("owner", MetaType::Account),
];

/// The key the metadata edit adds.
const ADDED_KEY: &str = "note";

/// The accounts and tags every case books against.
struct Ledger {
    checking: AccountId,
    broker: AccountId,
    food: AccountId,
    placeholder: AccountId,
    tags: BTreeMap<&'static str, TagId>,
}

fn key(name: &str) -> MetaKey {
    MetaKey::new(name).expect("valid key")
}

async fn account(accounts: &AccountService, path: &str) -> AccountId {
    accounts
        .create_path(
            &PathSpec::builder()
                .path(AccountPath::parse(path).expect("path"))
                .build(),
        )
        .await
        .expect("account")
}

/// Creates the accounts, the `VTS` commodity, the tags and the metadata keys.
async fn ledger(pool: &sqlx::SqlitePool) -> Ledger {
    let commodities = CommodityService::new(pool.clone());
    commodities.seed_defaults().await.expect("currencies");
    commodities
        .create(&Commodity::builder().code("VTS").decimals(4).build())
        .await
        .expect("commodity");
    let metadata = MetadataService::new(pool.clone());
    for (name, ty) in KEYS
        .into_iter()
        .chain([("payee", MetaType::Text), (ADDED_KEY, MetaType::Text)])
    {
        metadata.register(&key(name), ty).await.expect("register");
    }
    let tag_service = TagService::new(pool.clone());
    let mut tags = BTreeMap::new();
    for name in TAGS {
        let path: TagPath = name.parse().expect("tag path");
        tags.insert(name, tag_service.create_path(&path).await.expect("tag"));
    }
    let accounts = AccountService::new(pool.clone());
    Ledger {
        checking: account(&accounts, "Assets:Bank:Checking").await,
        broker: account(&accounts, "Assets:Broker").await,
        food: account(&accounts, "Expenses:Food").await,
        placeholder: account(&accounts, "Expenses:Uncategorised").await,
        tags,
    }
}

// MARK: Generation

/// How a transaction's legs are built. Every shape books one leg to the
/// placeholder account.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// The bank leg is elided.
    Elided,
    /// The bank leg is in USD with a total price in AUD.
    Priced,
    /// A broker leg buys one unit at a per-unit cost with a lot date and label.
    Cost,
    /// The placeholder leg is spread over the year.
    Spread,
}

/// A generated transaction, independent of any database's IDs.
#[derive(Debug, Clone)]
struct Spec {
    shape: Shape,
    month: i8,
    day: i8,
    cents: i64,
    reconciliation: Reconciliation,
    payee: Option<&'static str>,
    tags: BTreeSet<usize>,
    metadata: BTreeSet<usize>,
    leg_tags: BTreeSet<usize>,
    leg_metadata: BTreeSet<usize>,
    other_tags: BTreeSet<usize>,
    other_metadata: BTreeSet<usize>,
    per_unit: bool,
    seed: i64,
}

/// One edit to the exported text.
#[derive(Debug, Clone, Copy)]
enum Edit {
    /// Appends a leg to `Expenses:Food`.
    AddLeg,
    /// Replaces the first transaction tag, or adds one when there is none.
    ChangeTag,
    /// Moves the placeholder leg to `Expenses:Food`.
    ChangePlaceholder,
    /// Adds a text metadata entry after the transaction's others.
    AddMetadata,
}

fn shape() -> impl Strategy<Value = Shape> {
    proptest::sample::select(SHAPES.to_vec())
}

fn reconciliation() -> impl Strategy<Value = Reconciliation> {
    prop_oneof![
        Just(Reconciliation::Unreconciled),
        Just(Reconciliation::Flagged),
        Just(Reconciliation::Reconciled),
    ]
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        (shape(), 1_i8..=12, 1_i8..=28, 100_i64..100_000),
        reconciliation(),
        proptest::option::of(proptest::sample::select(vec![
            "Woolworths",
            "Coles \"Local\"",
            "Café Ōtautahi",
        ])),
        (
            btree_set(0..TAGS.len(), 0..3),
            btree_set(0..KEYS.len(), 0..=KEYS.len()),
        ),
        (
            btree_set(0..TAGS.len(), 0..3),
            btree_set(0..KEYS.len(), 0..3),
        ),
        (
            btree_set(0..TAGS.len(), 0..3),
            btree_set(0..KEYS.len(), 0..3),
        ),
        any::<bool>(),
        1_i64..10_000,
    )
        .prop_map(
            |(
                (shape, month, day, cents),
                reconciliation,
                payee,
                (tags, metadata),
                (leg_tags, leg_metadata),
                (other_tags, other_metadata),
                per_unit,
                seed,
            )| Spec {
                shape,
                month,
                day,
                cents,
                reconciliation,
                payee,
                tags,
                metadata,
                leg_tags,
                leg_metadata,
                other_tags,
                other_metadata,
                per_unit,
                seed,
            },
        )
}

/// Every shape, each paired with every edit.
const SHAPES: [Shape; 4] = [Shape::Elided, Shape::Priced, Shape::Cost, Shape::Spread];

/// Every edit.
const EDITS: [Edit; 4] = [
    Edit::AddLeg,
    Edit::ChangeTag,
    Edit::ChangePlaceholder,
    Edit::AddMetadata,
];

fn aud(cents: i64) -> Amount {
    Amount::new(Decimal::new(cents, 2), "AUD")
}

/// `cents` hundredths of `code`, negated.
fn spent(cents: i64, code: &str) -> Amount {
    let mut value = Decimal::new(cents, 2);
    value.set_sign_negative(true);
    Amount::new(value, code)
}

/// The value the metadata key at `index` takes, varied by `seed`.
fn meta_value(index: usize, seed: i64, ledger: &Ledger) -> MetaValue {
    match KEYS.get(index).map(|&(_, ty)| ty) {
        Some(MetaType::Number) => MetaValue::Number(Decimal::new(seed, 2)),
        Some(MetaType::Boolean) => MetaValue::Boolean(seed > 5_000),
        Some(MetaType::Date) => MetaValue::Date(date(2026, 6, 1)),
        Some(MetaType::Amount) => MetaValue::Amount(aud(seed)),
        Some(MetaType::Account) => MetaValue::Account(ledger.checking.clone()),
        _ => MetaValue::Text(format!("INV-{seed}\r\n\"é\"")),
    }
}

fn metadata(indices: &BTreeSet<usize>, seed: i64, ledger: &Ledger) -> Vec<MetaEntry> {
    indices
        .iter()
        .map(|&index| {
            MetaEntry::new(
                key(KEYS.get(index).map_or("receipt", |&(name, _)| name)),
                meta_value(index, seed, ledger),
            )
        })
        .collect()
}

fn tag_ids(indices: &BTreeSet<usize>, ledger: &Ledger) -> Vec<TagId> {
    indices
        .iter()
        .filter_map(|&index| {
            let name = TAGS.get(index)?;
            ledger.tags.get(name)
        })
        .cloned()
        .collect()
}

/// The legs of the transaction `spec` describes in `ledger`.
fn postings(spec: &Spec, ledger: &Ledger) -> Vec<Posting> {
    let placeholder = |amount: Amount, spread: bool| {
        Posting::builder()
            .id(PostingId::new())
            .account_id(ledger.placeholder.clone())
            .amount(amount)
            .tag_ids(tag_ids(&spec.leg_tags, ledger))
            .metadata(Metadata::new(metadata(
                &spec.leg_metadata,
                spec.seed,
                ledger,
            )))
            .maybe_spread_from(spread.then(|| date(2026, 1, 1)))
            .maybe_spread_until(spread.then(|| date(2026, 12, 31)))
            .build()
    };
    // The leg that is not the placeholder, with tags and metadata of its own.
    let other =
        |account: &AccountId, amount: Option<Amount>, price: Option<Quote>, cost: Option<Cost>| {
            Posting::builder()
                .id(PostingId::new())
                .account_id(account.clone())
                .maybe_amount(amount)
                .maybe_price(price)
                .maybe_cost(cost)
                .tag_ids(tag_ids(&spec.other_tags, ledger))
                .metadata(Metadata::new(metadata(
                    &spec.other_metadata,
                    spec.seed.saturating_add(1),
                    ledger,
                )))
                .build()
        };
    let lot = |basis: Quote| {
        Cost::builder()
            .basis(basis)
            .date(date(2026, 1, 2))
            .label("lot\na")
            .build()
    };
    match (spec.shape, spec.per_unit) {
        (Shape::Elided, _) => vec![
            placeholder(aud(spec.cents), false),
            other(&ledger.checking, None, None, None),
        ],
        (Shape::Priced, true) => vec![
            placeholder(aud(spec.cents.saturating_mul(2)), false),
            other(
                &ledger.checking,
                Some(spent(200, "USD")),
                Some(Quote::PerUnit(aud(spec.cents))),
                None,
            ),
        ],
        (Shape::Priced, false) => vec![
            placeholder(aud(spec.cents), false),
            other(
                &ledger.checking,
                Some(spent(spec.seed, "USD")),
                Some(Quote::Total(aud(spec.cents))),
                None,
            ),
        ],
        (Shape::Cost, true) => vec![
            other(
                &ledger.broker,
                Some(Amount::new(Decimal::ONE, "VTS")),
                None,
                Some(lot(Quote::PerUnit(aud(spec.cents)))),
            ),
            placeholder(spent(spec.cents, "AUD"), false),
        ],
        (Shape::Cost, false) => vec![
            other(
                &ledger.broker,
                Some(Amount::new(Decimal::TWO, "VTS")),
                None,
                Some(lot(Quote::Total(aud(spec.cents)))),
            ),
            placeholder(spent(spec.cents, "AUD"), false),
        ],
        (Shape::Spread, _) => vec![
            placeholder(aud(spec.cents), true),
            other(&ledger.checking, Some(spent(spec.cents, "AUD")), None, None),
        ],
    }
}

/// Builds the transaction `spec` describes in `ledger`.
fn transaction(spec: &Spec, ledger: &Ledger) -> Transaction {
    let mut entries = Vec::new();
    if let Some(payee) = spec.payee {
        entries.push(MetaEntry::new(
            key("payee"),
            MetaValue::Text(payee.to_owned()),
        ));
    }
    entries.extend(metadata(&spec.metadata, spec.seed, ledger));
    Transaction::builder()
        .id(TransactionId::new())
        .date(date(2026, spec.month, spec.day))
        .description(format!("Purchase\n{}", spec.seed))
        .postings(postings(spec, ledger))
        .reconciliation(spec.reconciliation)
        .tag_ids(tag_ids(&spec.tags, ledger))
        .metadata(Metadata::new(entries))
        .created_at(Timestamp::now())
        .build()
}

// MARK: Editing

/// Applies `edit` to the block of `text` holding `handle`.
fn edit_text(text: &str, handle: &str, edit: Edit, tags: &[&str]) -> String {
    let marker = format!("  bc: \"{handle}\"");
    let blocks: Vec<String> = text
        .split("\n\n")
        .map(|block| {
            if block.lines().any(|line| line == marker) {
                edit_block(block, edit, tags)
            } else {
                block.to_owned()
            }
        })
        .collect();
    blocks.join("\n\n")
}

/// Whether `line` is a posting line: two spaces, then an account.
fn is_posting(line: &str) -> bool {
    line.strip_prefix("  ")
        .and_then(|rest| rest.chars().next())
        .is_some_and(char::is_uppercase)
}

fn edit_block(block: &str, edit: Edit, tags: &[&str]) -> String {
    let mut lines: Vec<String> = block.lines().map(str::to_owned).collect();
    match edit {
        Edit::AddLeg => {
            lines.push("  Expenses:Food  1.00 AUD".to_owned());
        }
        Edit::ChangeTag => {
            let header = lines.first_mut().expect("header");
            let replacement = TAGS
                .iter()
                .find(|tag| !tags.contains(tag))
                .expect("a free tag");
            if let Some(first) = tags.first() {
                *header = header.replacen(&format!(" #{first}"), &format!(" #{replacement}"), 1);
            } else {
                header.push_str(" #");
                header.push_str(replacement);
            }
        }
        Edit::ChangePlaceholder => {
            let line = lines
                .iter_mut()
                .find(|line| line.starts_with("  Expenses:Uncategorised"))
                .expect("a placeholder leg");
            *line = line.replacen("Expenses:Uncategorised", "Expenses:Food", 1);
        }
        Edit::AddMetadata => {
            let at = lines
                .iter()
                .position(|line| is_posting(line))
                .expect("a posting");
            lines.insert(at, format!("  {ADDED_KEY}: \"added\""));
        }
    }
    let mut edited = lines.join("\n");
    if block.ends_with('\n') {
        edited.push('\n');
    }
    edited
}

// MARK: Comparison

/// Every stored transaction, by ID, as JSON.
async fn dump(transactions: &TransactionService) -> BTreeMap<String, Value> {
    transactions
        .list()
        .await
        .expect("list")
        .into_iter()
        .map(|tx| {
            (
                tx.id().to_string(),
                serde_json::to_value(&tx).expect("serialise"),
            )
        })
        .collect()
}

fn field<'a>(tx: &'a Value, name: &str) -> &'a Value {
    tx.get(name).expect("field present")
}

/// `tx` without the named fields.
fn without(tx: &Value, names: &[&str]) -> Value {
    let mut copy = tx.clone();
    if let Some(object) = copy.as_object_mut() {
        for name in names {
            object.remove(*name);
        }
    }
    copy
}

/// The JSON of `value`.
fn json(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serialise")
}

/// A failed case, for a value of the wrong shape.
fn fail(what: &str) -> TestCaseError {
    TestCaseError::fail(format!("unexpected shape: {what}"))
}

/// Reads `value` as a `T`.
fn parse<T>(value: &Value) -> Result<T, TestCaseError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value.clone()).map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Checks that `after` is `before` with exactly `edit` applied, and flagged.
fn check_edited(
    before: &Value,
    after: &Value,
    edit: Edit,
    ledger: &Ledger,
    text: &str,
) -> Result<(), TestCaseError> {
    let name = match edit {
        Edit::AddLeg | Edit::ChangePlaceholder => "postings",
        Edit::ChangeTag => "tag_ids",
        Edit::AddMetadata => "metadata",
    };
    prop_assert_eq!(
        without(after, &[name, "reconciliation"]),
        without(before, &[name, "reconciliation"]),
        "only {} changed; file:\n{}",
        name,
        text
    );
    prop_assert_eq!(
        field(after, "reconciliation"),
        &json(&Reconciliation::Flagged)
    );

    let was = field(before, name);
    let now = field(after, name);
    match edit {
        Edit::AddLeg => {
            let kept = was.as_array().ok_or_else(|| fail("postings"))?;
            let grown = now.as_array().ok_or_else(|| fail("postings"))?;
            let (added, rest) = grown.split_last().ok_or_else(|| fail("no legs"))?;
            prop_assert_eq!(rest, kept.as_slice(), "file:\n{}", text);
            prop_assert_eq!(field(added, "account_id"), &json(&ledger.food));
            prop_assert_eq!(field(added, "amount"), &json(&aud(100)));
        }
        Edit::ChangePlaceholder => {
            let mut expected = was.clone();
            let moved = expected
                .as_array_mut()
                .ok_or_else(|| fail("postings"))?
                .iter_mut()
                .find(|leg| field(leg, "account_id") == &json(&ledger.placeholder))
                .ok_or_else(|| fail("no placeholder leg"))?;
            moved["account_id"] = json(&ledger.food);
            prop_assert_eq!(now, &expected, "file:\n{}", text);
        }
        Edit::ChangeTag => {
            let old_tags: BTreeSet<String> = parse(was)?;
            let new_tags: BTreeSet<String> = parse(now)?;
            prop_assert_eq!(new_tags.len(), old_tags.len().max(1), "file:\n{}", text);
            prop_assert_eq!(new_tags.difference(&old_tags).count(), 1, "file:\n{}", text);
        }
        Edit::AddMetadata => {
            let mut expected: Vec<MetaEntry> = parse::<Metadata>(was)?.iter().cloned().collect();
            expected.push(MetaEntry::new(
                key(ADDED_KEY),
                MetaValue::Text("added".to_owned()),
            ));
            prop_assert_eq!(now, &json(&Metadata::new(expected)), "file:\n{}", text);
        }
    }
    Ok(())
}

// MARK: Property

/// The directory holding the built plugins.
fn plugin_dir() -> PathBuf {
    env::var("BORROW_CHECKER_PLUGIN_DIR").map_or_else(
        |_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join("plugins")
        },
        PathBuf::from,
    )
}

/// Loads the beancount plugin alone, staged in `stage`, reading files from
/// `documents`.
fn load_beancount(stage: &Path, documents: &Path) -> impl Exporter + Importer {
    let built = plugin_dir().join("beancount.wasm");
    assert!(
        built.is_file(),
        "the beancount plugin is not built at {}: run `mise run build-plugins`",
        built.display()
    );
    std::fs::copy(&built, stage.join("beancount.wasm")).expect("stage plugin");
    let registry =
        PluginRegistry::load(&[stage.to_path_buf()], Some(documents)).expect("load registry");
    let plugin = registry.exporter("beancount").expect("beancount is loaded");
    drop(registry);
    Arc::try_unwrap(plugin).expect("the registry is dropped")
}

/// Seeds `specs`, exports into `documents`, applies `edit` to the transaction
/// at `target` and checks the database changed in exactly that edit.
async fn case(
    plugin: &(impl Exporter + Importer),
    documents: &Path,
    specs: &[Spec],
    target: Index,
    edit: Edit,
) -> Result<(), TestCaseError> {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = bc_core::open_db_at(&dir.path().join("db.sqlite"))
        .await
        .expect("open db");
    let ledger = ledger(&pool).await;
    let transactions = TransactionService::new(pool.clone());
    let mut ids = Vec::new();
    for spec in specs {
        let tx = transaction(spec, &ledger);
        ids.push(transactions.create(tx).await.expect("create").value);
    }
    let edited_id = target.get(&ids).clone();

    let service = RoundTripService::new(pool.clone());
    let export = service
        .export(&TransactionQuery::default(), ExportLevel::Working)
        .await
        .expect("export");
    let files = plugin.export(&export.snapshot).expect("plugin export");
    prop_assert_eq!(files.len(), 1, "a working export is one file");
    let file = files.first().expect("one file");
    let path = documents.join("slice.bean");
    std::fs::write(&path, &file.contents).expect("write export");
    export
        .sidecar
        .expect("a working export has a sidecar")
        .write(&Sidecar::path_for(&path))
        .expect("write sidecar");

    let sidecar = Sidecar::read(&Sidecar::path_for(&path)).expect("read sidecar");
    let handle = sidecar
        .entries
        .iter()
        .find(|entry| entry.transaction.id() == &edited_id)
        .map(|entry| entry.handle.clone())
        .expect("the target is exported");
    let target_spec = target.get(specs);
    let target_tags: Vec<&str> = target_spec
        .tags
        .iter()
        .filter_map(|&index| TAGS.get(index).copied())
        .collect();
    let text = edit_text(&file.contents, &handle, edit, &target_tags);
    prop_assert_ne!(&text, &file.contents, "the edit changed the text");
    std::fs::write(&path, &text).expect("write edit");

    let before = dump(&transactions).await;
    let output = plugin
        .import(&ImportConfig::from_value(
            serde_json::json!({ "source_file": "slice.bean" }),
        ))
        .expect("parse");
    let plan = service
        .plan_file(&output, &sidecar)
        .await
        .expect("plan")
        .map_err(|refused| TestCaseError::fail(format!("{refused}; file:\n{text}")))?;
    let outcome = service.apply(plan).await.expect("apply");
    prop_assert_eq!(outcome.edited, 1, "{:?}; file:\n{}", outcome, text);
    prop_assert_eq!(outcome.created, 0);
    prop_assert_eq!(outcome.deleted, 0);
    prop_assert_eq!(outcome.skipped, specs.len().saturating_sub(1));
    prop_assert!(outcome.refused.is_empty(), "{:?}", outcome.refused);
    let after = dump(&transactions).await;

    prop_assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (id, old) in &before {
        let new = after.get(id).expect("still stored");
        if *id == edited_id.to_string() {
            check_edited(old, new, edit, &ledger, &text)?;
        } else {
            prop_assert_eq!(
                new,
                old,
                "an untouched transaction changed; file:\n{}",
                text
            );
        }
    }
    pool.close().await;
    Ok(())
}

#[test]
fn random_edit_lands_exactly() {
    let stage = tempfile::tempdir().expect("stage dir");
    let documents = tempfile::tempdir().expect("documents dir");
    let plugin = load_beancount(stage.path(), documents.path());
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    // Every shape meets every edit on the edited transaction, twice.
    for shape in SHAPES {
        for edit in EDITS {
            let mut runner = TestRunner::new(Config {
                cases: 2,
                failure_persistence: None,
                ..Config::default()
            });
            runner
                .run(
                    &(vec(spec(), 1..4), any::<Index>()),
                    |(mut specs, target)| {
                        let at = target.index(specs.len());
                        if let Some(edited) = specs.get_mut(at) {
                            edited.shape = shape;
                        }
                        runtime.block_on(case(&plugin, documents.path(), &specs, target, edit))
                    },
                )
                .unwrap_or_else(|error| panic!("{edit:?} on {shape:?}: {error}"));
        }
    }
}
