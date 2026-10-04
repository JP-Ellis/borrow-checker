/**
 * Spec §2's worked example: which legs of "Shared holiday booking" a query
 * lights. The register's account scope joins every query, so the SplitCard
 * leg (outside Assets:Split) always dims here. Each case also narrows the
 * register to the booking alone, which proves that the filtered load has
 * landed before the row is opened.
 */
import { expect, test, type Page } from '@playwright/test';

import { expectSplitRows, openSplitRegister, runQuery, splitRow } from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

/** Opens the booking row and reads each leg's lit state by its account's leaf name. */
async function legStates(page: Page): Promise<Record<string, boolean>> {
  await splitRow(page, 'booking').click();
  const legs = page.getByTestId('posting-row');
  await expect(legs).toHaveCount(5);
  const states: Record<string, boolean> = {};
  for (const leg of await legs.all()) {
    const account = await leg.getByTestId('account-input').inputValue();
    const leaf = account.split(/\s*::\s*|:/).pop() ?? account;
    states[leaf] = (await leg.getAttribute('data-dimmed')) !== 'true';
  }
  return states;
}

/** A query and the legs it lights. */
const CASES: ReadonlyArray<readonly [string, readonly string[]]> = [
  ['account:Assets tag:me', ['Me']],
  ['account:Assets -tag:me -tag:partner status:reconciled', ['Holiday', 'Shared']],
  ['-tag:me -tag:partner status:reconciled', ['Holiday', 'Shared']],
  ['tag:me or tag:partner', ['Me', 'Partner']],
  /* `any:` holds on the transaction; `account:Assets` then picks the legs. */
  ['any:(tag:me) account:Assets', ['Me', 'Partner', 'Holiday', 'Shared']],
];

for (const [query, lit] of CASES) {
  test(`${query} lights ${lit.join(', ')}`, async ({ page }) => {
    await openSplitRegister(page);
    await runQuery(page, query);
    await expectSplitRows(page, ['booking']);

    const expected = Object.fromEntries(
      ['SplitCard', 'Me', 'Partner', 'Holiday', 'Shared'].map((leg) => [leg, lit.includes(leg)]),
    );
    expect(await legStates(page)).toEqual(expected);
  });
}

test('-any:(…) drops the whole transaction', async ({ page }) => {
  await openSplitRegister(page);
  await runQuery(page, 'account:Assets -any:(tag:me)');
  await expectSplitRows(page, ['fuel', 'topUp']);
});
