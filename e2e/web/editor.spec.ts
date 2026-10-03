/**
 * The transaction editor's save-race and failure paths. `page.route` holds an
 * RPC open or fails it, to reach states a real server rarely produces.
 *
 * A successful save refreshes the register. The refresh leaves the open
 * editor mounted, so these tests let it run.
 */
import { expect, test, type Page, type Route } from '@playwright/test';

const INJECTED = JSON.stringify({ Internal: 'injected failure' });

/** Opens the most recent Groceries transaction from `payee`. */
async function openGroceries(page: Page, payee: string): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  await page.getByRole('navigation', { name: 'account navigation' })
    .getByText('Groceries', { exact: true }).click();
  await page.getByLabel('transaction register').getByText(payee, { exact: true }).first().click();
  await expect(page.getByTestId('status-pill')).toBeVisible();
}

/** Holds each `cmd` request until `release` is called. */
async function hold(page: Page, cmd: string): Promise<{ reached: Promise<void>; release: () => void }> {
  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  let arrive!: () => void;
  const reached = new Promise<void>((r) => { arrive = r; });
  await page.route(`**/rpc/${cmd}`, async (route: Route) => {
    arrive();
    await gate;
    await route.continue();
  });
  return { reached, release };
}

// A Rust panic in the WASM app surfaces as a page error; any one fails the test.
let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

function saveButton(page: Page) {
  return page.getByRole('button', { name: 'save transaction' });
}

test('keystrokes typed during a save survive its refetch', async ({ page }) => {
  await openGroceries(page, 'Woolworths');
  const edit = await hold(page, 'edit_transaction');
  const desc = page.getByPlaceholder('description');

  await desc.fill('Weekly shop');
  await saveButton(page).click();
  await edit.reached;
  await desc.fill('Weekly shop and more');
  const refetch = page.waitForResponse('**/rpc/get_transaction');
  edit.release();
  await refetch;

  await expect(desc).toHaveValue('Weekly shop and more');
  await expect(page.getByText('unsaved changes')).toBeVisible();
  await expect(page.getByLabel('transaction register')).toHaveAttribute('aria-busy', 'false');
  await expect(desc).toHaveValue('Weekly shop and more');

  // The refetch gave the editor a fresh base, so the kept edit saves cleanly.
  await saveButton(page).click();
  await expect(saveButton(page)).toBeHidden();
});

test('discard is disabled while a save is in flight', async ({ page }) => {
  await openGroceries(page, 'IGA');
  const edit = await hold(page, 'edit_transaction');
  const desc = page.getByPlaceholder('description');

  await desc.fill('Top-up shop');
  await saveButton(page).click();
  await edit.reached;
  await expect(page.getByRole('button', { name: 'discard changes' })).toBeDisabled();

  const refetch = page.waitForResponse('**/rpc/get_transaction');
  edit.release();
  await refetch;
  await expect(desc).toHaveValue('Top-up shop');
  await expect(saveButton(page)).toBeHidden();
});

test('Escape during a save closes the editor and the save lands', async ({ page }) => {
  await openGroceries(page, 'IGA');
  const edit = await hold(page, 'edit_transaction');
  const desc = page.getByPlaceholder('description');

  await desc.fill('Top-up via Escape');
  await saveButton(page).click();
  await edit.reached;
  // Escape from an input is ignored; send it from the panel itself.
  await page.getByText('balances', { exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(desc).toBeHidden();

  const refreshed = page.waitForResponse('**/rpc/register_page');
  edit.release();
  // The save's register refresh reaches the row; reopen it to see the stored copy.
  await refreshed;
  await expect(page.getByLabel('transaction register')).toHaveAttribute('aria-busy', 'false');
  await page.getByLabel('transaction register').getByText('IGA', { exact: true }).first().click();
  await expect(page.getByPlaceholder('description')).toHaveValue('Top-up via Escape');
});

test('a failed reconciliation is reported and kept in the draft', async ({ page }) => {
  await openGroceries(page, 'Coles');
  await page.route('**/rpc/set_reconciliation', (route) =>
    route.fulfill({ status: 500, contentType: 'application/json', body: INJECTED }));
  const pill = page.getByTestId('status-pill');

  const before = await pill.innerText();
  await pill.click();
  const requested = await pill.innerText();
  expect(requested).not.toBe(before);
  const refetch = page.waitForResponse('**/rpc/get_transaction');
  await saveButton(page).click();
  await refetch;

  await expect(page.getByText("Couldn't save changes")).toBeVisible();
  await expect(pill).toHaveText(requested);
  await expect(saveButton(page)).toBeVisible();
});

test('a failed refetch after a save requires a reload', async ({ page }) => {
  await openGroceries(page, 'Woolworths');
  let failNext = true;
  await page.route('**/rpc/get_transaction', (route) => {
    if (failNext) {
      failNext = false;
      return route.fulfill({ status: 500, contentType: 'application/json', body: INJECTED });
    }
    return route.continue();
  });
  const desc = page.getByPlaceholder('description');

  await desc.fill('Big shop');
  await saveButton(page).click();

  await expect(page.getByText("Saved, but couldn't refresh the transaction")).toBeVisible();
  await expect(saveButton(page)).toBeHidden();
  const reload = page.getByRole('button', { name: 'discard and reload' });
  await expect(reload).toBeVisible();

  await reload.click();
  await expect(reload).toBeHidden();
  await expect(desc).toHaveValue('Big shop');

  // The reload fetched the stored copy, so the next save is not stale.
  await desc.fill('Big shop, again');
  await saveButton(page).click();
  await expect(saveButton(page)).toBeHidden();
  await expect(page.getByText('This transaction changed since you opened it.')).toBeHidden();
});

test('Escape during a reload closes the editor cleanly', async ({ page }) => {
  await openGroceries(page, 'Coles');
  // The save's refetch fails, which offers the reload; the reload's fetch is held.
  let calls = 0;
  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  let arrive!: () => void;
  const reached = new Promise<void>((r) => { arrive = r; });
  await page.route('**/rpc/get_transaction', async (route) => {
    calls += 1;
    if (calls === 1) {
      return route.fulfill({ status: 500, contentType: 'application/json', body: INJECTED });
    }
    arrive();
    await gate;
    return route.continue();
  });
  const desc = page.getByPlaceholder('description');

  await desc.fill('Coles, before a reload');
  await saveButton(page).click();
  await page.getByRole('button', { name: 'discard and reload' }).click();
  await reached;
  // Escape from an input is ignored; send it from the panel itself.
  await page.getByText('balances', { exact: true }).click();
  await page.keyboard.press('Escape');
  await expect(desc).toBeHidden();

  const reloaded = page.waitForResponse('**/rpc/get_transaction');
  release();
  await reloaded;
  // The app still responds once the reload lands on the closed editor.
  await page.getByLabel('transaction register').getByText('Coles', { exact: true }).first().click();
  await expect(page.getByPlaceholder('description')).toBeVisible();
});

test('posting inputs follow the refetched posting order', async ({ page }) => {
  await openGroceries(page, 'IGA');
  // The server may return postings in any order; reverse them to force it.
  // The register serves the same order, or the open editor would adopt its
  // copy as another user's change.
  await page.route('**/rpc/get_transaction', async (route) => {
    const response = await route.fetch();
    const tx = await response.json();
    tx.postings.reverse();
    await route.fulfill({ response, json: tx });
  });
  await page.route('**/rpc/register_page', async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    for (const row of body.rows) row.transaction.postings.reverse();
    await route.fulfill({ response, json: body });
  });
  const rows = page.getByTestId('posting-row');
  await expect(rows).toHaveCount(2);
  const accounts = [
    await rows.nth(0).getByTestId('account-input').inputValue(),
    await rows.nth(1).getByTestId('account-input').inputValue(),
  ];
  const amounts = [
    await rows.nth(0).getByTestId('posting-amount').inputValue(),
    await rows.nth(1).getByTestId('posting-amount').inputValue(),
  ];
  expect(accounts[0]).not.toBe(accounts[1]);

  await page.getByPlaceholder('description').fill('Top-up, reordered');
  const refetch = page.waitForResponse('**/rpc/get_transaction');
  await saveButton(page).click();
  await refetch;
  await expect(saveButton(page)).toBeHidden();

  await expect(rows.nth(0).getByTestId('account-input')).toHaveValue(accounts[1]);
  await expect(rows.nth(0).getByTestId('posting-amount')).toHaveValue(amounts[1]);
  await expect(rows.nth(1).getByTestId('account-input')).toHaveValue(accounts[0]);
  await expect(rows.nth(1).getByTestId('posting-amount')).toHaveValue(amounts[0]);
});
