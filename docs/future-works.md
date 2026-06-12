# Future Works

Tracked improvements that are out of scope for the PR that identified them.

______________________________________________________________________

## Accounts sidebar — all account types

**Identified in:** PR #105 (feat/ipc-add-transaction)
**File:** `crates/bc-ui/src/pages/accounts/components/sidebar/mod.rs`

The sidebar currently renders only `Asset` and `Liability` accounts as top-level
sections. It should iterate through **all** account types (`Asset`, `Liability`,
`Equity`, `Income`, `Expense`) so that the full account tree is visible without
special-casing. Any sections with zero accounts can be hidden.

______________________________________________________________________

## Account dashboard — live balance

**Identified in:** PR #105 (feat/ipc-add-transaction)
**File:** `crates/bc-app/src/commands/accounts.rs` (`IntoIpc for &bc_models::Account`)

`AccountNode.balance` is hardcoded to `Amount::new(0, "AUD")` with a `TODO(ipc)`
comment. It should call `BalanceEngine::balance_for(account_id, commodity)` from
`bc-core` to surface the real balance through the IPC layer.

The currency code must also be derived from the account's actual commodity rather
than hardcoded. A secondary concern is that `AccountNode` carries only a single
`balance` field; multi-currency accounts may eventually require a `Vec<Amount>`.

______________________________________________________________________

## Account dashboard — live stat cards and sparkline

**Identified in:** PR #105 (feat/ipc-add-transaction)
**File:** `crates/bc-ui/src/pages/accounts/dashboard/mod.rs`

All four stat cards and the 6-month cash-flow sparkline are fully hardcoded stubs.
Each needs a corresponding IPC command and bc-core query before it can show real
data:

- **Income (30d)** — sum of income postings to the selected account over the last
  30 calendar days. Requires a new date-range query on `TransactionService` (none
  exists today).
- **Expenses (30d)** — same, but for expense postings. Shares the same new query
  surface as income.
- **Uncategorised** — count of postings with no envelope/category assigned.
  Depends on the envelope/budget model existing first (see Milestone 5).
- **Last import** — timestamp of the most recent import run touching the selected
  account. Requires surfacing import-run metadata through IPC.
- **Sparkline (6 months)** — monthly income and expense totals for the last six
  months. Requires a monthly-bucket aggregation query in bc-core and a new IPC
  response type carrying `Vec<MonthlyStats>`.

The `stub_spark_points()` function and all hardcoded stat strings can be removed
once these are wired up.

______________________________________________________________________

## CLI — `account balance` subcommand

**Identified in:** PR #128 (feat/cli-account-balance)
**File:** `crates/bc-cli/src/commands/account.rs`

The `borrow-checker account` subcommand should gain a `balance` child command that
lists all account balances in a table: ID | NAME | TYPE | BALANCE | COMMODITY.

Requirements:

- Optional positional `<account-id>` to filter to a single account.
- `--commodity <CODE>` flag to filter by commodity.
- Rows sorted by account type (Asset → Liability → Equity → Income → Expense) then
  alphabetically by name within each type.
- `--json` flag emitting a JSON array of `{ id, name, type, balance, commodity }`
  objects.

The command should call `BalanceEngine::default_balances` (already in `bc-core`)
and format the result with `comfy-table` consistent with other `bc-cli` table
commands.

______________________________________________________________________

## Add-transaction form — per-posting tags and notes

**Identified in:** PR #127 (feat/add-transaction-form)
**File:** `crates/bc-ui/src/pages/accounts/components/add_transaction/mod.rs`

The add-transaction form currently supports date, payee, status, and N postings
(account + amount), but not per-transaction tags or per-posting notes.

- **Transaction tags** — free-form string labels (e.g. `budget:food`,
  `category:groceries`) attached to the `NewTransaction.tags` field. The IPC
  type already supports `Vec<String>` tags; only the UI is missing. A
  tag-token input (type-and-press-Enter to add, click token to remove) would
  map naturally to this field.

- **Posting notes** — the optional `note` field on `NewPosting`. A small
  per-row text input in the postings grid would surface this.

Both additions depend on the envelope/budget model being defined (Milestone 5)
to give tags a canonical vocabulary; until then a free-form input is acceptable.
