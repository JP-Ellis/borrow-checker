import { test, expect } from '../fixtures';

test.describe('TagToken component (/__test/tag-token)', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`renders all tokens — ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto('/__test/tag-token');

      const main = page.locator('main');
      for (const label of [
        'default',
        'expenses:food',
        'a:very:deeply:nested:path',
        'keyword',
        'string',
        'number',
        'type',
        'fn',
        'comment',
      ]) {
        await expect(main.getByText(label, { exact: true })).toBeVisible();
      }

      await expect(page).toHaveScreenshot(`tag-token-${scheme}.png`);
    });
  }
});
