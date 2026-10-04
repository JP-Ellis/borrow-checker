/**
 * Chips: a commit ANDs onto the query; clicking a chip edits it in place; ✕
 * removes it; "edit query" edits the whole query; path chips show the
 * shortest ending, with the full path on hover.
 */
import { expect, test } from '@playwright/test';

import { chipLabels, paletteInput, runQuery } from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

test('a commit ANDs onto the query and brackets an or', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'tag:me or tag:partner');
  await runQuery(page, 'booking');
  await expect(chipLabels(page)).toHaveText(['tag:me or tag:partner', 'booking']);

  await page.getByRole('button', { name: 'edit query' }).click();
  await expect(paletteInput(page)).toHaveValue('(tag:me or tag:partner) booking');
});

test('clicking a chip edits it in place', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'tag:me');
  await runQuery(page, 'status:reconciled');

  await chipLabels(page).first().click();
  const input = paletteInput(page);
  await expect(input).toHaveValue('tag:me');
  await input.fill('tag:partner');
  await input.press('Enter');

  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeHidden();
  await expect(chipLabels(page)).toHaveText(['tag:partner', 'status:reconciled']);
});

test('a blank edit removes the chip', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'tag:me');
  await runQuery(page, 'booking');

  await chipLabels(page).first().click();
  await paletteInput(page).fill('');
  await paletteInput(page).press('Enter');

  await expect(chipLabels(page)).toHaveText(['booking']);
});

test('✕ removes a chip', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'tag:me');
  await runQuery(page, 'status:reconciled');
  await page.getByRole('button', { name: 'remove status:reconciled filter' }).click();
  await expect(chipLabels(page)).toHaveText(['tag:me']);
});

test('"edit query" replaces the whole query', async ({ page }) => {
  await page.goto('/');
  await runQuery(page, 'tag:me');
  await runQuery(page, 'booking');

  await page.getByRole('button', { name: 'edit query' }).click();
  await paletteInput(page).fill('coffee');
  await paletteInput(page).press('Enter');

  await expect(chipLabels(page)).toHaveText(['coffee']);
});

/** A query, its chip's label and its hover title. */
const PATHS: ReadonlyArray<readonly [string, string, string]> = [
  ['account:Assets:Split:Me', 'account:Me', 'account:Assets:Split:Me'],
  ['tag:flights', 'tag:flights', 'tag:holiday:flights'],
];

for (const [query, label, title] of PATHS) {
  test(`${query} chips as ${label} with ${title} on hover`, async ({ page }) => {
    await page.goto('/');
    await runQuery(page, query);
    await expect(chipLabels(page)).toHaveText([label]);
    await expect(page.getByTestId('filter-chips').locator(`[title="${title}"]`)).toHaveCount(1);
  });
}
