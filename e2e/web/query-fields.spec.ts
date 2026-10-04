/**
 * Every built-in field, typed metadata keys (payee included), `or`, grouping,
 * `-` and `-any:(…)`, each run against the Split register's three seeded
 * transactions. Every test only reads the seed.
 */
import { expect, test } from '@playwright/test';

import { expectSplitRows, monthsAgo, openSplitRegister, register, runQuery, type SplitRow } from './support/query.js';

let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

/** A query and the Split rows it leaves. */
const CASES: ReadonlyArray<readonly [string, readonly SplitRow[]]> = [
  ['@payee:"Example Travel"', ['booking']],
  ['@payee:fuel', ['fuel', 'topUp']],
  ['@payee:="Example Fuel Stop"', ['fuel', 'topUp']],
  ['booking', ['booking']],
  ['description:top-up', ['topUp']],
  ['account:Split:Me', ['booking']],
  ['tag:me', ['booking']],
  ['tag:holiday', ['booking']],
  ['status:reconciled', ['booking']],
  ['status:unreconciled', ['fuel', 'topUp']],
  ['amount:>=1000', ['booking']],
  ['amount:<50', ['topUp']],
  ['amount:A$80', ['fuel']],
  /* Every Split leg is in AUD, so each commodity case pairs with a term that
     leaves a set unlike the unfiltered one. */
  ['commodity:AUD amount:<50', ['topUp']],
  ['commodity:JPY or booking', ['booking']],
  ['@odometer:>40000', ['fuel']],
  ['@odometer:*', ['fuel', 'topUp']],
  ['@receipt:R-1002', ['booking']],
  ['@due:*', ['fuel']],
  ['-@due:*', ['booking', 'topUp']],
  ['@deposit:>=A$500', ['booking']],
  [`date:${monthsAgo(2)}-20..`, ['fuel', 'topUp']],
  ['top-up or booking', ['booking', 'topUp']],
  ['(top-up or booking) status:reconciled', ['booking']],
  /* `-tag:me` alone keeps all three rows, because the booking's other legs
     carry no `me` tag. Pinned to the Me leg, the negation drops the booking. */
  ['-tag:me account:Split:Me or top-up', ['topUp']],
  ['-any:(tag:me)', ['fuel', 'topUp']],
];

for (const [query, rows] of CASES) {
  test(`${query} leaves ${rows.join(', ') || 'nothing'}`, async ({ page }) => {
    await openSplitRegister(page);
    await expectSplitRows(page, ['booking', 'fuel', 'topUp']);

    await runQuery(page, query);

    await expectSplitRows(page, rows);
  });
}

test('a tag matches its exact path only with =', async ({ page }) => {
  await openSplitRegister(page);
  await expectSplitRows(page, ['booking', 'fuel', 'topUp']);

  await runQuery(page, 'tag:=holiday');

  await expectSplitRows(page, []);
  await expect(register(page).getByRole('status')).toHaveText('// no transactions');
});
