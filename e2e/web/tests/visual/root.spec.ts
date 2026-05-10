import { test, expect } from '../fixtures';

test.describe('Root styling (/__test)', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`ink scale — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test');

      await expect(page.getByText('--bc-ink (primary text)')).toBeVisible();
      await expect(page.getByText('--bc-ink-soft')).toBeVisible();
      await expect(page.getByText('--bc-ink-mute')).toBeVisible();
      await expect(page.getByText('--bc-ink-dim')).toBeVisible();

      // CSS custom property is defined (non-empty)
      const defined = await page.evaluate(() =>
        getComputedStyle(document.documentElement)
          .getPropertyValue('--bc-ink')
          .trim() !== '',
      );
      expect(defined).toBe(true);

      await expect(page).toHaveScreenshot(`root-ink-${scheme}.png`);
    });

    test(`surface scale — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test');

      await expect(page.getByText('bg', { exact: true })).toBeVisible();
      await expect(page.getByText('surface', { exact: true })).toBeVisible();
      await expect(page.getByText('surface-alt', { exact: true })).toBeVisible();
      await expect(page.getByText('surface-hi', { exact: true })).toBeVisible();

      await expect(page).toHaveScreenshot(`root-surface-${scheme}.png`);
    });

    test(`semantic colours — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test');

      await expect(page.getByText('good')).toBeVisible();
      await expect(page.getByText('warn')).toBeVisible();
      await expect(page.getByText('bad')).toBeVisible();
      await expect(page.getByText('accent')).toBeVisible();

      await expect(page).toHaveScreenshot(`root-semantic-${scheme}.png`);
    });

    test(`typography — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test');

      await expect(page.getByText(/Sans: Inter Tight/)).toBeVisible();
      await expect(page.getByText(/Mono: Fira Code/)).toBeVisible();

      await expect(page).toHaveScreenshot(`root-typography-${scheme}.png`);
    });
  }
});
