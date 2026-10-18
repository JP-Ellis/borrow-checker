//! Renders an export snapshot as Beancount text.
//!
//! The text is what the importer in this crate reads back: the `bc:` handle is
//! the first metadata line of every transaction, and the flag, tags and
//! spread of a posting travel in `bc-` keys.

mod archive;
mod escape;

use bc_sdk::Amount;
use bc_sdk::Cost;
use bc_sdk::Date;
use bc_sdk::ExportError;
use bc_sdk::ExportFile;
use bc_sdk::ExportLevel;
use bc_sdk::ExportPosting;
use bc_sdk::ExportSnapshot;
use bc_sdk::ExportTransaction;
use bc_sdk::MetaEntry;
use bc_sdk::MetaValue;
use bc_sdk::Quote;
use bc_sdk::Reconciliation;

use crate::writer::escape::is_account;
use crate::writer::escape::is_commodity;
use crate::writer::escape::is_cost_label;
use crate::writer::escape::is_meta_key;
use crate::writer::escape::is_tag;
use crate::writer::escape::quoted;

/// The file a working-level export produces.
const WORKING_FILE: &str = "export.bean";

/// The column the end of a posting's number lines up on, when the postings
/// are narrow enough to leave it free.
const MIN_AMOUNT_COLUMN: usize = 56;

/// Renders `snapshot` as Beancount files.
///
/// A working-level snapshot yields one file, `export.bean`, holding the
/// transactions in the snapshot's order. An archive-level snapshot yields the
/// multi-file layout described in [`archive`].
///
/// # Arguments
///
/// * `snapshot` - Everything the export covers.
///
/// # Returns
///
/// The files to write.
///
/// # Errors
///
/// Returns [`ExportError::Unspellable`] naming every item Beancount cannot
/// spell, once each, before any text is produced. Returns
/// [`ExportError::Other`] for a level or value this writer cannot spell.
pub(crate) fn write(snapshot: &ExportSnapshot) -> Result<Vec<ExportFile>, ExportError> {
    match snapshot.level {
        ExportLevel::Working => working(snapshot),
        ExportLevel::Archive => archive::write(snapshot),
        _ => Err(ExportError::Other(
            "the Beancount exporter does not write this export level".to_owned(),
        )),
    }
}

/// Renders the transactions of `snapshot` as the single working file.
fn working(snapshot: &ExportSnapshot) -> Result<Vec<ExportFile>, ExportError> {
    let mut offences = Offences::default();
    offences.transactions(&snapshot.transactions);
    offences.finish()?;

    let column = amount_column(&snapshot.transactions);
    let blocks = snapshot
        .transactions
        .iter()
        .map(|tx| transaction(tx, column))
        .collect::<Result<Vec<_>, _>>()?;
    let mut contents = blocks.join("\n");
    if !contents.is_empty() {
        contents.push('\n');
    }
    Ok(vec![
        ExportFile::builder()
            .path(WORKING_FILE)
            .contents(contents)
            .build(),
    ])
}

// MARK: Name checks

/// The names Beancount cannot spell, in first-seen order.
#[derive(Default)]
struct Offences(Vec<String>);

impl Offences {
    /// Records `name` unless `valid`, or unless already recorded.
    fn require(&mut self, valid: bool, name: &str) {
        if !valid && !self.0.iter().any(|seen| seen == name) {
            self.0.push(name.to_owned());
        }
    }

    /// Checks an account path.
    fn account(&mut self, path: &str) {
        self.require(is_account(path), path);
    }

    /// Checks a commodity code.
    fn commodity(&mut self, code: &str) {
        self.require(is_commodity(code), code);
    }

    /// Checks tag names.
    fn tags(&mut self, tags: &[String]) {
        for tag in tags {
            self.require(is_tag(tag), tag);
        }
    }

    /// Checks the keys and the names inside the values of `entries`.
    fn metadata(&mut self, entries: &[MetaEntry]) {
        for entry in entries {
            self.require(is_meta_key(&entry.key), &entry.key);
            match entry.value {
                MetaValue::Amount(ref amount) => self.commodity(&amount.commodity),
                MetaValue::Account(ref path) => self.account(path),
                _ => {}
            }
        }
    }

    /// Checks a price or cost basis.
    fn quote(&mut self, quote: &Quote) {
        if let Ok((_, amount)) = quote_parts(quote) {
            self.commodity(&amount.commodity);
        }
    }

    /// Checks one posting.
    fn posting(&mut self, posting: &ExportPosting) {
        self.account(&posting.account);
        self.tags(&posting.tags);
        self.metadata(&posting.metadata);
        if let Some(ref amount) = posting.amount {
            self.commodity(&amount.commodity);
        }
        if let Some(ref price) = posting.price {
            self.quote(price);
        }
        if let Some(ref cost) = posting.cost {
            self.quote(&cost.basis);
            if let Some(ref label) = cost.label {
                self.require(is_cost_label(label), label);
            }
        }
    }
}

impl Offences {
    /// Checks the names in every transaction.
    fn transactions(&mut self, transactions: &[ExportTransaction]) {
        for tx in transactions {
            self.tags(&tx.tags);
            self.metadata(&tx.metadata);
            for posting in &tx.postings {
                self.posting(posting);
            }
        }
    }

    /// Fails with every offending name, when there is one.
    fn finish(self) -> Result<(), ExportError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ExportError::Unspellable(self.0))
        }
    }
}

// MARK: Transactions

/// Splits a price or cost basis into whether it is a total, and its amount.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for a form this writer cannot spell.
fn quote_parts(quote: &Quote) -> Result<(bool, &Amount), ExportError> {
    match quote {
        Quote::PerUnit(amount) => Ok((false, amount)),
        Quote::Total(amount) => Ok((true, amount)),
        _ => Err(ExportError::Other(
            "a price has a form the Beancount exporter cannot spell".to_owned(),
        )),
    }
}

/// Spells a date as `YYYY-MM-DD`.
fn date(date: &Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

/// Spells an amount as `N CCY`, keeping the number's scale.
fn amount(amount: &Amount) -> String {
    format!("{} {}", amount.value, amount.commodity)
}

/// Spells a metadata value in Beancount syntax.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for a value type this writer cannot spell.
fn meta_value(value: &MetaValue) -> Result<String, ExportError> {
    Ok(match value {
        MetaValue::Text(text) => quoted(text),
        MetaValue::Number(number) => number.to_string(),
        MetaValue::Boolean(true) => "TRUE".to_owned(),
        MetaValue::Boolean(false) => "FALSE".to_owned(),
        MetaValue::Date(day) => date(day),
        // Beancount has no instant type; the RFC 3339 text is kept as a string.
        MetaValue::Timestamp(stamp) => quoted(stamp),
        MetaValue::Amount(value) => amount(value),
        // The reader types a bare word as an account only when it holds a `:`.
        MetaValue::Account(path) if path.contains(':') => path.clone(),
        MetaValue::Account(path) => quoted(path),
        _ => {
            return Err(ExportError::Other(
                "a metadata value has a type the Beancount exporter cannot spell".to_owned(),
            ));
        }
    })
}

/// Appends `key: value` lines at `indent` spaces.
fn push_meta(out: &mut String, indent: usize, key: &str, value: &str) {
    out.push_str(&" ".repeat(indent));
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

/// Spells a cost block: `{}` per unit, `{{}}` in total, with lot date and label.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for a basis that cannot be spelled.
fn cost(cost: &Cost) -> Result<String, ExportError> {
    let (total, basis) = quote_parts(&cost.basis)?;
    let mut parts = vec![amount(basis)];
    parts.extend(cost.date.as_ref().map(date));
    parts.extend(cost.label.as_deref().map(quoted));
    let inner = parts.join(", ");
    Ok(if total {
        format!("{{{{{inner}}}}}")
    } else {
        format!("{{{inner}}}")
    })
}

/// Spells the text after a posting's number: commodity, cost and price.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for a price or basis that cannot be spelled.
fn annotations(posting: &ExportPosting, amount: &Amount) -> Result<String, ExportError> {
    let mut out = format!(" {}", amount.commodity);
    if let Some(ref lot) = posting.cost {
        out.push(' ');
        out.push_str(&cost(lot)?);
    }
    if let Some(ref price) = posting.price {
        let (total, figure) = quote_parts(price)?;
        out.push_str(if total { " @@ " } else { " @ " });
        out.push_str(&self::amount(figure));
    }
    Ok(out)
}

/// The column the numbers of `transactions` end on.
fn amount_column<'a>(transactions: impl IntoIterator<Item = &'a ExportTransaction>) -> usize {
    let widest = transactions
        .into_iter()
        .flat_map(|tx| &tx.postings)
        .map(|posting| {
            let number = posting
                .amount
                .as_ref()
                .map_or(0, |a| a.value.to_string().chars().count());
            // Two spaces of indent, the account, two spaces of gap, the number.
            2_usize
                .saturating_add(posting.account.chars().count())
                .saturating_add(2)
                .saturating_add(number)
        })
        .max()
        .unwrap_or(0);
    widest.max(MIN_AMOUNT_COLUMN)
}

/// Spells one posting: its line, then its metadata lines.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for a metadata value that cannot be spelled.
fn posting(out: &mut String, posting: &ExportPosting, column: usize) -> Result<(), ExportError> {
    out.push_str("  ");
    out.push_str(&posting.account);
    if let Some(ref leg) = posting.amount {
        let number = leg.value.to_string();
        let used = 2_usize
            .saturating_add(posting.account.chars().count())
            .saturating_add(number.chars().count());
        out.push_str(&" ".repeat(column.saturating_sub(used).max(2)));
        out.push_str(&number);
        out.push_str(&annotations(posting, leg)?);
    }
    out.push('\n');

    if let Some(ref handle) = posting.handle {
        push_meta(out, 4, "bc", &quoted(handle));
    }
    if !posting.tags.is_empty() {
        push_meta(out, 4, "bc-tags", &quoted(&posting.tags.join(" ")));
    }
    if let Some(ref from) = posting.spread_from {
        push_meta(out, 4, "bc-spread-from", &date(from));
    }
    if let Some(ref until) = posting.spread_until {
        push_meta(out, 4, "bc-spread-until", &date(until));
    }
    for entry in &posting.metadata {
        push_meta(out, 4, &entry.key, &meta_value(&entry.value)?);
    }
    Ok(())
}

/// Spells one transaction, without a trailing blank line.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for a metadata value that cannot be spelled.
fn transaction(tx: &ExportTransaction, column: usize) -> Result<String, ExportError> {
    let flag = match tx.reconciliation {
        Reconciliation::Reconciled => "*",
        Reconciliation::Flagged => "!",
        _ => "txn",
    };
    let mut out = format!("{} {flag}", date(&tx.date));

    // Only a leading text `payee` entry becomes the payee string.
    let payee = match tx.metadata.first() {
        Some(MetaEntry {
            key,
            value: MetaValue::Text(text),
            ..
        }) if key == "payee" => Some(text),
        _ => None,
    };
    if let Some(text) = payee {
        out.push(' ');
        out.push_str(&quoted(text));
    }
    out.push(' ');
    out.push_str(&quoted(&tx.description));
    for tag in &tx.tags {
        out.push_str(" #");
        out.push_str(tag);
    }
    out.push('\n');

    push_meta(&mut out, 2, "bc", &quoted(&tx.handle));
    for entry in tx.metadata.iter().skip(usize::from(payee.is_some())) {
        push_meta(&mut out, 2, &entry.key, &meta_value(&entry.value)?);
    }
    for leg in &tx.postings {
        posting(&mut out, leg, column)?;
    }
    Ok(out)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_sdk::Amount;
    use bc_sdk::Cost;
    use bc_sdk::Date;
    use bc_sdk::ExportAccount;
    use bc_sdk::ExportBalance;
    use bc_sdk::ExportBudget;
    use bc_sdk::ExportCommodity;
    use bc_sdk::ExportLevel;
    use bc_sdk::ExportPosting;
    use bc_sdk::ExportTransaction;
    use bc_sdk::ImportConfig;
    use bc_sdk::Importer as _;
    use bc_sdk::MetaEntry;
    use bc_sdk::Quote;
    use bc_sdk::Reconciliation;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::BeancountImporter;
    use crate::ast::Directive;

    /// A one-file working snapshot of `transactions`.
    fn snapshot(transactions: Vec<ExportTransaction>) -> ExportSnapshot {
        ExportSnapshot::builder()
            .level(ExportLevel::Working)
            .transactions(transactions)
            .build()
    }

    /// Writes `transactions` and returns the single file's text.
    fn text(transactions: Vec<ExportTransaction>) -> String {
        let files = write(&snapshot(transactions)).expect("spellable");
        assert_eq!(files.len(), 1);
        let file = files.into_iter().next().expect("one file");
        assert_eq!(file.path, "export.bean");
        file.contents
    }

    /// A two-leg transaction with the given handle and description.
    fn simple(handle: &str, description: &str) -> ExportTransaction {
        ExportTransaction::builder()
            .handle(handle)
            .date(Date::new(2025, 3, 4))
            .description(description)
            .reconciliation(Reconciliation::Unreconciled)
            .postings(vec![
                ExportPosting::builder()
                    .account("Expenses:Food")
                    .amount(Amount::new(dec!(985.6300), "AUD"))
                    .build(),
                ExportPosting::builder().account("Assets:Bank").build(),
            ])
            .build()
    }

    #[test]
    fn working_transaction_spelling() {
        let reconciled = ExportTransaction::builder()
            .handle("tx-1")
            .date(Date::new(2025, 1, 15))
            .description("Weekly shop")
            .reconciliation(Reconciliation::Reconciled)
            .tags(vec!["groceries".to_owned(), "Owner/Alice".to_owned()])
            .metadata(vec![
                MetaEntry::text("payee", "Woolworths"),
                MetaEntry::text("note", "say \"hi\""),
                MetaEntry::number("count", dec!(3.50)),
                MetaEntry::boolean("checked", true),
                MetaEntry::date("due", Date::new(2025, 2, 1)),
                MetaEntry::amount("limit", Amount::new(dec!(20.00), "AUD")),
                MetaEntry::account("source", "Assets:Bank"),
            ])
            .postings(vec![
                ExportPosting::builder()
                    .handle("p-1")
                    .account("Expenses:Food")
                    .amount(Amount::new(dec!(42.10), "AUD"))
                    .tags(vec!["Owner/Alice".to_owned(), "Holiday".to_owned()])
                    .spread_from(Date::new(2025, 1, 1))
                    .spread_until(Date::new(2025, 1, 31))
                    .metadata(vec![MetaEntry::text("memo", "milk")])
                    .build(),
                ExportPosting::builder()
                    .handle("p-2")
                    .account("Assets:Bank")
                    .build(),
            ])
            .build();
        let flagged = ExportTransaction::builder()
            .handle("tx-2")
            .date(Date::new(2025, 1, 16))
            .description("Check later")
            .reconciliation(Reconciliation::Flagged)
            .postings(vec![
                ExportPosting::builder()
                    .account("Assets:Broker")
                    .amount(Amount::new(dec!(10), "VTI"))
                    .price(Quote::Total(Amount::new(dec!(2500.00), "USD")))
                    .build(),
                ExportPosting::builder()
                    .account("Assets:Broker:Lot")
                    .amount(Amount::new(dec!(5), "VTI"))
                    .cost(
                        Cost::builder()
                            .basis(Quote::Total(Amount::new(dec!(1200.00), "USD")))
                            .date(Date::new(2024, 6, 30))
                            .label("first lot")
                            .build(),
                    )
                    .build(),
                ExportPosting::builder()
                    .account("Assets:Broker:Unit")
                    .amount(Amount::new(dec!(-5), "VTI"))
                    .cost(
                        Cost::builder()
                            .basis(Quote::PerUnit(Amount::new(dec!(240.00), "USD")))
                            .build(),
                    )
                    .price(Quote::PerUnit(Amount::new(dec!(250.50), "USD")))
                    .build(),
            ])
            .build();
        let unreconciled = simple("tx-3", "Plain");
        insta::assert_snapshot!(text(vec![reconciled, flagged, unreconciled]));
    }

    #[test]
    fn unspellable_names_are_all_listed() {
        let tx = ExportTransaction::builder()
            .handle("tx-1")
            .date(Date::new(2025, 3, 4))
            .description("Bad names")
            .reconciliation(Reconciliation::Unreconciled)
            .tags(vec!["my tag".to_owned()])
            .postings(vec![
                ExportPosting::builder()
                    .account("Assets:bank")
                    .amount(Amount::new(dec!(1.00), "aud"))
                    .build(),
                ExportPosting::builder()
                    .account("Assets:bank")
                    .amount(Amount::new(dec!(-1.00), "aud"))
                    .build(),
            ])
            .build();
        let error = write(&snapshot(vec![tx])).expect_err("unspellable");
        assert_eq!(
            error,
            ExportError::Unspellable(vec![
                "my tag".to_owned(),
                "Assets:bank".to_owned(),
                "aud".to_owned(),
            ])
        );
    }

    #[test]
    fn amount_scale_is_preserved() {
        assert!(text(vec![simple("tx-1", "Scale")]).contains("985.6300 AUD"));
    }

    #[test]
    fn non_first_payee_is_a_metadata_line() {
        let mut tx = simple("tx-1", "Narration");
        tx.metadata = vec![
            MetaEntry::text("note", "first"),
            MetaEntry::text("payee", "Later"),
        ];
        let out = text(vec![tx]);
        assert!(out.contains("* \"Narration\"") || out.contains("txn \"Narration\""));
        assert!(out.contains("  payee: \"Later\""));
    }

    #[test]
    fn written_text_parses_back() {
        let mut tx = simple("42", "Round trip");
        tx.metadata = vec![MetaEntry::text("payee", "Shop")];
        if let Some(leg) = tx.postings.first_mut() {
            leg.cost = Some(
                Cost::builder()
                    .basis(Quote::PerUnit(Amount::new(dec!(2.50), "USD")))
                    .date(Date::new(2024, 1, 2))
                    .label("lot a")
                    .build(),
            );
            leg.price = Some(Quote::Total(Amount::new(dec!(3.00), "USD")));
        }
        let directives = crate::parser::parse(&text(vec![tx])).expect("parses");
        let Some(Directive::Transaction(parsed)) = directives.first() else {
            panic!("expected a transaction");
        };
        assert_eq!(parsed.payee.as_deref(), Some("Shop"));
        assert_eq!(parsed.narration, "Round trip");
        let handle = parsed.metadata.first().expect("handle line");
        assert_eq!(handle.key, "bc");
        let leg = parsed.postings.first().expect("first leg");
        assert_eq!(leg.amount.as_ref().map(|a| a.value), Some(dec!(985.6300)));
        assert_eq!(
            leg.cost.as_ref().and_then(|c| c.label.as_deref()),
            Some("lot a")
        );
        assert_eq!(
            leg.price,
            Some(Quote::Total(Amount::new(dec!(3.00), "USD")))
        );
    }

    #[test]
    fn newline_in_text_becomes_a_space() {
        let out = text(vec![simple("tx-1", "two\nlines")]);
        assert!(out.contains("\"two lines\""));
    }

    // MARK: Archive

    /// A transaction on `date` with the given handles.
    fn dated(handle: &str, date: Date, posting_handles: [&str; 2]) -> ExportTransaction {
        let mut tx = simple(handle, "Groceries");
        tx.date = date;
        for (leg, id) in tx.postings.iter_mut().zip(posting_handles) {
            leg.handle = Some(id.to_owned());
        }
        tx
    }

    /// Two months of transactions, an account with extra metadata, a
    /// commodity, an expression budget and two balances.
    fn archive_snapshot() -> ExportSnapshot {
        ExportSnapshot::builder()
            .level(ExportLevel::Archive)
            .commodities(vec![
                ExportCommodity::builder()
                    .code("AUD")
                    .extra(vec![MetaEntry::text("bc-name", "Australian dollar")])
                    .build(),
            ])
            .accounts(vec![
                ExportAccount::builder()
                    .path("Assets:Bank")
                    .opened_on(Date::new(2025, 1, 1))
                    .commodities(vec!["AUD".to_owned()])
                    .extra(vec![MetaEntry::text("bc-kind", "asset")])
                    .build(),
                ExportAccount::builder()
                    .path("Expenses:Food")
                    .opened_on(Date::new(2025, 1, 1))
                    .closed_on(Date::new(2025, 6, 30))
                    .build(),
            ])
            .budgets(vec![
                ExportBudget::builder()
                    .account("Expenses:Food")
                    .effective_from(Date::new(2025, 1, 1))
                    .target(Amount::new(dec!(600.00), "AUD"))
                    .expression("(6 * 100)")
                    .extra(vec![MetaEntry::text("bc-rollover", "carry")])
                    .build(),
            ])
            .transactions(vec![
                dated("tx-3", Date::new(2025, 2, 10), ["p-5", "p-6"]),
                dated("tx-2", Date::new(2025, 1, 20), ["p-3", "p-4"]),
                dated("tx-1", Date::new(2025, 1, 5), ["p-1", "p-2"]),
            ])
            .balances(vec![
                ExportBalance::builder()
                    .date(Date::new(2025, 1, 31))
                    .account("Assets:Bank")
                    .amount(Amount::new(dec!(-1971.26), "AUD"))
                    .build(),
                ExportBalance::builder()
                    .date(Date::new(2025, 2, 28))
                    .account("Assets:Bank")
                    .amount(Amount::new(dec!(-2956.89), "AUD"))
                    .build(),
            ])
            .build()
    }

    #[test]
    fn archive_layout() {
        let files = write(&archive_snapshot()).expect("spellable");
        let rendered = files
            .iter()
            .map(|file| format!("=== {}\n{}", file.path, file.contents))
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[rstest]
    #[case(Date::new(2025, 1, 31), "2025-02-01", "2025-02.bean")]
    #[case(Date::new(2025, 2, 28), "2025-03-01", "2025-03.bean")]
    #[case(Date::new(2024, 2, 28), "2024-02-29", "2024-02.bean")]
    #[case(Date::new(2024, 12, 31), "2025-01-01", "2025-01.bean")]
    fn balance_is_dated_the_day_after_month_end(
        #[case] stated: Date,
        #[case] written: &str,
        #[case] file: &str,
    ) {
        let snap = ExportSnapshot::builder()
            .level(ExportLevel::Archive)
            .balances(vec![
                ExportBalance::builder()
                    .date(stated)
                    .account("Assets:Bank")
                    .amount(Amount::new(dec!(10.00), "AUD"))
                    .build(),
            ])
            .build();
        let files = write(&snap).expect("spellable");
        let month = files.iter().find(|f| f.path == file).expect("month file");
        assert!(
            month
                .contents
                .contains(&format!("{written} balance Assets:Bank")),
            "{}",
            month.contents
        );
    }

    #[test]
    fn archive_names_are_all_checked() {
        let mut snap = archive_snapshot();
        snap.commodities = vec![ExportCommodity::builder().code("aud").build()];
        snap.accounts = vec![
            ExportAccount::builder()
                .path("Assets:bank")
                .extra(vec![MetaEntry::text("Bad Key", "x")])
                .build(),
        ];
        let error = write(&snap).expect_err("unspellable");
        assert_eq!(
            error,
            ExportError::Unspellable(vec![
                "aud".to_owned(),
                "Assets:bank".to_owned(),
                "Bad Key".to_owned(),
            ])
        );
    }

    #[test]
    fn archive_reimports() {
        let dir = std::env::temp_dir().join("bc-beancount-archive-reimports");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for file in write(&archive_snapshot()).expect("spellable") {
            std::fs::write(dir.join(&file.path), file.contents).expect("write file");
        }
        let root = dir.join("main.bean").to_str().expect("utf8").to_owned();
        let config = ImportConfig::from_json_string(
            serde_json::json!({ "source_file": root.clone() }).to_string(),
        );
        let imported = BeancountImporter
            .import(config)
            .expect("the archive imports")
            .into_transactions();
        let handles = imported
            .iter()
            .map(|tx| tx.native.as_ref().and_then(|n| n.handle.clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            handles,
            vec![
                Some("tx-1".to_owned()),
                Some("tx-2".to_owned()),
                Some("tx-3".to_owned())
            ]
        );
        let posting_handles = imported
            .iter()
            .flat_map(|tx| &tx.postings)
            .map(|p| p.native.as_ref().and_then(|n| n.handle.clone()))
            .collect::<Vec<_>>();
        let expected = ["p-1", "p-2", "p-3", "p-4", "p-5", "p-6"]
            .map(|h| Some(h.to_owned()))
            .to_vec();
        assert_eq!(posting_handles, expected);

        let found = crate::budgets(&root).expect("budgets read back");
        let [budget] = found.as_slice() else {
            panic!("expected one budget, got {found:?}");
        };
        assert_eq!(budget.account, "Expenses:Food");
        assert_eq!(budget.expression.as_deref(), Some("(6 * 100)"));
    }
}
