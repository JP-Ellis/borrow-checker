/**
 * The accounts page's dashboard and register: rolled-up rows, keyboard
 * handling, dates, empty and hidden states. Every test only reads the seed.
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

function sidebar(page: Page) {
  return page.getByRole('navigation', { name: 'account navigation' }).first();
}

function register(page: Page) {
  return page.getByLabel('transaction register');
}

/** Opens the accounts page on `name`, expanding `parents` in the sidebar first. */
async function openAccount(page: Page, name: string, parents: string[] = []): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  for (const parent of parents) {
    const toggle = sidebar(page).getByRole('button', { name: `toggle ${parent}` });
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
      await toggle.click();
    }
  }
  await sidebar(page).getByText(name, { exact: true }).click();
  await expect(page.getByLabel('account dashboard').getByText(name, { exact: true })).toBeVisible();
}

test('a rolled-up parent shows each row its amount and counterpart', async ({ page }) => {
  // Utilities has no postings of its own; every row comes from a child.
  await openAccount(page, 'Utilities', ['Expenses']);
  await expect(page.getByLabel('include sub-accounts')).toBeChecked();

  const row = register(page).locator('[data-tx-id]').filter({ hasText: 'Power Company' }).first();
  await expect(row).toContainText('A$120.00');
  await expect(row.getByTitle('Assets :: Checking')).toBeVisible();
  await expect(register(page).getByTitle('split transaction')).toHaveCount(0);
});

test('Enter on a selected row expands it without opening the add form', async ({ page }) => {
  await openAccount(page, 'Checking');
  await expect(register(page).locator('[data-tx-id]').first()).toBeVisible();

  await register(page).focus();
  await page.keyboard.press('j');
  await page.keyboard.press('Enter');

  await expect(register(page).locator('[data-tx-id][aria-expanded="true"]')).toHaveCount(1);
  await expect(page.getByTestId('add-transaction-form')).toHaveCount(0);
});

test('register dates are ISO, dropping the year a period pins', async ({ page }) => {
  await openAccount(page, 'Checking');
  const firstDate = register(page).locator('[data-tx-id]').first().locator('span').first();
  await expect(firstDate).toHaveText(/^\d{4}-\d{2}-\d{2}$/);

  // The seed spans the months before today, so the current calendar year has rows.
  await register(page).getByLabel('period').selectOption('calendar_year');
  await expect(firstDate).toHaveText(/^\d{2}-\d{2}$/);
});

test('the header shows the full path and offers rollup only with children', async ({ page }) => {
  await openAccount(page, 'Electricity', ['Expenses', 'Utilities']);
  await expect(page.getByText('Expenses :: Utilities :: Electricity', { exact: true })).toBeVisible();
  await expect(page.getByLabel('include sub-accounts')).toHaveCount(0);

  await sidebar(page).getByText('Utilities', { exact: true }).click();
  await expect(page.getByLabel('include sub-accounts')).toBeVisible();
});

test('the dashboard offers no inert actions or placeholder status', async ({ page }) => {
  await openAccount(page, 'Checking');
  await expect(page.getByRole('button', { name: /^\+ transaction/ })).toBeVisible();
  await expect(page.getByRole('button', { name: /^reconcile/ })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^import/ })).toHaveCount(0);
  await expect(page.getByText('• reconciled')).toHaveCount(0);
});

test('an account with no transactions says so', async ({ page }) => {
  await openAccount(page, 'Uncategorised', ['Expenses']);
  await expect(register(page).getByRole('status').filter({ hasText: '// no transactions' })).toBeVisible();
});

test('turning the balance off removes the column', async ({ page }) => {
  await openAccount(page, 'Checking');
  const header = register(page).getByText('balance', { exact: true });
  await expect(header).toBeVisible();

  const toggle = register(page).getByRole('button', { name: /^balance:/ });
  while ((await toggle.textContent()) !== 'balance: off') {
    await toggle.click();
  }
  await expect(header).toBeHidden();
});

test('the collapsed rail names accounts by full path in tree order', async ({ page }) => {
  await openAccount(page, 'Checking');
  await page.getByRole('button', { name: 'toggle sidebar' }).click();

  const dots = sidebar(page).locator('a[aria-label]');
  await expect(dots.first()).toHaveAttribute('aria-label', 'Assets');
  await expect(sidebar(page).getByLabel('Expenses :: Utilities :: Electricity')).toHaveCount(1);
});

test('the browser tab names the shown account', async ({ page }) => {
  await openAccount(page, 'Checking');
  await expect(page).toHaveTitle('Checking · borrow-checker');
});
