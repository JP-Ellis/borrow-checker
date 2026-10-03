/**
 * The editor tells its own echoes from foreign changes by comparing the
 * register's copy of a transaction with copies `get_transaction` returned.
 * Both commands must therefore serialise one stored state identically.
 */
import { expect, test, type Page } from '@playwright/test';

async function capturedRegister(page: Page, account: string, parents: string[] = []) {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  const nav = page.getByRole('navigation', { name: 'account navigation' }).first();
  for (const parent of parents) {
    const toggle = nav.getByRole('button', { name: `toggle ${parent}` });
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  }
  const response = page.waitForResponse('**/rpc/register_page');
  await nav.getByText(account, { exact: true }).click();
  return (await (await response).json()) as { rows: { transaction: { id: string } }[] };
}

for (const [account, parents] of [['Groceries', []], ['Utilities', ['Expenses']]] as const) {
  test(`register rows equal get_transaction on ${account}`, async ({ page }) => {
    const pageJson = await capturedRegister(page, account, [...parents]);
    expect(pageJson.rows.length).toBeGreaterThan(0);
    for (const row of pageJson.rows) {
      const res = await page.request.post('/rpc/get_transaction', { data: { id: row.transaction.id } });
      expect(res.ok()).toBe(true);
      expect(await res.json()).toEqual(row.transaction);
    }
  });
}
