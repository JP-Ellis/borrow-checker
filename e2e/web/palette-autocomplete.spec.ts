/**
 * The palette's dropdown and highlighting: keys with their types, operators
 * for a field's type, values for paths, statuses, amounts and dates, Tab and
 * Enter, and a highlight that scrolls with long input.
 */
import { expect, test } from '@playwright/test';

import {
  chipLabels,
  hint,
  inserts,
  monthsAgo,
  openPalette,
  options,
  paletteInput,
  serveCatalog,
} from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

test('@ lists metadata keys with their types', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.pressSequentially('@');
  for (const [key, type] of [['@payee', 'text'], ['@odometer', 'number'], ['@due', 'date'], ['@deposit', 'amount']]) {
    await expect(options(page).filter({ hasText: key })).toContainText(type);
  }
});

test('Tab completes a field and the dropdown moves on to its operators', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.pressSequentially('acc');
  await input.press('Tab');
  await expect(input).toHaveValue('account:');
  await expect(options(page).first()).toContainText('this account only');
});

/** Text typed into the palette, and the first inserts the dropdown offers. */
const OFFERS: ReadonlyArray<readonly [string, readonly string[]]> = [
  ['amount:', ['=', '>', '>=', '<', '<=']],
  ['@payee:', ['=', '*']],
  ['tag:', ['=', '*']],
  ['status:', ['unreconciled', 'flagged', 'reconciled', 'balanced', 'unbalanced']],
  ['account:split:m', ['Me']],
  ['tag:fli', ['flights']],
  ['amount:20.5', ['"20.5 AUD"']],
];

for (const [typed, first] of OFFERS) {
  test(`${typed} offers ${first.join(' ')} first`, async ({ page }) => {
    await page.goto('/');
    const input = await openPalette(page);
    await input.fill(typed);
    await expect.poll(async () => (await inserts(page)).slice(0, first.length)).toEqual([...first]);
  });
}

test('a date offers this month and this year', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('date:');
  const month = monthsAgo(0);
  await expect.poll(() => inserts(page)).toEqual(expect.arrayContaining([month, month.slice(0, 4)]));
});

test('an account row shows the full path and Enter commits its ending', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('account:split:m');
  await expect(options(page).first()).toHaveText('Assets :: Split :: Me');
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');
  await expect(chipLabels(page)).toHaveText(['account:Me']);
});

test('clicking a suggestion replaces the token under the caret', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('booking account:split:m tag:me');
  /* The caret sits at the end of `account:split:m`. */
  await input.evaluate((el: HTMLInputElement) => el.setSelectionRange(22, 22));
  await input.press('ArrowRight');
  await expect(options(page).first()).toHaveText('Assets :: Split :: Me');

  await options(page).first().click();

  await expect(input).toHaveValue('booking account:Me tag:me');
});

test('each token kind is highlighted', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('-tag:me or @payee:"Example Travel" amount:>=100 coffee');
  const highlight = page.getByTestId('palette-highlight');
  const runs: ReadonlyArray<readonly [string, string]> = [
    ['keyword', '-'],
    ['field', 'tag:'],
    ['value', 'me'],
    ['keyword', 'or'],
    ['key', '@payee:'],
    ['value', '"Example Travel"'],
    ['field', 'amount:'],
    ['operator', '>='],
    ['value', '100'],
    ['text', 'coffee'],
  ];
  for (const [kind, text] of runs) {
    await expect(highlight.locator(`[data-kind="${kind}"]`, { hasText: text }).first()).toBeVisible();
  }
  await expect(paletteInput(page)).toBeFocused();
});

test('the highlight scrolls with a query longer than the input', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill(Array.from({ length: 40 }, (_, i) => `word${i}`).join(' '));
  await input.press('End');
  await expect
    .poll(() =>
      page.evaluate(() => {
        const field = document.querySelector('[role="combobox"]') as HTMLInputElement;
        const backdrop = document.querySelector('[data-testid="palette-highlight"]') as HTMLElement;
        return [field.scrollLeft > 0, backdrop.scrollLeft === field.scrollLeft];
      }),
    )
    .toEqual([true, true]);
});

test('an ambiguous ending blocks and lists its candidates', async ({ page }) => {
  const served = await serveCatalog(page);
  served.extra.push({ id: 'account_fake_income_groceries', path: ['Income', 'Groceries'] });
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('account:groceries');
  await expect(options(page)).toHaveCount(2);

  await input.press('Enter');

  await expect(input).toHaveValue('account:groceries');
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeVisible();
  await expect(hint(page)).toContainText("'groceries' is ambiguous: Expenses:Groceries, Income:Groceries");
  await expect(chipLabels(page)).toHaveCount(0);
});

test('a unique ending still commits its full path on Enter', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('account:split:m');
  await expect(options(page)).toHaveCount(1);

  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');

  await expect(chipLabels(page)).toHaveText(['account:Me']);
  await expect(page.getByTestId('filter-chips').locator('[title="account:Assets:Split:Me"]')).toHaveCount(1);
});
