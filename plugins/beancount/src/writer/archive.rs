//! The archive level: a multi-file export of the whole ledger.
//!
//! - `main.bean` includes the other files in order.
//! - `accounts.bean` holds the `commodity` directives, then the `open`
//!   directives, then the `close` directives.
//! - `budgets.bean` holds one `custom "budget"` line per budget.
//! - `YYYY-MM.bean` holds one month of transactions in date order, then the
//!   `balance` lines that fall in that month.
//!
//! Beancount asserts a balance at the start of its day, while the snapshot
//! states a balance at the end of its day. Each balance is therefore written
//! on the day after the date it states, in that day's month file.

use std::collections::BTreeMap;

use bc_sdk::Date;
use bc_sdk::ExportAccount;
use bc_sdk::ExportBalance;
use bc_sdk::ExportBudget;
use bc_sdk::ExportCommodity;
use bc_sdk::ExportError;
use bc_sdk::ExportFile;
use bc_sdk::ExportSnapshot;
use bc_sdk::ExportTransaction;
use bc_sdk::MetaEntry;

use crate::writer::Offences;
use crate::writer::amount;
use crate::writer::amount_column;
use crate::writer::date;
use crate::writer::escape::quoted;
use crate::writer::meta_value;
use crate::writer::push_meta;
use crate::writer::transaction;

/// The root file, which includes every other file.
const MAIN_FILE: &str = "main.bean";

/// The file holding commodities, `open` and `close`.
const ACCOUNTS_FILE: &str = "accounts.bean";

/// The file holding budgets.
const BUDGETS_FILE: &str = "budgets.bean";

/// The year-and-month a month file covers.
type Month = (i32, u8);

/// One month file's contents, before rendering.
#[derive(Default)]
struct MonthBucket<'a> {
    /// Transactions, in the snapshot's order.
    transactions: Vec<&'a ExportTransaction>,
    /// Balances with the date each is written on.
    balances: Vec<(Date, &'a ExportBalance)>,
}

/// Renders `snapshot` as the archive files.
///
/// # Errors
///
/// Returns [`ExportError::Unspellable`] naming every item Beancount cannot
/// spell, and [`ExportError::Other`] for a budget with no target or a value
/// that cannot be spelled.
pub(super) fn write(snapshot: &ExportSnapshot) -> Result<Vec<ExportFile>, ExportError> {
    check_names(snapshot)?;

    let fallback = earliest(snapshot);
    let months = buckets(snapshot);

    let mut files = vec![
        file(ACCOUNTS_FILE, accounts(snapshot, &fallback)?),
        file(BUDGETS_FILE, budgets(&snapshot.budgets)?),
    ];
    let mut main = format!("include \"{ACCOUNTS_FILE}\"\ninclude \"{BUDGETS_FILE}\"\n");
    for (&(year, month), bucket) in &months {
        let name = format!("{year:04}-{month:02}.bean");
        main.push_str(&format!("include \"{name}\"\n"));
        files.push(file(&name, month_file(bucket)?));
    }
    files.insert(0, file(MAIN_FILE, main));
    Ok(files)
}

/// Builds a file.
fn file(path: &str, contents: String) -> ExportFile {
    ExportFile::builder().path(path).contents(contents).build()
}

// MARK: Name checks

/// Fails with every name in the snapshot that Beancount cannot spell.
fn check_names(snapshot: &ExportSnapshot) -> Result<(), ExportError> {
    let mut offences = Offences::default();
    for commodity in &snapshot.commodities {
        offences.commodity(&commodity.code);
        offences.metadata(&commodity.extra);
    }
    for account in &snapshot.accounts {
        offences.account(&account.path);
        for code in &account.commodities {
            offences.commodity(code);
        }
        offences.metadata(&account.extra);
    }
    for budget in &snapshot.budgets {
        offences.account(&budget.account);
        if let Some(ref target) = budget.target {
            offences.commodity(&target.commodity);
        }
        offences.metadata(&budget.extra);
    }
    offences.transactions(&snapshot.transactions);
    for balance in &snapshot.balances {
        offences.account(&balance.account);
        offences.commodity(&balance.amount.commodity);
    }
    offences.finish()
}

// MARK: Dates

/// The day after `day`.
fn next_day(day: &Date) -> Date {
    let leap = day.year % 4 == 0 && (day.year % 100 != 0 || day.year % 400 == 0);
    let length = match day.month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day.day < length {
        Date::new(day.year, day.month, day.day.saturating_add(1))
    } else if day.month < 12 {
        Date::new(day.year, day.month.saturating_add(1), 1)
    } else {
        Date::new(day.year.saturating_add(1), 1, 1)
    }
}

/// The date given to a directive that has none: the earliest date anywhere in
/// the snapshot, so it never follows a use of the thing it declares.
fn earliest(snapshot: &ExportSnapshot) -> Date {
    let dates = snapshot
        .transactions
        .iter()
        .map(|tx| tx.date.clone())
        .chain(snapshot.balances.iter().map(|b| b.date.clone()))
        .chain(snapshot.accounts.iter().filter_map(|a| a.opened_on.clone()))
        .chain(snapshot.budgets.iter().map(|b| b.effective_from.clone()));
    dates.min().unwrap_or_else(|| Date::new(1970, 1, 1))
}

// MARK: Month files

/// Groups transactions and balances by the month they are written in.
fn buckets(snapshot: &ExportSnapshot) -> BTreeMap<Month, MonthBucket<'_>> {
    let mut months: BTreeMap<Month, MonthBucket<'_>> = BTreeMap::new();
    for tx in &snapshot.transactions {
        months
            .entry((tx.date.year, tx.date.month))
            .or_default()
            .transactions
            .push(tx);
    }
    for balance in &snapshot.balances {
        let written = next_day(&balance.date);
        months
            .entry((written.year, written.month))
            .or_default()
            .balances
            .push((written, balance));
    }
    for bucket in months.values_mut() {
        bucket.transactions.sort_by(|a, b| a.date.cmp(&b.date));
        bucket.balances.sort_by(|a, b| a.0.cmp(&b.0));
    }
    months
}

/// Renders one month: transactions, then balances.
fn month_file(bucket: &MonthBucket<'_>) -> Result<String, ExportError> {
    let column = amount_column(bucket.transactions.iter().copied());
    let mut blocks = bucket
        .transactions
        .iter()
        .map(|tx| transaction(tx, column))
        .collect::<Result<Vec<_>, _>>()?;
    for (written, balance) in &bucket.balances {
        blocks.push(format!(
            "{} balance {}  {}\n",
            date(written),
            balance.account,
            amount(&balance.amount)
        ));
    }
    let mut out = blocks.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

// MARK: Accounts

/// Appends `extra` as metadata lines under a directive.
fn push_extra(out: &mut String, extra: &[MetaEntry]) -> Result<(), ExportError> {
    for entry in extra {
        push_meta(out, 2, &entry.key, &meta_value(&entry.value)?);
    }
    Ok(())
}

/// Renders `accounts.bean`.
fn accounts(snapshot: &ExportSnapshot, fallback: &Date) -> Result<String, ExportError> {
    let mut out = String::new();
    for ExportCommodity { code, extra, .. } in &snapshot.commodities {
        out.push_str(&format!("{} commodity {code}\n", date(fallback)));
        push_extra(&mut out, extra)?;
    }
    out.push('\n');
    for ExportAccount {
        path,
        opened_on,
        commodities,
        extra,
        ..
    } in &snapshot.accounts
    {
        let opened = opened_on.as_ref().unwrap_or(fallback);
        out.push_str(&format!("{} open {path}", date(opened)));
        if !commodities.is_empty() {
            out.push(' ');
            out.push_str(&commodities.join(","));
        }
        out.push('\n');
        push_extra(&mut out, extra)?;
    }
    out.push('\n');
    for account in &snapshot.accounts {
        if let Some(ref closed) = account.closed_on {
            out.push_str(&format!("{} close {}\n", date(closed), account.path));
        }
    }
    Ok(out)
}

// MARK: Budgets

/// Renders `budgets.bean`.
///
/// A budget with a target writes its figure and the target's currency,
/// using the expression text in place of the number when there is one. A
/// budget with no target is a tracking-only budget and is written without an
/// amount; the importer skips such a line.
///
/// # Errors
///
/// Returns [`ExportError::Other`] for an expression with no target, which has
/// no currency to carry it.
fn budgets(budgets: &[ExportBudget]) -> Result<String, ExportError> {
    let mut out = String::new();
    for budget in budgets {
        out.push_str(&format!(
            "{} custom {} {} {}",
            date(&budget.effective_from),
            quoted("budget"),
            budget.account,
            quoted(&budget.period),
        ));
        match (&budget.target, &budget.expression) {
            (Some(target), Some(expression)) => {
                out.push_str(&format!(" {expression} {}", target.commodity));
            }
            (Some(target), None) => {
                out.push_str(&format!(" {} {}", target.value, target.commodity));
            }
            (None, None) => {}
            (None, Some(_)) => {
                return Err(ExportError::Other(format!(
                    "the budget for {} has an expression but no target",
                    budget.account
                )));
            }
        }
        out.push('\n');
        push_extra(&mut out, &budget.extra)?;
    }
    Ok(out)
}
