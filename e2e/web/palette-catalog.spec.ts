/**
 * The palette against the query catalog: two `account:` terms that no leg can
 * satisfy, a catalog that never loads, the server's own rejection reaching the
 * register toast and the budget page, and a catalog that gains an account
 * between two opens. Only the catalog responses are altered, in the page's
 * routes; the database is never written.
 */
import { expect, test } from '@playwright/test';

import {
  chipLabels,
  failCatalog,
  hint,
  openPalette,
  openSplitRegister,
  options,
  paletteInput,
  runQuery,
  serveCatalog,
} from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  /* An aborted catalog fetch logs a console warning, not a page error. */
  expect(pageErrors).toEqual([]);
});

test('two account terms on unrelated subtrees both carry a warning', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'account:Checking');
  await runQuery(page, 'account:Groceries');

  await expect(chipLabels(page)).toHaveText(['account:Checking', 'account:Groceries']);
  for (const chip of await chipLabels(page).all()) {
    await expect(chip).toHaveAttribute('data-severity', 'warning');
  }
});

test('an account term and its descendant draw no warning', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'account:Assets');
  await runQuery(page, 'account:Checking');

  await expect(chipLabels(page)).toHaveText(['account:Assets', 'account:Checking']);
  for (const chip of await chipLabels(page).all()) {
    await expect(chip).not.toHaveAttribute('data-severity', /.+/);
  }
});

test('without a catalog the palette says so and still commits the term', async ({ page }) => {
  await failCatalog(page);
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('account:Split:Me');

  await expect(hint(page)).toContainText("Couldn't load accounts, tags, commodities and keys; the server will check this query.");
  await expect(page.getByTestId('palette-highlight').locator('[data-mark="error"]')).toHaveCount(0);

  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');

  await expect(chipLabels(page)).toHaveText(['account:Split:Me']);
});

test('an error no catalog could change still blocks without one', async ({ page }) => {
  await failCatalog(page);
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('acount:x');

  await expect(hint(page).locator('[data-severity="error"]').first()).toContainText("unknown field 'acount'");
  await input.press('Enter');

  await expect(input).toHaveValue('acount:x');
  await expect(chipLabels(page)).toHaveCount(0);
});

test('the register toast quotes the server\'s complaint and "edit query" opens the current query', async ({ page }) => {
  await failCatalog(page);
  await openSplitRegister(page);

  await runQuery(page, 'account:NoSuchAccount');

  const toasts = page.getByRole('status').filter({ hasText: 'The filter didn’t run' });
  await expect(toasts).toContainText("no account matches 'NoSuchAccount' (at “NoSuchAccount”)");

  /* A conjunct added after the failure is part of the current query. */
  await runQuery(page, 'booking');
  await toasts.getByRole('button', { name: 'edit query' }).first().click();

  await expect(paletteInput(page)).toHaveValue('account:NoSuchAccount booking');
});

test('the budget page quotes the same complaint in its banner', async ({ page }) => {
  await failCatalog(page);
  await page.goto('/');
  await runQuery(page, 'account:NoSuchAccount');

  await page.getByRole('navigation', { name: 'main navigation' }).getByRole('link', { name: 'Budget' }).click();

  await expect(page.getByText('The filter didn’t run')).toContainText(
    "no account matches 'NoSuchAccount' (at “NoSuchAccount”)",
  );
});

test('an account added after the page loaded resolves on the next open', async ({ page }) => {
  const served = await serveCatalog(page);
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('account:FakeZzzFund');
  await expect(hint(page).locator('[data-severity="error"]').first()).toContainText(
    "no account matches 'FakeZzzFund'",
  );
  await input.press('Escape');

  served.extra.push({ id: 'account_fake_zzz_fund', path: ['Assets', 'FakeZzzFund'] });
  const refetched = page.waitForResponse('**/rpc/query_catalog');
  const reopened = await openPalette(page);
  await refetched;
  await reopened.fill('account:FakeZzz');

  await expect(options(page).first()).toHaveText('Assets :: FakeZzzFund');
  await reopened.fill('account:FakeZzzFund');
  await expect(hint(page).locator('[data-severity="error"]')).toHaveCount(0);
});
