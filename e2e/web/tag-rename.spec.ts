/**
 * A tag renamed while a transaction carrying it is open must not block the
 * save. A tags the transaction through the picker and reopens it, B renames
 * the tag over RPC, and A's next save goes through.
 */
import { expect, test, type Page } from '@playwright/test';

const ACCOUNT = 'Groceries';
const DESCRIPTION = 'Supermarket';

async function open(page: Page): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  await page.getByRole('navigation', { name: 'account navigation' })
    .getByText(ACCOUNT, { exact: true }).click();
  await page.getByLabel('transaction register').getByText(DESCRIPTION).first().click();
  await expect(page.getByTestId('status-pill')).toBeVisible();
}

test('a tag rename during an open edit does not block the save', async ({ browser }) => {
  const a = await (await browser.newContext()).newPage();
  const b = await (await browser.newContext()).newPage();

  await open(a);
  // The transaction-level picker precedes the per-posting pickers.
  await a.getByTestId('tag-input').first().fill('e2e-rename-probe');
  await a.getByText('create new "e2e-rename-probe"').click();
  await a.getByRole('button', { name: 'save transaction' }).click();
  await expect(a.getByRole('button', { name: 'save transaction' })).toBeHidden();

  await open(a);
  const listed = await b.request.post('/rpc/list_tags', { data: {} });
  expect(listed.ok()).toBeTruthy();
  const tags = await listed.json();
  const probe = tags.find((t: { path: string }) => t.path === 'e2e-rename-probe');
  expect(probe).toBeDefined();
  const renamed = await b.request.post('/rpc/rename_tag', {
    data: { id: probe.id, new_name: 'e2e-rename-probe-renamed' },
  });
  expect(renamed.ok()).toBeTruthy();

  await a.getByPlaceholder('description').fill(`${DESCRIPTION} (after rename)`);
  await a.getByRole('button', { name: 'save transaction' }).click();
  await expect(a.getByRole('button', { name: 'save transaction' })).toBeHidden();
  await expect(a.getByText('This transaction changed since you opened it.')).toBeHidden();
});
