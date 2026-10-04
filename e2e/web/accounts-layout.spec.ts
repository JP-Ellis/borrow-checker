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

test('at 1280 px every tab is inline and the overflow button is hidden', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('/');
  await expect(page.getByTestId('nav-more')).toBeHidden();
  await page.getByTestId('nav-settings').filter({ visible: true }).click();
  await expect(page).toHaveURL(/\/settings$/);
});

test('at 400 px settings is reachable through the overflow menu', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  await expect(page.getByTestId('nav-accounts')).toBeVisible();
  await page.getByTestId('nav-more').click();
  await page.getByTestId('nav-settings').filter({ visible: true }).click();
  await expect(page).toHaveURL(/\/settings$/);
  // Choosing an item closes the menu; the button marks the active overflow route.
  await expect(page.locator('#bc-nav-more')).toBeHidden();
  await expect(page.getByTestId('nav-more')).toHaveClass(/top-bar__tab--active/);
});

test('at 400 px the top bar fits without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  const fits = await page.getByRole('banner').evaluate((el) => el.scrollWidth <= el.clientWidth);
  expect(fits).toBe(true);
  await expect(page.getByRole('button', { name: /open command palette/ })).toBeVisible();
});

test('at 400 px active filters fold into one chip that opens them', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  await expect(page.getByTestId('filter-more')).toHaveCount(0);
  await page.getByRole('button', { name: /open command palette/ }).click();
  await page.keyboard.type('status:unreconciled');
  await page.locator('#palette-listbox div[role="option"]').first().waitFor();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('filter-more')).toHaveText('1 filter');
  await page.getByTestId('filter-more').click();
  await expect(page.locator('#bc-filter-more')).toContainText('status: unreconciled');
});
