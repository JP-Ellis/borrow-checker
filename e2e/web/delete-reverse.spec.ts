/**
 * Delete and reverse from the register detail. `page.route` stands in for the
 * server, so the shared ledger is never mutated: each test asserts the gate's
 * wording and the arguments the confirm sends.
 */
import { expect, test, type Page } from '@playwright/test';

async function openSupermarket(page: Page): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  await page.getByRole('navigation', { name: 'account navigation' })
    .getByText('Groceries', { exact: true }).click();
  await page.getByLabel('transaction register').getByText('Supermarket').first().click();
  await expect(page.getByTestId('status-pill')).toBeVisible();
}

test('an imported transaction defaults to skipping on re-import', async ({ page }) => {
  await page.route('**/rpc/transaction_provenance', (route) =>
    route.fulfill({ json: { rows: 2, accounts: ['Assets:Bank:Everyday', 'Expenses:Groceries'] } }));
  let sent: unknown = null;
  await page.route('**/rpc/delete_transaction', async (route) => {
    sent = route.request().postDataJSON();
    await route.fulfill({ json: { references_kept: 2, references_forgotten: 0 } });
  });
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('Imported from Assets:Bank:Everyday, Expenses:Groceries.')).toBeVisible();
  await page.getByRole('button', { name: 'Cancel' }).click();
  expect(sent).toBeNull();

  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await page.getByRole('button', { name: 'Delete, skip on re-import' }).click();
  await expect(page.getByText(/^Deleted\. A re-import will skip it;/)).toBeVisible();
  expect(sent).toMatchObject({ forget_provenance: false });
});

test('allow re-import sends forget_provenance', async ({ page }) => {
  await page.route('**/rpc/transaction_provenance', (route) =>
    route.fulfill({ json: { rows: 1, accounts: ['Assets:Bank:Everyday'] } }));
  let sent: unknown = null;
  await page.route('**/rpc/delete_transaction', async (route) => {
    sent = route.request().postDataJSON();
    await route.fulfill({ json: { references_kept: 0, references_forgotten: 1 } });
  });
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await page.getByRole('button', { name: 'Delete, allow re-import' }).click();
  await expect(page.getByText('Deleted. A re-import will recreate it.')).toBeVisible();
  expect(sent).toMatchObject({ forget_provenance: true });
});

test('a failed delete keeps the detail open and says why', async ({ page }) => {
  await page.route('**/rpc/transaction_provenance', (route) =>
    route.fulfill({ json: { rows: 0, accounts: [] } }));
  await page.route('**/rpc/delete_transaction', (route) =>
    route.fulfill({ status: 404, json: { NotFound: 'transaction' } }));
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('Delete this transaction?')).toBeVisible();
  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText("Couldn't delete: it no longer exists.")).toBeVisible();
  await expect(page.getByTestId('status-pill')).toBeVisible();
});

test('reverse warns when already reversed, then confirms', async ({ page }) => {
  let reversed = false;
  await page.route('**/rpc/reverse_transaction', async (route) => {
    reversed = true;
    await route.fulfill({ json: 'transaction_01h455vb4pex5vsknk084sn02q' });
  });
  await page.route('**/rpc/get_transaction_audit', (route) =>
    route.fulfill({
      json: [{ time: '2000-01-01T00:00:00Z', kind: 'reverse', message: 'reversed by x' }],
    }));
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Reverse', exact: true }).click();
  await expect(page.getByText('This transaction has already been reversed.')).toBeVisible();
  await page.getByRole('button', { name: 'Reverse', exact: true }).click();
  await expect(page.getByText('Reversal added.')).toBeVisible();
  expect(reversed).toBe(true);
});

test('reverse waits for its reversal check', async ({ page }) => {
  let release!: () => void;
  const answered = new Promise<void>((resolve) => { release = resolve; });
  await page.route('**/rpc/get_transaction_audit', async (route) => {
    await answered;
    await route.fulfill({ json: [] });
  });
  await page.route('**/rpc/reverse_transaction', (route) =>
    route.fulfill({ json: 'transaction_01h455vb4pex5vsknk084sn02q' }));
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Reverse', exact: true }).click();
  await expect(page.getByText('Checking for an earlier reversal…')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reverse', exact: true })).toBeDisabled();
  release();
  await expect(page.getByText(/^Add a reversing transaction dated /)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reverse', exact: true })).toBeEnabled();
  await expect(page.getByText('This transaction has already been reversed.')).toHaveCount(0);
});

test('a failed reversal check warns without blocking', async ({ page }) => {
  await page.route('**/rpc/get_transaction_audit', (route) =>
    route.fulfill({ status: 500, json: { Internal: 'disk full' } }));
  let reversed = false;
  await page.route('**/rpc/reverse_transaction', async (route) => {
    reversed = true;
    await route.fulfill({ json: 'transaction_01h455vb4pex5vsknk084sn02q' });
  });
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Reverse', exact: true }).click();
  await expect(page.getByText("Couldn't check whether this was already reversed.")).toBeVisible();
  await page.getByRole('button', { name: 'Reverse', exact: true }).click();
  await expect(page.getByText('Reversal added.')).toBeVisible();
  expect(reversed).toBe(true);
});

test('Escape closes the delete gate and keeps the detail open', async ({ page }) => {
  await page.route('**/rpc/transaction_provenance', (route) =>
    route.fulfill({ json: { rows: 1, accounts: ['Assets:Bank:Everyday'] } }));
  let sent = false;
  await page.route('**/rpc/delete_transaction', async (route) => {
    sent = true;
    await route.fulfill({ json: { references_kept: 1, references_forgotten: 0 } });
  });
  await openSupermarket(page);

  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Delete, skip on re-import' })).toBeFocused();
  await page.keyboard.press('Escape');

  await expect(page.getByRole('button', { name: 'Delete, skip on re-import' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Delete', exact: true })).toBeVisible();
  await expect(page.getByTestId('status-pill')).toBeVisible();
  expect(sent).toBe(false);
});

for (const action of ['delete', 'reverse'] as const) {
  test(`Save is disabled while a ${action} is in flight`, async ({ page }) => {
    await page.route('**/rpc/transaction_provenance', (route) =>
      route.fulfill({ json: { rows: 0, accounts: [] } }));
    await page.route('**/rpc/get_transaction_audit', (route) => route.fulfill({ json: [] }));
    let saved = false;
    await page.route('**/rpc/edit_transaction', async (route) => {
      saved = true;
      await route.fulfill({ json: null });
    });
    let release!: () => void;
    const answered = new Promise<void>((resolve) => { release = resolve; });
    await page.route(`**/rpc/${action}_transaction`, async (route) => {
      await answered;
      await route.fulfill({ status: 404, json: { NotFound: 'transaction' } });
    });
    await openSupermarket(page);

    await page.getByPlaceholder('description').fill('Supermarket (draft)');
    const save = page.getByRole('button', { name: 'save transaction' });
    await expect(save).toBeEnabled();

    const verb = action === 'delete' ? 'Delete' : 'Reverse';
    await page.getByRole('button', { name: verb, exact: true }).click();
    await page.getByRole('button', { name: verb, exact: true }).click();
    await expect(save).toBeDisabled();

    release();
    await expect(page.getByText(`Couldn't ${action}: it no longer exists.`)).toBeVisible();
    await expect(save).toBeEnabled();
    expect(saved).toBe(false);
  });
}
