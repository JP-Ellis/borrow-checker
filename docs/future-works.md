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

______________________________________________________________________

## IPC budget commands — missing Tauri backend

**Identified in:** PR #??? (feat/budget-design)
**File:** `crates/bc-ipc/src/` and `crates/bc-app/src/commands/`

The CLI (`bc-cli`) supports full budget management operations (create, update,
archive, list, status), but these commands are not yet wired up for the desktop
GUI via Tauri IPC. `bc-ipc` is missing IPC message types for budget operations,
and `bc-app` is missing the corresponding command handlers.

The reason for deferral: the Leptos frontend (`bc-ui`) does not yet have a
budget management UI page, so implementing these commands would be premature.
They have no consumer in the current state.

When the budget UI is built out, add:

- Budget command types to `crates/bc-ipc/src/` (analogous to existing
  account commands like `CreateAccount`, `UpdateAccount`).
- Tauri command handlers in `crates/bc-app/src/commands/` (or similar module)
  that delegate to `bc-core` services.
- Wire up from the Leptos frontend to call these commands when budget
  management actions occur on the budget page.

______________________________________________________________________

## Budget detail — reuse TransactionRow from accounts page

**Identified in:** PR #159 (feat/bc-ui: budget page implementation)
**File:** `crates/bc-ui/src/pages/budget/components/budget_detail/mod.rs`

The budget detail panel has its own `TxRow` component instead of reusing
`TransactionRow` from the accounts page. This leads to UI inconsistency and
duplicated logic.

The accounts `TransactionRow` requires `viewing_account_id`, `selected`,
`expanded`, and `on_toggle` props (keyboard navigation). To reuse it in the
budget context:

- Pass `node.account_id` as `viewing_account_id` so the amount shown is the
  posting relevant to this budget's account, not a sum of all positive postings.
- Manage `selected` and `expanded` signals locally in `BudgetDetail` (or make
  those props optional with sensible defaults).

This also eliminates `tx_display_amount` once the account-relative posting
amount is sourced from `TransactionRow`'s existing logic.

______________________________________________________________________

## bc-ui — WASM test runner for period_nav unit tests

**Identified in:** PR #159 (feat/bc-ui: budget page implementation)
**File:** `crates/bc-ui/src/pages/budget/period_nav.rs`

`period_nav.rs` contains pure date-arithmetic unit tests (`#[test]`) that
previously ran natively via a `#[path]` shim in `main.rs`. The shim was removed
(PR #159) because `#[path]` is fragile and the reviewer flagged it as a
last-resort tool.

The correct solution is a WASM test runner (`wasm-bindgen-test` or `wasm-pack test`) so the tests run in the actual target environment. Until that
infrastructure exists, `period_nav` unit tests do not run anywhere — coverage
comes only from the E2E suite.

When implementing:

- Convert `#[test]` annotations in `period_nav.rs` to `#[wasm_bindgen_test]`.
- Add a `mise run test:wasm` task that invokes `wasm-pack test --headless --firefox` (or similar) against `bc-ui`.
- Gate the `wasm-bindgen-test` dev-dependency on `target_arch = "wasm32"`.

______________________________________________________________________

## IPC — carry `Decimal` directly instead of round-tripping through minor units

**Identified in:** PR #166 (feat/budget: mutable budgets with revision timeline)
**Files:** `crates/bc-ipc/src/money.rs`, `crates/bc-ipc/src/accounts.rs`,
`crates/bc-app/src/ipc.rs`, `crates/bc-app/src/commands/accounts.rs`

This PR established that `rust_decimal::Decimal` can live in `bc-ipc` and cross
the Tauri boundary losslessly: the workspace pins `rust_decimal` with the
`serde-with-str` feature, so a `Decimal` serialises as a string (e.g.
`"0.00000001"`), preserving arbitrary precision on both native and `wasm32`
targets. The budget target now uses this directly. Several other fields still
perform a `Decimal -> intermediate -> Decimal` round-trip and should be migrated
the same way.

**`bc_ipc::Amount` — the universal money type.** Currently
`{ minor_units: i64, scale: u8, currency_code: String }`. Every money value is
decomposed from a `bc_models::Amount` via `.mantissa()`/`.scale()`
(`bc-app/src/ipc.rs:39-40`) and reconstructed via
`Decimal::new(minor_units, scale)` (`ipc.rs:62`). It flows through nearly every
money-bearing command (account stats, sparkline, budget overview, native
periods, budget revisions). Replace the `{ minor_units, scale }` pair with a
single `value: Decimal`, keeping `currency_code`.

- ⚠️ **Latent corruption bug to fix in passing:** `ipc.rs:40` does
  `i64::try_from(self.value().mantissa()).unwrap_or(0)`. `mantissa()` is an
  `i128`; a large or high-precision value (e.g. a big BTC balance at 8 dp)
  overflows `i64` and silently becomes `0`. Carrying the `Decimal` removes the
  failure mode entirely.
- **Display nuance:** dropping `scale` means display fraction-digits must come
  from the currency registry (`currency.decimals`) rather than the value's own
  scale, since the two can differ. Decide once whether to format from the
  value's scale or normalise to the currency's decimals.
- `Amount::format_short` (bc-ipc) and `to_decimal_string` / `format_with_symbol`
  (bc-ui) are one-way display helpers, not round-trips — keep them, but they
  format straight from the `Decimal` afterwards.

**`bc_ipc::SparkPoint` — lossy today (highest severity).** Stores
`{ income: i64, expenses: i64 }` (minor units only); the scale is discarded at
`accounts.rs:397-398`, forcing the frontend to assume a scale to render. Carry
`Amount` (or `Decimal`) per point instead.

**Not at the IPC boundary yet — use `Decimal` directly when added:** loan terms
(`LoanTerms::principal`, `annual_rate`, `AmortizationRow` fields in
`bc-models/src/loan.rs`), depreciation (`DepreciationPolicy::annual_rate` in
`valuation.rs`), and `Account::acquisition_cost` (`account.rs`). None have a
`bc-ipc` mirror today, so there is no round-trip yet — but build them with
`Decimal` from the start.

The change is mechanical but touches all display paths and the round-trip tests
in `ipc.rs`. The `SparkPoint` scale-loss and the `unwrap_or(0)` overflow each
warrant their own commit + regression test.
