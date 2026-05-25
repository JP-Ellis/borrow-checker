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

## Remove Playwright — consolidate on WebdriverIO

**Identified in:** PR #107 (test/desktop-e2e-visual)

The Playwright suite covers only visual component-level tests of `bc-ui`.
Since bc-ui is never served as a standalone web app (it runs exclusively inside
the Tauri WebView), Playwright tests a non-production environment and cannot
exercise the IPC layer or SQLite persistence.

WebdriverIO + tauri-driver tests the actual compiled app running on the same
WebKitGTK engine that ships to users, covering the full stack end-to-end.
Keeping both ecosystems imposes two containers, two package managers, two CI
jobs, and an ambiguous boundary between what belongs in each suite.

**Action:** Delete `e2e/web/` (the Playwright tree), remove its CI step, and
migrate any test cases worth preserving into the wdio suite.

______________________________________________________________________
