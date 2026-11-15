/**
 * Editing and adding accounts from Settings → Accounts: each dirty row saves
 * through its own `update_account`, a refused row keeps its draft and error,
 * a row changed elsewhere offers a reload, and a draft row saves through
 * `create_account`. Each test edits seed accounts no other web spec searches
 * for (Freelance, Interest, Healthcare, Entertainment, Water, Sewer); Car's
 * cost is typed but never saved. Added accounts take names no other spec
 * searches for (Sabbatical, Ferry Fares, Mothballed, Shelved).
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
async function createUnderAssets(page: Page, name: string, kind: string): Promise<string> {
  const details = await rpc<Detail[]>(page, 'list_account_details', {});
  const assets = details.find((d) => d.path === 'Assets');
  expect(assets).toBeDefined();
  const created = await rpc<Detail>(page, 'create_account', {
    name,
    parent_id: assets!.id,
    account_type: 'Asset',
    kind,
    description: null,
    commodity_codes: [],
    tag_ids: [],
    opened_on: null,
    acquisition: null,
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
  await expect(parent.locator('option:checked')).toHaveText('Expenses:Utilities:Water Supply');
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
    label: 'Assets:Sabbatical',
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
  ).toHaveText('Assets:Sabbatical');
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
  await expect(options.filter({ hasText: /^Assets:Checking$/ })).toHaveCount(1);
  await expect(options.filter({ hasText: /Mothballed|Shelved/ })).toHaveCount(0);

  // A chosen root type narrows the picker to that type.
  await draft.getByRole('combobox', { name: 'type', exact: true }).selectOption('liability');
  await expect(options.filter({ hasText: /^Assets/ })).toHaveCount(0);
  await expect(options.filter({ hasText: /^Liabilities:CreditCard$/ })).toHaveCount(1);

  await page.getByRole('button', { name: 'discard' }).click();
  await expect(draft).toHaveCount(0);
  await expect(page.getByTestId('accounts-savebar')).toBeHidden();
});
