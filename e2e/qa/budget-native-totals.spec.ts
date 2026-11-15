/**
 * Unbudgeted spend under a type root without a report currency totals in its
 * own currencies. The budget QA pages render such rows and header entries
 * from fixtures, so the checks read the formatted lists.
 */
import { expect, test } from '@playwright/test';

/** Three amounts in code order (AUD, CHF, USD), joined with ` · `. */
const THREE_TOTALS = /300\.00[^·]*·[^·]*99\.00[^·]*·[^·]*123\.00/;

test('an unbudgeted row and its root list each native total', async ({ page }) => {
  await page.goto('/__test/page/budget/budget-tree');

  const region = page.getByRole('region', { name: 'Native unbudgeted totals' });
  await expect(region.getByText('↳ unbudgeted', { exact: true })).toBeVisible();
  // The root row and its `↳ unbudgeted` row each show the three totals.
  await expect(region.getByText(THREE_TOTALS)).toHaveCount(2);
});

test('a lone unbudgeted row lists both currencies', async ({ page }) => {
  await page.goto('/__test/page/budget/budget-row');

  await expect(page.getByText(/300\.00[^·]*·[^·]*123\.00/)).toBeVisible();
});

test('the header gathers one root’s currencies under one label', async ({ page }) => {
  await page.goto('/__test/page/budget/header');

  await expect(page.getByText(/300\.00.*,.*123\.00.* unbudgeted expenses/)).toBeVisible();
});
