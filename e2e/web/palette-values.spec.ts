/**
 * Text-value suggestions: a text key's stored values by substring, prefix
 * matches first and then the most used; Tab inserting one; Enter searching
 * the typed text; a value picked after the colon replacing the one there.
 */
import { expect, test } from '@playwright/test';

import { chipLabels, inserts, openPalette, options } from './support/query.js';

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
    .toEqual(['Coles', '"Archive Co"', '"The Coffee Club"', '"Fine Dining Co"', '"Power Company"']);
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
