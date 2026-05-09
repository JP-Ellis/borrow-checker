import { test, expect } from '../fixtures';

test.describe('StatusPill component (/__test/status-pill)', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`renders all tones — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test/status-pill');

      const main = page.locator('main');
      for (const label of ['synced', 'pending', 'error', 'good', 'warn', 'bad']) {
        await expect(main.getByText(label, { exact: true })).toBeVisible();
      }

      await expect(page).toHaveScreenshot(`status-pill-${scheme}.png`);
    });
  }
});
