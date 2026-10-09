/**
 * Crawls the `/__test` QA index breadth-first and writes every reachable route
 * to `qa/.routes.json`, which the `qa` project reads at collection time.
 *
 * The routes exist only in debug builds of `bc-ui`; a `--release` bundle
 * renders the app's not-found page instead, and this spec fails loudly.
 */
import { expect, test } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { ROUTES_FILE } from './routes.js';

const START   = process.env.QA_ROOT ?? '/__test';
const MISSING = 'QA routes missing — was the web bundle built with --release?';

test('discover QA routes', async ({ page, baseURL }) => {
  // One test visits every route in turn.
  test.setTimeout(300_000);
  const seen  = new Set<string>([START]);
  const queue = [START];

  while (queue.length > 0) {
    const route = queue.shift()!;
    await page.goto(route);
    await expect(page.locator('nav[aria-label="QA navigation"]'), MISSING).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('.not-found'), MISSING).toHaveCount(0);

    const hrefs = await page.locator('a[href^="/__test"]').evaluateAll(
      (anchors) => anchors.map((a) => (a as HTMLAnchorElement).href),
    );
    for (const href of hrefs) {
      const url = new URL(href, baseURL);
      const path = url.pathname.replace(/\/+$/, '');
      if (url.origin === new URL(baseURL!).origin && !seen.has(path)) {
        seen.add(path);
        queue.push(path);
      }
    }
  }

  const routes = [...seen].sort();
  expect(routes.length, MISSING).toBeGreaterThanOrEqual(10);
  writeFileSync(ROUTES_FILE, `${JSON.stringify(routes, null, 2)}\n`);
});
