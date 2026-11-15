/**
 * Editing, adding, closing and archiving accounts from Settings → Accounts: each dirty row saves
 * through its own `update_account`, a refused row keeps its draft and error,
 * a row changed elsewhere offers a reload, and a draft row saves through
 * `create_account`. Each test edits seed accounts no other web spec searches
 * for (Freelance, Interest, Healthcare, Entertainment, Water, Sewer); Car's
 * cost is typed but never saved. Added accounts take names no other spec
 * searches for (Sabbatical, Ferry Fares, Mothballed, Shelved). The lifecycle
 * tests close, archive and reopen accounts they add (Lighthouse, Keeper,
 * Piggybank, Piggyseed, Driftwood, Flotsam, Jetsam, Tidepool, Tidewater,
 * Foghorn, Foghut). The reload-failure test renames one it adds (Periwinkle).
 * The sidebar and dashboard tests do the same with Lanternfish, Anglerfish
 * and Barnacle.
 */
import { expect, test, type Page } from '@playwright/test';

// A Rust panic in the WASM app surfaces as a page error; any one fails the test.
let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

async function openAccountsSettings(page: Page): Promise<void> {
  await page.goto('/settings?section=accounts');
  await expect(page.getByTestId('accounts-table')).toBeVisible();
}

/** The table row of the account whose saved name is `name`. */
function row(page: Page, name: string) {
  return page.getByRole('row', { name, exact: true });
}

function field(page: Page, account: string, label: string) {
  return row(page, account).getByRole('textbox', { name: label, exact: true });
}

/** Calls a backend command directly, as another writer would. */
async function rpc<T>(page: Page, command: string, data: object): Promise<T> {
  const response = await page.request.post(`/rpc/${command}`, { data });
  expect(response.ok(), `${command}: ${await response.text()}`).toBeTruthy();
  return response.json();
}

type Detail = { id: string; name: string; path: string; event_count: number };

/** Creates an account under the seed's Assets root, returning its ID. */
async function createUnderAssets(
  page: Page,
  name: string,
  kind: string,
  extra: { commodity_codes?: string[]; opened_on?: string } = {},
): Promise<string> {
  const details = await rpc<Detail[]>(page, 'list_account_details', {});
  const assets = details.find((d) => d.path === 'Assets');
  expect(assets).toBeDefined();
  return createUnder(page, name, assets!.id, kind, extra);
}

/** Creates an asset account under `parentId`, returning its ID. */
async function createUnder(
  page: Page,
  name: string,
  parentId: string,
  kind: string,
  extra: { commodity_codes?: string[]; opened_on?: string } = {},
): Promise<string> {
  const created = await rpc<Detail>(page, 'create_account', {
    name,
    parent_id: parentId,
    account_type: 'Asset',
    kind,
    description: null,
    commodity_codes: [],
    tag_ids: [],
    opened_on: null,
    acquisition: null,
    ...extra,
  });
  return created.id;
}

test('rename and save updates the sidebar', async ({ page }) => {
  await openAccountsSettings(page);
  await field(page, 'Freelance', 'name').fill('Side Gigs');
  await page.getByRole('button', { name: 'save 1 change' }).click();
  await expect(row(page, 'Side Gigs')).toBeVisible();
  await expect(page.getByTestId('accounts-savebar')).toBeHidden();

  await page.getByTestId('nav-accounts').click();
  const sidebar = page.getByRole('navigation', { name: 'account navigation' }).first();
  await expect(sidebar.getByText('Side Gigs', { exact: true })).toBeVisible();
  await expect(sidebar.getByText('Freelance', { exact: true })).toHaveCount(0);
});

test('one failing row leaves the other saved', async ({ page }) => {
  await openAccountsSettings(page);
  await field(page, 'Interest', 'name').fill('Salary');
  await field(page, 'Healthcare', 'description').fill('Clinic visits');
  await page.getByRole('button', { name: 'save 2 changes' }).click();

  await expect(row(page, 'Interest').getByRole('alert')).toBeVisible();
  await expect(field(page, 'Interest', 'name')).toHaveValue('Salary');
  await expect(page.getByRole('button', { name: 'save 1 change' })).toBeVisible();

  // The saved row is clean and keeps its value across a reload.
  await page.reload();
  await expect(field(page, 'Healthcare', 'description')).toHaveValue('Clinic visits');
  await expect(field(page, 'Interest', 'name')).toHaveValue('Interest');
});

test('stale row offers reload', async ({ page }) => {
  await openAccountsSettings(page);
  await field(page, 'Entertainment', 'description').fill('Edited here');

  // Another writer changes the account after the page loaded it.
  const listed = await page.request.post('/rpc/list_account_details', { data: {} });
  expect(listed.ok()).toBeTruthy();
  const details: { id: string; name: string; event_count: number }[] = await listed.json();
  const target = details.find((d) => d.name === 'Entertainment');
  expect(target).toBeDefined();
  const updated = await page.request.post('/rpc/update_account', {
    data: {
      id: target!.id,
      expected_events: target!.event_count,
      patch: { description: 'Edited elsewhere' },
    },
  });
  expect(updated.ok()).toBeTruthy();

  await page.getByRole('button', { name: 'save 1 change' }).click();
  const alert = row(page, 'Entertainment').getByRole('alert');
  await expect(alert).toContainText('conflict');
  await expect(field(page, 'Entertainment', 'description')).toHaveValue('Edited here');

  await alert.getByRole('button', { name: 'reload' }).click();
  await expect(field(page, 'Entertainment', 'description')).toHaveValue('Edited elsewhere');
  await expect(row(page, 'Entertainment').getByRole('alert')).toHaveCount(0);
  await expect(page.getByTestId('accounts-savebar')).toBeHidden();
});

test('renaming a parent updates its children\'s parent picker', async ({ page }) => {
  await openAccountsSettings(page);
  await field(page, 'Water', 'name').fill('Water Supply');
  await page.getByRole('button', { name: 'save 1 change' }).click();
  await expect(row(page, 'Water Supply')).toBeVisible();

  const parent = row(page, 'Drinking').getByRole('combobox', { name: 'parent', exact: true });
  await expect(parent.locator('option:checked')).toHaveText('Expenses :: Utilities :: Water Supply');
});

test('an unparsed cost blocks only its own row', async ({ page }) => {
  await openAccountsSettings(page);
  const cost = field(page, 'Car', 'cost');
  await cost.fill('12,000');
  await cost.blur();
  await expect(row(page, 'Car').getByRole('alert')).toHaveText('“12,000” is not a number');
  await expect(cost).toHaveValue('12,000');

  await field(page, 'Sewer', 'description').fill('Wastewater');
  await page.getByRole('button', { name: 'save 2 changes' }).click();
  await expect(page.getByRole('button', { name: 'save 1 change' })).toBeVisible();
  await expect(row(page, 'Car').getByRole('alert')).toBeVisible();

  await page.reload();
  await expect(field(page, 'Sewer', 'description')).toHaveValue('Wastewater');
  await expect(field(page, 'Car', 'cost')).not.toHaveValue('12,000');
});

test('add an envelope under a group', async ({ page }) => {
  await createUnderAssets(page, 'Sabbatical', 'Group');
  await openAccountsSettings(page);

  await page.getByRole('button', { name: '+ add account' }).click();
  const draft = page.getByTestId('account-draft');
  await expect(draft.getByText('name is required')).toBeVisible();
  await draft.getByRole('textbox', { name: 'name', exact: true }).fill('Ferry Fares');
  await draft.getByRole('combobox', { name: 'parent', exact: true }).selectOption({
    label: 'Assets :: Sabbatical',
  });
  // The parent fixes the type, so the type select goes away.
  await expect(draft.getByRole('combobox', { name: 'type', exact: true })).toHaveCount(0);
  await draft.getByRole('combobox', { name: 'kind', exact: true }).selectOption({ label: 'virtual' });
  await draft.getByRole('combobox', { name: 'add commodity', exact: true }).selectOption('AUD');
  await draft.getByLabel('opened', { exact: true }).fill('2026-11-01');
  await page.getByRole('button', { name: 'save 1 change' }).click();

  await expect(draft).toHaveCount(0);
  const saved = row(page, 'Ferry Fares');
  await expect(saved).toBeVisible();
  await expect(saved.getByRole('cell', { name: 'virtual', exact: true })).toBeVisible();
  await expect(
    saved.getByRole('combobox', { name: 'parent', exact: true }).locator('option:checked'),
  ).toHaveText('Assets :: Sabbatical');
  await expect(saved.getByLabel('opened', { exact: true })).toHaveValue('2026-11-01');
  await expect(saved.getByRole('button', { name: 'AUD', exact: true })).toBeVisible();
  await expect(page.getByTestId('accounts-savebar')).toBeHidden();

  await page.getByTestId('nav-accounts').click();
  const sidebar = page.getByRole('navigation', { name: 'account navigation' }).first();
  const toggle = sidebar.getByRole('button', { name: 'toggle Sabbatical' });
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
    await toggle.click();
  }
  await expect(sidebar.getByText('Ferry Fares', { exact: true })).toBeVisible();
});

test('parent picker hides closed and archived accounts', async ({ page }) => {
  const closed = await createUnderAssets(page, 'Mothballed', 'DepositAccount');
  await rpc(page, 'close_account', { id: closed, on: '2026-10-31', cascade: false });
  const archived = await createUnderAssets(page, 'Shelved', 'DepositAccount');
  await rpc(page, 'archive_account', { id: archived, cascade: false });
  await openAccountsSettings(page);
  // Both still list as saved rows.
  await expect(row(page, 'Mothballed')).toBeVisible();
  await expect(row(page, 'Shelved')).toBeVisible();

  await page.getByRole('button', { name: '+ add account' }).click();
  const draft = page.getByTestId('account-draft');
  const options = draft.getByRole('combobox', { name: 'parent', exact: true }).locator('option');
  await expect(options.filter({ hasText: /^Assets :: Checking$/ })).toHaveCount(1);
  await expect(options.filter({ hasText: /Mothballed|Shelved/ })).toHaveCount(0);

  // A chosen root type narrows the picker to that type.
  await draft.getByRole('combobox', { name: 'type', exact: true }).selectOption('liability');
  await expect(options.filter({ hasText: /^Assets/ })).toHaveCount(0);
  await expect(options.filter({ hasText: /^Liabilities :: CreditCard$/ })).toHaveCount(1);

  await page.getByRole('button', { name: 'discard' }).click();
  await expect(draft).toHaveCount(0);
  await expect(page.getByTestId('accounts-savebar')).toBeHidden();
});

/** Today's date in the local time zone, as the date input shows it. */
function localToday(): string {
  const d = new Date();
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${month}-${day}`;
}

/** Posts `value` AUD into `accountId` on `date`, balanced by `otherId`. */
async function transfer(page: Page, accountId: string, otherId: string, date: string, value: string) {
  const posting = (account_id: string, amount: object | null) => ({
    account_id,
    amount,
    metadata: [],
    tags: [],
    spread_from: null,
    spread_until: null,
    price: null,
    cost: null,
  });
  await rpc(page, 'create_transaction', {
    tx: {
      date,
      description: 'Coin jar top-up',
      metadata: [],
      reconciliation: 'Unreconciled',
      tags: [],
      postings: [posting(accountId, { value, currency_code: 'AUD' }), posting(otherId, null)],
    },
  });
}

function stateCell(page: Page, account: string) {
  return row(page, account).getByRole('cell', { name: /^(active|closed|archived)$/ });
}

test('close with open child requires ticking close these too', async ({ page }) => {
  const parent = await createUnderAssets(page, 'Lighthouse', 'Group');
  await createUnder(page, 'Keeper', parent, 'DepositAccount');
  await openAccountsSettings(page);

  await row(page, 'Lighthouse').getByRole('button', { name: 'close…' }).click();
  const panel = page.getByRole('region', { name: 'close account' });
  await expect(panel.getByText('Assets :: Lighthouse :: Keeper')).toBeVisible();
  const confirm = panel.getByRole('button', { name: 'close', exact: true });
  await expect(confirm).toBeDisabled();
  await panel.getByRole('checkbox', { name: 'close these too' }).check();
  await expect(confirm).toBeEnabled();
  await confirm.click();

  await expect(panel).toHaveCount(0);
  await expect(stateCell(page, 'Lighthouse')).toHaveText('closed');
  await expect(stateCell(page, 'Keeper')).toHaveText('closed');
  await expect(row(page, 'Lighthouse').getByRole('button', { name: 'reopen' })).toBeVisible();
});

test('close shows remaining balance and closes anyway', async ({ page }) => {
  const jar = await createUnderAssets(page, 'Piggybank', 'DepositAccount');
  const seed = await createUnderAssets(page, 'Piggyseed', 'DepositAccount');
  await transfer(page, jar, seed, '2020-01-15', '50.00');
  await openAccountsSettings(page);

  await row(page, 'Piggybank').getByRole('button', { name: 'close…' }).click();
  const panel = page.getByRole('region', { name: 'close account' });
  const closeOn = panel.getByLabel('close on', { exact: true });
  const balance = panel
    .getByRole('list', { name: 'balances' })
    .getByRole('listitem')
    .filter({ hasText: 'Assets :: Piggybank' });
  await expect(closeOn).toHaveValue(localToday());
  await expect(balance).toContainText('50.00');

  // An earlier date re-runs the preview: no balance yet, one posting after.
  await closeOn.fill('2020-01-01');
  await expect(panel.getByText('1 posting falls after 2020-01-01')).toBeVisible();
  await expect(balance).toHaveCount(0);

  // A cleared date blocks confirm instead of closing on the last valid one.
  const confirm = panel.getByRole('button', { name: 'close', exact: true });
  await closeOn.fill('');
  await expect(panel.getByText('enter a close date')).toBeVisible();
  await expect(confirm).toBeDisabled();

  await closeOn.fill(localToday());
  await expect(balance).toBeVisible();
  await panel.getByRole('button', { name: 'close', exact: true }).click();

  await expect(panel).toHaveCount(0);
  await expect(stateCell(page, 'Piggybank')).toHaveText('closed');
  await expect(row(page, 'Piggybank').getByLabel('closed', { exact: true })).toHaveValue(localToday());

  await row(page, 'Piggybank').getByRole('button', { name: 'reopen' }).click();
  await expect(stateCell(page, 'Piggybank')).toHaveText('active');
});

test('archive without close date shows nudge', async ({ page }) => {
  await createUnderAssets(page, 'Driftwood', 'DepositAccount');
  await openAccountsSettings(page);

  await row(page, 'Driftwood').getByRole('button', { name: 'archive…' }).click();
  const panel = page.getByRole('region', { name: 'archive account' });
  await expect(panel.getByText('this account has no close date')).toBeVisible();
  await expect(panel.getByRole('checkbox', { name: 'archive these too' })).toHaveCount(0);
  await panel.getByRole('button', { name: 'archive', exact: true }).click();

  await expect(panel).toHaveCount(0);
  await expect(stateCell(page, 'Driftwood')).toHaveText('archived');
  await expect(row(page, 'Driftwood').getByRole('button', { name: 'unarchive' })).toBeVisible();
});

test('unarchive restores the row', async ({ page }) => {
  const id = await createUnderAssets(page, 'Flotsam', 'DepositAccount');
  await rpc(page, 'archive_account', { id, cascade: false });
  await openAccountsSettings(page);
  await expect(stateCell(page, 'Flotsam')).toHaveText('archived');

  await row(page, 'Flotsam').getByRole('button', { name: 'unarchive' }).click();
  await expect(stateCell(page, 'Flotsam')).toHaveText('active');
  await expect(row(page, 'Flotsam').getByRole('button', { name: 'archive…' })).toBeEnabled();

  await page.reload();
  await expect(stateCell(page, 'Flotsam')).toHaveText('active');
});

test('lifecycle actions are disabled on a dirty row', async ({ page }) => {
  await createUnderAssets(page, 'Jetsam', 'DepositAccount');
  await openAccountsSettings(page);
  const close = row(page, 'Jetsam').getByRole('button', { name: 'close…' });
  const archive = row(page, 'Jetsam').getByRole('button', { name: 'archive…' });
  await expect(close).toBeEnabled();

  await field(page, 'Jetsam', 'description').fill('Washed ashore');
  await expect(close).toBeDisabled();
  await expect(archive).toBeDisabled();

  // A new account has no lifecycle actions until it is saved.
  await page.getByRole('button', { name: '+ add account' }).click();
  const draft = page.getByTestId('account-draft');
  await expect(draft.getByRole('button', { name: /close…|archive…|reopen|unarchive/ })).toHaveCount(0);

  await page.getByRole('button', { name: 'discard' }).click();
  await expect(close).toBeEnabled();
  await expect(archive).toBeEnabled();
});

test('a slow preview for an earlier date never overwrites a newer one', async ({ page }) => {
  const pool = await createUnderAssets(page, 'Tidepool', 'DepositAccount');
  const water = await createUnderAssets(page, 'Tidewater', 'DepositAccount');
  await transfer(page, pool, water, '2020-01-15', '50.00');

  // Hold the preview for today until the one for 2020-01-01 has landed.
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route('**/rpc/preview_close_account', async (route) => {
    if (route.request().postDataJSON().on === localToday()) {
      await held;
    }
    await route.continue();
  });
  await openAccountsSettings(page);

  await row(page, 'Tidepool').getByRole('button', { name: 'close…' }).click();
  const panel = page.getByRole('region', { name: 'close account' });
  await panel.getByLabel('close on', { exact: true }).fill('2020-01-01');
  const late = panel.getByText('1 posting falls after 2020-01-01');
  await expect(late).toBeVisible();

  const stale = page.waitForResponse('**/rpc/preview_close_account');
  release();
  await stale;
  await expect(late).toBeVisible();
  await expect(panel.getByRole('list', { name: 'balances' })).toHaveCount(0);
});

test('a failed preview warns and the close still refuses an open child', async ({ page }) => {
  const parent = await createUnderAssets(page, 'Foghorn', 'Group');
  await createUnder(page, 'Foghut', parent, 'DepositAccount');
  await page.route('**/rpc/preview_close_account', (route) =>
    route.fulfill({ status: 500, body: 'preview offline' }),
  );
  await openAccountsSettings(page);

  await row(page, 'Foghorn').getByRole('button', { name: 'close…' }).click();
  const panel = page.getByRole('region', { name: 'close account' });
  await expect(panel.getByText(/^preview unavailable — /)).toBeVisible();
  await expect(panel.getByRole('checkbox')).toHaveCount(0);

  // Confirm sends no cascade, so the write names the open child.
  await panel.getByRole('button', { name: 'close', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Foghut');
  await expect(stateCell(page, 'Foghorn')).toHaveText('active');
});

test('a failed reload after a save shows a retry that recovers', async ({ page }) => {
  await createUnderAssets(page, 'Periwinkle', 'DepositAccount');
  await openAccountsSettings(page);
  let failing = true;
  await page.route('**/rpc/list_account_details', (route) =>
    failing ? route.fulfill({ status: 500, body: 'list offline' }) : route.fallback(),
  );

  await field(page, 'Periwinkle', 'name').fill('Limpet');
  await page.getByRole('button', { name: 'save 1 change' }).click();
  const notice = page.getByTestId('accounts-reload-error');
  await expect(notice).toContainText(/^\/\/ reload failed — /);
  // The save itself landed.
  await expect(row(page, 'Limpet')).toBeVisible();

  // Retry waits while another save runs.
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route('**/rpc/update_account', async (route) => {
    await held;
    await route.fallback();
  });
  await field(page, 'Limpet', 'description').fill('Rock pool');
  await page.getByRole('button', { name: 'save 1 change' }).click();
  await expect(notice.getByRole('button', { name: 'retry' })).toBeDisabled();
  release();
  await expect(page.getByTestId('accounts-savebar')).toBeHidden();
  await expect(notice.getByRole('button', { name: 'retry' })).toBeEnabled();

  failing = false;
  await notice.getByRole('button', { name: 'retry' }).click();
  await expect(notice).toHaveCount(0);
});

test('a failed tag load shows a retry that recovers', async ({ page }) => {
  let failing = true;
  await page.route('**/rpc/list_tags', (route) =>
    failing ? route.fulfill({ status: 500, body: 'tags offline' }) : route.fallback(),
  );
  await openAccountsSettings(page);
  const notice = page.getByTestId('accounts-tags-error');
  await expect(notice).toContainText(/^\/\/ tags failed to load — /);

  failing = false;
  await notice.getByRole('button', { name: 'retry' }).click();
  await expect(notice).toHaveCount(0);
});

// MARK: Sidebar and dashboard

function sidebar(page: Page) {
  return page.getByRole('navigation', { name: 'account navigation' }).first();
}

function dashboard(page: Page) {
  return page.getByLabel('account dashboard');
}

test('closed account is dimmed with its close date in the sidebar', async ({ page }) => {
  const id = await createUnderAssets(page, 'Lanternfish', 'DepositAccount');
  await rpc(page, 'close_account', { id, on: '2026-10-31', cascade: false });
  await page.goto('/accounts');

  const link = sidebar(page).getByRole('link').filter({ hasText: 'Lanternfish' });
  await expect(link).toContainText('closed 2026-10-31');
  await expect(link).toHaveCSS('opacity', '0.5');
  const open = sidebar(page).getByRole('link').filter({ hasText: 'Checking' });
  await expect(open).toHaveCSS('opacity', '1');
  await expect(open).not.toContainText('closed');
});

test('dashboard shows kind, commodity and dates', async ({ page }) => {
  const id = await createUnderAssets(page, 'Anglerfish', 'DepositAccount', {
    commodity_codes: ['AUD'],
    opened_on: '2026-01-05',
  });
  await rpc(page, 'close_account', { id, on: '2026-10-31', cascade: false });
  await page.goto(`/accounts/${id}`);

  await expect(dashboard(page).getByTestId('dashboard-meta')).toHaveText(
    'deposit · AUD · opened 2026-01-05 · closed 2026-10-31',
  );
});

test('dashboard close shows reopen afterwards', async ({ page }) => {
  const id = await createUnderAssets(page, 'Barnacle', 'DepositAccount');
  await page.goto(`/accounts/${id}`);
  await expect(dashboard(page).getByTestId('dashboard-meta')).toHaveText('deposit');

  await dashboard(page).getByRole('button', { name: 'close…' }).click();
  const panel = page.getByRole('region', { name: 'close account' });
  await panel.getByRole('button', { name: 'close', exact: true }).click();

  await expect(panel).toHaveCount(0);
  const reopen = dashboard(page).getByRole('button', { name: 'reopen' });
  await expect(reopen).toBeVisible();
  await expect(dashboard(page).getByTestId('dashboard-meta')).toHaveText(
    `deposit · closed ${localToday()}`,
  );
  await expect(sidebar(page).getByRole('link').filter({ hasText: 'Barnacle' })).toContainText(
    `closed ${localToday()}`,
  );

  await reopen.click();
  await expect(dashboard(page).getByRole('button', { name: 'close…' })).toBeVisible();
  await expect(dashboard(page).getByTestId('dashboard-meta')).toHaveText('deposit');
});
