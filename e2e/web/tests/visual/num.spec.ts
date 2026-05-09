import { test, expect } from '../fixtures';

const ROWS = [
  'positive',
  'zero',
  'negative',
  'one cent',
  'minus one cent',
  'large',
  'large negative',
] as const;

test.describe('Num component (/__test/num)', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`renders all value rows — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test/num');

      for (const label of ROWS) {
        await expect(page.getByText(label, { exact: true })).toBeVisible();
      }

      await expect(page.locator('table')).toBeVisible();
      await expect(page.locator('thead')).toBeVisible();
      await expect(page.locator('tbody tr')).toHaveCount(7);

      await expect(page).toHaveScreenshot(`num-${scheme}.png`);
    });
  }
});
