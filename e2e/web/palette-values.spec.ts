/**
 * Text-value suggestions: a text key's stored values by substring, prefix
 * matches first and then the most used; Tab inserting one; Enter searching
 * the typed text; a value picked after the colon replacing the one there;
 * a failed fetch clearing the values; a late reply for an older needle being
 * dropped.
 */
import { expect, type Request, test } from '@playwright/test';

import { chipLabels, inserts, openPalette, options } from './support/query.js';

const INJECTED = JSON.stringify({ Internal: 'injected failure' });

/** Whether `request` fetches stored values with `needle`. */
function fetches(request: Request, needle: string): boolean {
  return request.url().endsWith('/rpc/metadata_values') && request.postDataJSON()?.needle === needle;
}

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

test('@payee: offers stored payees containing the text, prefix matches first', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@payee:co');
  await expect
    .poll(() => inserts(page))
    .toEqual(['Coles', 'Costco', '"Archive Co"', '"The Coffee Club"', '"Fine Dining Co"', '"Power Company"']);
  await expect(options(page).filter({ hasText: 'Archive Co' })).toContainText('150 uses');
});

test('matching ignores case', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@payee:COFFEE');
  await expect.poll(() => inserts(page)).toEqual(['"The Coffee Club"']);
});

test('Tab inserts a payee, quoted', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@payee:coff');
  await expect.poll(() => inserts(page)).toEqual(['"The Coffee Club"']);
  await input.press('Tab');
  await expect(input).toHaveValue('@payee:"The Coffee Club"');
});

test('Enter searches the typed text without inserting a payee', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@payee:coff');
  await expect.poll(() => inserts(page)).toEqual(['"The Coffee Club"']);
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await expect(chipLabels(page)).toHaveText(['@payee:coff']);
});

test('a payee picked just after the colon replaces the one there', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@payee:"The Coffee Club"');
  await input.press('Home');
  for (let i = 0; i < '@payee:'.length; i += 1) {
    await input.press('ArrowRight');
  }
  const archive = options(page).filter({ hasText: 'Archive Co' });
  await expect(archive).toBeVisible();
  await archive.click();
  await expect(input).toHaveValue('@payee:"Archive Co"');
});

test('closing the palette before values arrive leaves it working', async ({ page }) => {
  await page.goto('/');
  let input = await openPalette(page);
  await input.fill('@payee:co');
  await input.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeHidden();
  input = await openPalette(page);
  await input.fill('@payee:coff');
  await expect.poll(() => inserts(page)).toEqual(['"The Coffee Club"']);
});

test('a failed fetch clears the values, and the next keystroke fetches again', async ({ page }) => {
  let calls = 0;
  await page.route('**/rpc/metadata_values', (route) => {
    calls += 1;
    if (calls === 2) {
      return route.fulfill({ status: 500, contentType: 'application/json', body: INJECTED });
    }
    return route.continue();
  });
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@payee:co');
  await expect.poll(() => inserts(page)).toHaveLength(6);
  const failure = page.waitForResponse((r) => fetches(r.request(), 'coff'));
  await input.fill('@payee:coff');
  expect((await failure).status()).toBe(500);
  await expect.poll(() => inserts(page)).toEqual([]);
  await input.press('e');
  await expect.poll(() => inserts(page)).toEqual(['"The Coffee Club"']);
});

test('a reply for an older needle that lands late is dropped', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  await page.route('**/rpc/metadata_values', async (route) => {
    if (fetches(route.request(), 'coff')) {
      await gate;
    }
    await route.continue();
  });
  await page.goto('/');
  const input = await openPalette(page);
  const held = page.waitForRequest((r) => fetches(r, 'coff'));
  await input.fill('@payee:coff');
  await held;
  await input.fill('@payee:co');
  await expect.poll(() => inserts(page)).toHaveLength(6);
  const late = page.waitForResponse((r) => fetches(r.request(), 'coff'));
  release();
  await late;
  /* The reply resolves in the page after the response event; let it land. */
  await page.evaluate(() => new Promise((r) => setTimeout(r, 200)));
  expect(await inserts(page)).toHaveLength(6);
});
