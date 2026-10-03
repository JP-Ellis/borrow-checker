/**
 * The register keeps an open editor mounted across every refetch: a save,
 * a page append, a rollup toggle. Assertions hold element handles across
 * the refresh: a remount would detach them.
 */
import { expect, test, type Locator, type Page } from '@playwright/test';

const MAIN = '[data-testid="accounts-main-scroll"]';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

function register(page: Page) {
  return page.getByLabel('transaction register');
}

function rows(page: Page) {
  return register(page).locator('[data-tx-id]');
}

async function openAccount(page: Page, name: string, parents: string[] = []): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  const nav = page.getByRole('navigation', { name: 'account navigation' }).first();
  for (const parent of parents) {
    const toggle = nav.getByRole('button', { name: `toggle ${parent}` });
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  }
  await nav.getByText(name, { exact: true }).click();
  await expect(rows(page).first()).toBeVisible();
  await expect(register(page)).toHaveAttribute('aria-busy', 'false');
}

/** Clicks without Playwright's scroll-into-view, which would move the view. */
async function clickNoScroll(target: Locator): Promise<void> {
  await target.evaluate((el) => (el as HTMLElement).click());
}

async function settled(page: Page): Promise<void> {
  await expect(page.getByRole('button', { name: 'save transaction' })).toBeHidden();
  await expect(register(page)).toHaveAttribute('aria-busy', 'false');
}

test('typing during a save keeps the same input, its text and focus', async ({ page }) => {
  await openAccount(page, 'Dining');
  await rows(page).first().click();
  const desc = page.getByPlaceholder('description');
  const handle = await desc.elementHandle();

  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  await page.route('**/rpc/get_transaction', async (route) => { await gate; await route.continue(); });

  await desc.fill('Coffee with a colleague');
  await page.getByRole('button', { name: 'save transaction' }).click();
  await desc.focus();
  await page.keyboard.type(' and cake');
  const refreshed = page.waitForResponse('**/rpc/register_page');
  release();
  await refreshed;
  await expect(register(page)).toHaveAttribute('aria-busy', 'false');

  expect(await handle!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(desc).toHaveValue('Coffee with a colleague and cake');
  expect(await handle!.evaluate((el) => el === document.activeElement)).toBe(true);
});

test('a save updates the row header in place', async ({ page }) => {
  await openAccount(page, 'Dining');
  const row = rows(page).first();
  const handle = await row.elementHandle();
  const before = {
    flagged: await row.getByLabel('flagged').count(),
    unrec: await row.getByLabel('unreconciled').count(),
  };
  await row.click();
  await page.getByTestId('status-pill').click();
  await page.getByRole('button', { name: 'save transaction' }).click();
  await settled(page);

  // The pill cycles unreconciled → flagged → reconciled → unreconciled.
  const expected = before.unrec ? { flagged: 1, unrec: 0 }
    : before.flagged ? { flagged: 0, unrec: 0 }
    : { flagged: 0, unrec: 1 };
  expect(await handle!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(row.getByLabel('flagged')).toHaveCount(expected.flagged);
  await expect(row.getByLabel('unreconciled')).toHaveCount(expected.unrec);
});

test('append keeps an open editor', async ({ page }) => {
  await openAccount(page, 'Archive');
  await expect(rows(page)).toHaveCount(100);
  await rows(page).nth(5).click();
  const desc = page.getByPlaceholder('description');
  const handle = await desc.elementHandle();
  await desc.fill('Archive draft');

  await clickNoScroll(page.getByTestId('load-more'));
  await expect(rows(page)).toHaveCount(150);

  expect(await handle!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(desc).toHaveValue('Archive draft');
});

test('re-sorted row keeps its editor', async ({ page }) => {
  await openAccount(page, 'Archive');
  const target = rows(page).nth(20);
  const id = await target.getAttribute('data-tx-id');
  await target.click();
  const dateInput = page.getByPlaceholder('YYYY-MM-DD');
  const handle = await dateInput.elementHandle();

  // Archive never dates past today and sorts newest first, so tomorrow
  // moves the row to the top.
  const tomorrow = new Date();
  tomorrow.setDate(tomorrow.getDate() + 1);
  await dateInput.fill(tomorrow.toISOString().slice(0, 10));
  await clickNoScroll(page.getByRole('button', { name: 'save transaction' }));
  await settled(page);

  await expect(rows(page).first()).toHaveAttribute('data-tx-id', id!);
  expect(await handle!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(page.locator(`[data-tx-id="${id}"]`)).toHaveAttribute('aria-expanded', 'true');
});

test('a rollup toggle closes an editor whose row leaves, without a panic', async ({ page }) => {
  await openAccount(page, 'Utilities', ['Expenses']);
  await expect(page.getByLabel('include sub-accounts')).toBeChecked();
  await rows(page).first().click();
  await page.getByPlaceholder('description').fill('Utilities draft');

  // Utilities has no postings of its own: off drops every row, open one included.
  await page.getByLabel('include sub-accounts').uncheck();
  await expect(register(page)).toHaveAttribute('aria-busy', 'false');
  await expect(page.getByPlaceholder('description')).toBeHidden();

  await page.getByLabel('include sub-accounts').check();
  await expect(rows(page).first()).toBeVisible();
});

test('a save holds the edited row at its offset', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await openAccount(page, 'Archive');
  const target = rows(page).nth(60);
  const id = await target.getAttribute('data-tx-id');
  await target.evaluate((r) => r.scrollIntoView({ block: 'center' }));
  await clickNoScroll(target);
  await page.getByPlaceholder('description').fill('Archive offset check');

  const offset = () => page.evaluate(([main, txId]) => {
    const el = document.querySelector(main)!;
    const row = document.querySelector(`[data-tx-id="${txId}"]`)!;
    return row.getBoundingClientRect().top - el.getBoundingClientRect().top;
  }, [MAIN, id] as const);
  const before = await offset();
  await clickNoScroll(page.getByRole('button', { name: 'save transaction' }));
  await settled(page);
  await page.waitForTimeout(100);
  expect(Math.abs((await offset()) - before)).toBeLessThan(2);
});
