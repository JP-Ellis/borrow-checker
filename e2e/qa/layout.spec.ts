/**
 * Structural layout checks on every discovered QA route, at a phone and a
 * desktop width in both themes. No screenshots: each test fails with the
 * offending element and its sizes.
 */
import { expect, test } from '@playwright/test';
import { collectLayoutFindings, type Check } from './layout-probe.js';
import { ROUTES, THEMES, requireRoutes, setTheme } from './routes.js';

requireRoutes();

const WIDTHS = [375, 1440] as const;
const HEIGHT = 900;

/** The debug WASM bundle hydrates slowly when every worker loads it at once. */
const HYDRATE_MS = 20_000;

/** Console errors expected per route: the colour page imports APCA from a CDN for its badges. */
const ALLOWED_CONSOLE: Record<string, readonly RegExp[]> = {
  '/__test/fundamentals/colour': [/esm\.sh/],
};

/**
 * Commands that routes' fixtures call on mount. The fixture IDs are not valid
 * TypeIDs, so the server answers 4xx; that failed request is the only error
 * allowed, for exactly these commands. A 5xx from any `/rpc/` call fails.
 */
const LIVE_RPC: Record<string, readonly string[]> = {
  '/__test/page/accounts/full': ['get_account_sparkline'],
  '/__test/page/accounts/hero': ['get_account_sparkline'],
  '/__test/page/budget/budget-detail': ['get_budget_row_transactions', 'list_budget_revisions'],
  '/__test/page/budget/native-period-list': ['get_native_periods'],
};

/** The command an `/rpc/<command>` URL names, if it is one. */
function rpcCommand(url: string): string | undefined {
  return /\/rpc\/([a-z_]+)(?:[?#]|$)/.exec(url)?.[1];
}

/**
 * Exemptions keyed by route, or by `route@width` for one width, each with its
 * reason. A real bug is fixed, or filed and named here. An entry whose check
 * no longer fires fails the test, so a fix removes its exemption.
 */
const SKIP: Record<string, Partial<Record<Check, string>>> = {
  '/__test/page/budget/budget-row@375': { 'page-scroll': 'budget grid has no phone layout (#706)' },
  '/__test/page/budget/budget-tree@375': { 'page-scroll': 'budget grid has no phone layout (#706)' },
  '/__test/page/budget/native-period-list@375': { 'page-scroll': 'budget grid has no phone layout (#706)' },
};

for (const route of ROUTES) {
  for (const width of WIDTHS) {
    for (const theme of THEMES) {
      test(`${route.replace(/^\/__test\/?/, '') || 'index'} @${width} ${theme}`, async ({ page }) => {
        const errors: string[] = [];
        page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
        page.on('response', (r) => {
          const command = rpcCommand(r.url());
          if (command && r.status() >= 500) errors.push(`rpc ${command}: ${r.status()}`);
        });
        page.on('console', (m) => {
          const text = `${m.text()} ${m.location().url}`;
          const command = rpcCommand(m.location().url);
          const liveRpc = command !== undefined && (LIVE_RPC[route] ?? []).includes(command);
          const allowed = (ALLOWED_CONSOLE[route] ?? []).some((r) => r.test(text));
          if (m.type() === 'error' && !liveRpc && !allowed) {
            errors.push(`console.error: ${m.text()}`);
          }
        });

        await page.setViewportSize({ width, height: HEIGHT });
        await page.goto(route);
        await expect(page.locator('nav[aria-label="QA navigation"]')).toBeVisible({ timeout: HYDRATE_MS });
        await expect(page.locator('.not-found')).toHaveCount(0);
        // Lets a fetch made on mount resolve, so an error it causes is captured.
        await page.waitForLoadState('networkidle');
        await setTheme(page, theme);

        const skip = { ...SKIP[route], ...SKIP[`${route}@${width}`] };
        const all = await page.evaluate(collectLayoutFindings);
        const findings = all.filter((f) => !(f.check in skip));
        const stale = Object.keys(skip).filter((check) => !all.some((f) => f.check === check));

        expect.soft(errors, 'render errors').toEqual([]);
        expect.soft(findings.map((f) => `${f.check} at ${f.where}: ${f.detail}`), 'layout findings').toEqual([]);
        expect(stale, 'SKIP entries whose check no longer fires; remove them').toEqual([]);
      });
    }
  }
}
