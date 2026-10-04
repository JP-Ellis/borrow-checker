/**
 * The hint line and commit gating: errors underline their span and block
 * Enter; warnings show and still commit; a valid term is described; the
 * empty palette and the top bar carry the search copy.
 */
import { expect, test, type Page } from '@playwright/test';

import { chipLabels, openPalette, paletteInput } from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

/**
 * Commits `booking` from the open palette and asserts it is the only chip.
 * The palette handles key presses in order, so once the `booking` chip
 * renders, any commit from an earlier Enter has rendered too.
 */
async function expectOnlyLaterCommit(page: Page): Promise<void> {
  const input = paletteInput(page);
  await input.fill('booking');
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeHidden();
  await expect(chipLabels(page)).toHaveText(['booking']);
}

test('the top bar and the empty palette describe what search does', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('› search, filter, or ⌘K…')).toBeVisible();
  const input = await openPalette(page);
  await expect(input).toHaveAttribute('placeholder', 'search or filter');
  await expect(page.getByTestId('palette-hint')).toHaveText(
    'Type to search descriptions, or a field: account: tag: @payee: …',
  );
});

/** Text with an error, the hint it shows, and the text it underlines. */
const ERRORS: ReadonlyArray<readonly [string, string, string]> = [
  ['@pyee:x', "unknown key '@pyee' (did you mean '@payee'?)", '@pyee'],
  ['account:Nowhere:Else', "no account matches 'Nowhere:Else'", 'Nowhere:Else'],
  ['(coffee', "unclosed '('", '('],
];

for (const [typed, message, underlined] of ERRORS) {
  test(`${typed} blocks the commit`, async ({ page }) => {
    await page.goto('/');
    const input = await openPalette(page);
    await input.fill(typed);
    await expect(page.getByTestId('palette-hint').locator('[data-severity="error"]').first()).toContainText(message);
    await expect(page.getByTestId('palette-highlight').locator('[data-mark="error"]').first()).toHaveText(underlined);

    await input.press('Enter');

    await expect(input).toHaveValue(typed);
    await expect(page.getByTestId('palette-hint').locator('[data-severity="error"]').first()).toContainText(message);
    await expectOnlyLaterCommit(page);
  });
}

test('a warning shows and still commits', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('@odometer:>1');
  await expect(page.getByTestId('palette-hint').locator('[data-severity="warning"]')).toHaveText(
    "1 value of '@odometer' is not a number and was not compared",
  );
  await expect(page.getByTestId('palette-highlight').locator('[data-mark="warning"]').first()).toHaveText('@odometer:');
  await expect(page.getByTestId('palette-highlight').locator('[data-mark="error"]')).toHaveCount(0);

  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');

  await expect(chipLabels(page)).toHaveText(['@odometer:>1']);
  await expect(chipLabels(page).first()).toHaveAttribute('data-severity', 'warning');
});

test('an unknown field warns, commits and marks its chip', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('acount:x'); // spellchecker:disable-line
  await expect(page.getByTestId('palette-hint').locator('[data-severity="warning"]')).toContainText(
    "unknown field 'acount' is ignored", // spellchecker:disable-line
  );
  await expect(page.getByTestId('palette-highlight').locator('[data-mark="error"]')).toHaveCount(0);

  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');

  await expect(chipLabels(page)).toHaveText(['acount:x']); // spellchecker:disable-line
  await expect(chipLabels(page).first()).toHaveAttribute('data-severity', 'warning');
});

/** Valid text and a line the hint shows for it. */
const HINTS: ReadonlyArray<readonly [string, string]> = [
  ['amount:>100', 'Leg amount over 100, either sign, in any currency.'],
  ['-account:Split:Me', 'use -any:(account:Split:Me)'],
  ['@payee:fuel', '@payee contains “fuel”.'],
];

for (const [typed, line] of HINTS) {
  test(`${typed} explains itself`, async ({ page }) => {
    await page.goto('/');
    const input = await openPalette(page);
    await input.fill(typed);
    await expect(page.getByTestId('palette-hint')).toContainText(line);
  });
}

test('a leading > is reserved', async ({ page }) => {
  await page.goto('/');
  const input = await openPalette(page);
  await input.fill('>sync');
  await expect(page.getByTestId('palette-hint')).toHaveText("'>' starts a command; there are none yet");
  await input.press('Enter');
  await expect(input).toHaveValue('>sync');
  await expect(page.getByTestId('palette-hint')).toHaveText("'>' starts a command; there are none yet");
  await expectOnlyLaterCommit(page);
});
