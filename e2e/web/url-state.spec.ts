/**
 * URL state on the web build: hand-edited links canonicalise in place, the
 * query survives reserved characters, and the budget page keeps the filter.
 * The last location replays only on a tab's first load at bare `/`.
 */
import { expect, test } from '@playwright/test';

import { chipLabels, runQuery } from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

test('a hand-edited window snaps to its period without a history entry', async ({ page }) => {
  await page.goto('/budget');
  await page.goto('/accounts?period=monthly&start=2025-12-15');
  await expect(page).toHaveURL(/\/accounts\?period=monthly&start=2025-12-01$/);
  await page.goBack();
  await expect(page).toHaveURL(/\/budget$/);
});

test('a half-specified window falls back to all time', async ({ page }) => {
  await page.goto('/accounts?period=monthly');
  await expect(page).toHaveURL(/\/accounts$/);
});

test('reserved characters in the query survive a reload and a commit', async ({ page }) => {
  const query = '"Coffee & Co #1"';
  await page.goto(`/accounts?q=${encodeURIComponent(query)}`);
  await expect(chipLabels(page)).toHaveCount(1);

  await page.reload();
  expect(new URL(page.url()).searchParams.get('q')).toBe(query);

  await runQuery(page, 'booking');
  const q = new URL(page.url()).searchParams.get('q') ?? '';
  expect(q).toContain('Coffee & Co #1');
  expect(q).toContain('booking');
});

test('the budget page keeps the filter across a reload', async ({ page }) => {
  await page.goto('/budget');
  await runQuery(page, 'tag:me');
  await expect(chipLabels(page)).toHaveText(['tag:me']);
  await page.reload();
  await expect(chipLabels(page)).toHaveText(['tag:me']);
});

test('a new tab at / replays the last location and a reload there does not', async ({ context, page }) => {
  const restored = /\/accounts\?q=tag%3Ame$/;
  await page.goto('/');
  await expect(page.getByRole('navigation', { name: 'main navigation' })).toBeVisible();

  const other = await context.newPage();
  other.on('pageerror', (e) => pageErrors.push(e));
  await other.goto('/accounts');
  await runQuery(other, 'tag:me');
  await expect(other).toHaveURL(restored);

  const fresh = await context.newPage();
  fresh.on('pageerror', (e) => pageErrors.push(e));
  await fresh.goto('/');
  await expect(fresh).toHaveURL(restored);
  await expect(fresh.getByRole('status').filter({ hasText: 'Restored filter: tag:me' })).toBeVisible();

  // localStorage still holds the other tabs' location; this tab has loaded before.
  await page.reload();
  await expect(page.getByRole('navigation', { name: 'main navigation' })).toBeVisible();
  await page.waitForTimeout(1000);
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole('status').filter({ hasText: 'Restored filter' })).toHaveCount(0);
});
