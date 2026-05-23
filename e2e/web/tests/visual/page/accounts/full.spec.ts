import { test, expect } from '../../../fixtures';

test.describe('Full account view (/__test/page/accounts/full)', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`renders sidebar + hero + register — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test/page/accounts/full');

      await expect(page.getByText('Smart Access').first()).toBeVisible();
      await expect(page.getByText('Coles Carlton', { exact: false })).toBeVisible();

      await expect(page.locator('main')).toHaveScreenshot(`full-${scheme}.png`);
    });
  }

  test('transaction rows have correct ARIA attributes', async ({ page }) => {
    await page.goto('/__test/page/accounts/full');

    const rows = page.locator('[role="button"][aria-expanded]');
    await expect(rows.first()).toBeVisible();
    await expect(rows.first()).toHaveAttribute('aria-expanded', 'false');
  });
});
