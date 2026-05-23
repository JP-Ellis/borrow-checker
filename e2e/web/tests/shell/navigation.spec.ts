import { test, expect } from '../fixtures';

const ROUTES = [
  { name: 'dashboard', path: '/' },
  { name: 'accounts',  path: '/accounts' },
  { name: 'budget',    path: '/budget' },
  { name: 'reports',   path: '/reports' },
  { name: 'plugins',   path: '/plugins' },
  { name: 'settings',  path: '/settings' },
] as const;

test.describe('Shell navigation', () => {
  test('TopBar and main area visible on every route', async ({ page }) => {
    for (const { path } of ROUTES) {
      await page.goto(path);
      await expect(page.locator('header')).toBeVisible();
      await expect(page.locator('main')).toBeVisible();
    }
  });

  test('nav contains all six route links', async ({ page }) => {
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'main navigation' });
    await expect(nav).toBeVisible();

    for (const { name } of ROUTES) {
      await expect(nav.getByRole('link', { name })).toBeVisible();
    }

  });

  test('clicking a nav link navigates correctly', async ({ page }) => {
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'main navigation' });

    await nav.getByRole('link', { name: 'accounts' }).click();
    await expect(page).toHaveURL(/\/accounts/);
    await expect(page.locator('main')).toBeVisible();

  });

  test('direct navigation to each route renders a page (no fallback)', async ({ page }) => {
    for (const { path } of ROUTES) {
      await page.goto(path);
      await expect(page.locator('main')).toBeVisible();
      await expect(page.getByText('page not found')).not.toBeVisible();
    }
  });

  test('unknown route shows "page not found" fallback', async ({ page }) => {
    await page.goto('/this-does-not-exist');
    await expect(page.getByText('page not found')).toBeVisible();
  });
});
