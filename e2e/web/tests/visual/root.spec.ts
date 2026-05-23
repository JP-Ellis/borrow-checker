import { test, expect } from '../fixtures';
import type { Page } from '@playwright/test';
import { calcAPCA } from 'apca-w3';

// Helper to resolve a CSS colour string to sRGB hex via canvas
async function toHex(page: Page, cssColour: string): Promise<string> {
  return page.evaluate((colour) => {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext('2d')!;
    ctx.fillStyle = colour;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
    return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
  }, cssColour);
}

test.describe('Root design tokens (/__test/fundamentals/colour + typography)', () => {
  test('ink token is defined', async ({ page }) => {
    await page.goto('/__test/fundamentals/colour');
    await expect(page.getByText('Ink scale — text on bc-bg')).toBeVisible();

    const defined = await page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue('--bc-ink')
        .trim() !== '',
    );
    expect(defined).toBe(true);
  });

  test('surface token is defined', async ({ page }) => {
    await page.goto('/__test/fundamentals/colour');
    await expect(page.getByText('Surface scale — ink on each surface')).toBeVisible();

    const defined = await page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue('--bc-surface')
        .trim() !== '',
    );
    expect(defined).toBe(true);
  });

  test('semantic tone tokens are defined', async ({ page }) => {
    await page.goto('/__test/fundamentals/colour');
    await expect(page.getByText('Semantic tones — with soft background tint')).toBeVisible();

    for (const token of ['--bc-good', '--bc-bad', '--bc-warn']) {
      const defined = await page.evaluate(
        (t) => getComputedStyle(document.documentElement).getPropertyValue(t).trim() !== '',
        token,
      );
      expect(defined, `${token} should be defined`).toBe(true);
    }
  });

  test('typography tokens are defined', async ({ page }) => {
    await page.goto('/__test/fundamentals/typography');
    await expect(page.getByText('Fira Code — bc-font-mono')).toBeVisible();
    await expect(page.getByText('Inter Tight — bc-font-sans')).toBeVisible();

    for (const token of ['--bc-font-mono', '--bc-font-sans']) {
      const defined = await page.evaluate(
        (t) => getComputedStyle(document.documentElement).getPropertyValue(t).trim() !== '',
        token,
      );
      expect(defined, `${token} should be defined`).toBe(true);
    }
  });
});

test.describe('APCA contrast (WCAG 3 draft)', () => {
  test('body text ink-on-bg meets Lc ≥ 60 in light mode', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'light' });
    await page.goto('/__test/fundamentals/colour');

    const { ink, bg } = await page.evaluate(() => {
      const styles = getComputedStyle(document.documentElement);
      return {
        ink: styles.getPropertyValue('--bc-ink').trim(),
        bg:  styles.getPropertyValue('--bc-bg').trim(),
      };
    });

    const hexInk = await toHex(page, ink);
    const hexBg  = await toHex(page, bg);
    const lc = Math.abs(calcAPCA(hexInk, hexBg));
    expect(lc, `ink-on-bg Lc (light): ${lc}`).toBeGreaterThanOrEqual(60);
  });

  test('body text ink-on-bg meets Lc ≥ 60 in dark mode', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.goto('/__test/fundamentals/colour');

    const { ink, bg } = await page.evaluate(() => {
      const styles = getComputedStyle(document.documentElement);
      return {
        ink: styles.getPropertyValue('--bc-ink').trim(),
        bg:  styles.getPropertyValue('--bc-bg').trim(),
      };
    });

    const hexInk = await toHex(page, ink);
    const hexBg  = await toHex(page, bg);
    const lc = Math.abs(calcAPCA(hexInk, hexBg));
    expect(lc, `ink-on-bg Lc (dark): ${lc}`).toBeGreaterThanOrEqual(60);
  });
});
