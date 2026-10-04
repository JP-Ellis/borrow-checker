/**
 * The accounts page and top bar across viewport widths: what stays in flow,
 * which tabs and register columns survive, and what holds still on scroll.
 * Every test only reads the seed.
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

// MARK: Top bar

test('the top bar shows no status pill', async ({ page }) => {
  await page.goto('/');
  const topBar = page.getByRole('banner');
  await expect(topBar).toBeVisible();
  await expect(topBar).not.toContainText('pending');
});
