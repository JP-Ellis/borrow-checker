/**
 * Route list and theme helpers shared by the `qa` specs. `ROUTES` is read
 * synchronously at collection time from the file the `qa-discover` project
 * writes.
 */
import { test, type Page } from '@playwright/test';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export const ROUTES_FILE = join(import.meta.dirname, '.routes.json');

export const ROUTES_MISSING = !existsSync(ROUTES_FILE);

export const ROUTES: readonly string[] = ROUTES_MISSING
  ? []
  : (JSON.parse(readFileSync(ROUTES_FILE, 'utf8')) as string[]);

export const THEMES = ['light', 'dark'] as const;
export type Theme = (typeof THEMES)[number];

/** Declares one failing test when discovery has not run, so a bare run is never green. */
export function requireRoutes(): void {
  if (ROUTES_MISSING) {
    test('QA routes discovered', () => {
      throw new Error(
        `${ROUTES_FILE} is missing: run \`aubx playwright test --project qa-discover\` first, or use \`mise run test:qa\`.`,
      );
    });
  }
}

/** Forces a colour scheme and waits for fonts and two frames, so styles have settled. */
export async function setTheme(page: Page, theme: Theme): Promise<void> {
  await page.emulateMedia({ colorScheme: theme });
  await page.evaluate(async (t) => {
    document.documentElement.setAttribute('data-theme', t);
    await document.fonts.ready;
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  }, theme);
}
