/**
 * Two people open the seeded "Supermarket" transaction in Groceries. B saves a
 * new description; A's save is refused with the conflict banner and A's draft
 * survives. "Discard and reload" then shows B's version, and A's next save
 * goes through cleanly since the reload gave the editor a fresh base.
 */
import { expect, test, type Page } from '@playwright/test';

async function openSupermarket(page: Page): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  await page.getByRole('navigation', { name: 'account navigation' })
    .getByText('Groceries', { exact: true }).click();
  const register = page.getByLabel('transaction register');
  await register.getByText('Supermarket').first().click();
  await expect(page.getByTestId('status-pill')).toBeVisible();
}

test('a stale save is refused and keeps the draft', async ({ browser }) => {
  const a = await (await browser.newContext()).newPage();
  const b = await (await browser.newContext()).newPage();
  await openSupermarket(a);
  await openSupermarket(b);

  const descB = b.getByPlaceholder('description');
  await descB.fill('Supermarket (edited by B)');
  await b.getByRole('button', { name: 'save transaction' }).click();
  await expect(b.getByRole('button', { name: 'save transaction' })).toBeHidden();

  const descA = a.getByPlaceholder('description');
  await descA.fill('Supermarket (edited by A)');
  await a.getByRole('button', { name: 'save transaction' }).click();

  await expect(a.getByText('This transaction changed since you opened it.')).toBeVisible();
  await expect(descA).toHaveValue('Supermarket (edited by A)');

  // `discard and reload` also fires `on_change_cb`, which sends the account
  // page (register, stats, sparkline) refreshing in the background. Once its
  // `register_page` response lands, `apply_reset` (register_pages.rs) gives
  // this row a new `rev` because its description changed, so `ForEnumerate`
  // remounts it and `TransactionDetail` rebuilds from the fresh `tx`,
  // dropping any edit made in between. This is a known remount bug (tracked
  // separately, out of scope here); the wait below is a workaround so the
  // test exercises the real recovery path instead of racing it.
  const registerRefreshed = a.waitForResponse(
    (res) => res.url().includes('/rpc/register_page') && res.ok(),
  );
  await a.getByRole('button', { name: 'discard and reload' }).click();
  await registerRefreshed;
  await expect(descA).toHaveValue('Supermarket (edited by B)');

  // The reload gave A's editor a fresh base, so a subsequent save is no
  // longer stale and goes through without a conflict.
  await descA.fill('Supermarket (edited by A, take two)');
  await a.getByRole('button', { name: 'save transaction' }).click();
  await expect(a.getByRole('button', { name: 'save transaction' })).toBeHidden();
  await expect(a.getByText('This transaction changed since you opened it.')).toBeHidden();

  await openSupermarket(a);
  await expect(a.getByPlaceholder('description')).toHaveValue('Supermarket (edited by A, take two)');
});
