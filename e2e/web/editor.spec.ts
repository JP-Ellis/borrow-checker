/**
 * The transaction editor's save-race and failure paths. `page.route` holds an
 * RPC open or fails it, to reach states a real server rarely produces.
 *
 * A successful save refreshes the register, which remounts the row and
 * rebuilds the editor from the stored transaction. That rebuild hides what
 * these tests check, so each one holds the refresh open with `freezeRegister`.
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

/** Leaves every later `register_page` request pending, so the row never remounts. */
async function freezeRegister(page: Page): Promise<void> {
  await page.route('**/rpc/register_page', () => {});
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

function saveButton(page: Page) {
  return page.getByRole('button', { name: 'save transaction' });
}

test('keystrokes typed during a save survive its refetch', async ({ page }) => {
  await openGroceries(page, 'Woolworths');
  await freezeRegister(page);
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

  // The refetch gave the editor a fresh base, so the kept edit saves cleanly.
  await saveButton(page).click();
  await expect(saveButton(page)).toBeHidden();
});

test('discard is disabled while a save is in flight', async ({ page }) => {
  await openGroceries(page, 'IGA');
  await freezeRegister(page);
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

test('a failed reconciliation is reported and kept in the draft', async ({ page }) => {
  await openGroceries(page, 'Coles');
  await freezeRegister(page);
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
  await freezeRegister(page);
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

test('posting inputs follow the refetched posting order', async ({ page }) => {
  await openGroceries(page, 'IGA');
  await freezeRegister(page);
  // The server may return postings in any order; reverse them to force it.
  await page.route('**/rpc/get_transaction', async (route) => {
    const response = await route.fetch();
    const tx = await response.json();
    tx.postings.reverse();
    await route.fulfill({ response, json: tx });
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
