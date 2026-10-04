/**
 * Helpers for the query palette and the seed's Split register, shared by the
 * query specs. Every helper only reads the seed.
 */
import { expect, type Locator, type Page } from '@playwright/test';

/** The palette's input. */
export function paletteInput(page: Page): Locator {
  return page.getByRole('combobox', { name: 'Search filters' });
}

/** The palette's dropdown rows. */
export function options(page: Page): Locator {
  return page.locator('#palette-listbox [role="option"]');
}

/** Each dropdown row's inserted text, in order. */
export async function inserts(page: Page): Promise<string[]> {
  return options(page).evaluateAll((rows) => rows.map((r) => r.getAttribute('data-insert') ?? ''));
}

/** Opens the palette from the top bar and waits for its input. */
export async function openPalette(page: Page): Promise<Locator> {
  await page.getByRole('button', { name: 'open command palette (⌘K)' }).click();
  const input = paletteInput(page);
  await expect(input).toBeFocused();
  return input;
}

/** Commits `query` from a fresh palette, then closes the palette. */
export async function runQuery(page: Page, query: string): Promise<void> {
  const input = await openPalette(page);
  await input.fill(query);
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await input.press('Escape');
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeHidden();
}

/** An account or tag in the query catalog the server serves. */
export interface CatalogPath {
  id: string;
  path: string[];
}

/**
 * Serves the real query catalog with `extra` accounts added. Push onto
 * `extra` later to change what the next fetch returns. Call before the page
 * loads so the first fetch is covered.
 */
export async function serveCatalog(page: Page): Promise<{ extra: CatalogPath[] }> {
  const served = { extra: [] as CatalogPath[] };
  await page.route('**/rpc/query_catalog', async (route) => {
    const response = await route.fetch();
    const catalog = (await response.json()) as { accounts: CatalogPath[] };
    await route.fulfill({ response, json: { ...catalog, accounts: [...catalog.accounts, ...served.extra] } });
  });
  return served;
}

/** Makes every query catalog fetch fail, so the palette never gets one. */
export async function failCatalog(page: Page): Promise<void> {
  await page.route('**/rpc/query_catalog', (route) => route.abort());
}

/** The palette's hint line. */
export function hint(page: Page): Locator {
  return page.getByTestId('palette-hint');
}

/** The chip labels, which double as their edit buttons. */
export function chipLabels(page: Page): Locator {
  return page.getByTestId('filter-chips').getByRole('button', { name: /^edit .* filter$/ });
}

/** The transaction register. */
export function register(page: Page): Locator {
  return page.getByLabel('transaction register');
}

/**
 * Opens the Split register (Assets ▸ Split) over all time, with sub-accounts
 * rolled up and the balance column off, so each row shows only its own amount.
 */
export async function openSplitRegister(page: Page): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  const sidebar = page.getByRole('navigation', { name: 'account navigation' }).first();
  const toggle = sidebar.getByRole('button', { name: 'toggle Assets' });
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
    await toggle.click();
  }
  await sidebar.getByText('Split', { exact: true }).click();
  await expect(page.getByLabel('account dashboard').getByText('Split', { exact: true })).toBeVisible();
  const rollup = page.getByLabel('include sub-accounts');
  if (!(await rollup.isChecked())) {
    await rollup.check();
  }
  await register(page).getByLabel('period').selectOption('all_time');
  const balance = register(page).getByRole('button', { name: /^balance:/ });
  while ((await balance.textContent()) !== 'balance: off') {
    await balance.click();
  }
}

/** The Split register's three transactions, each by text only its row shows. */
export const SPLIT_ROWS = {
  booking: 'Example Travel Agency',
  fuel: 'A$80.00',
  topUp: 'A$40.00',
} as const;

/** One of the Split register's transactions. */
export type SplitRow = keyof typeof SPLIT_ROWS;

/** The register row for `row`. */
export function splitRow(page: Page, row: SplitRow): Locator {
  return register(page).locator('[data-tx-id]').filter({ hasText: SPLIT_ROWS[row] });
}

/** Asserts that the register shows exactly the `expected` Split rows. */
export async function expectSplitRows(page: Page, expected: readonly SplitRow[]): Promise<void> {
  for (const row of Object.keys(SPLIT_ROWS) as SplitRow[]) {
    await expect(splitRow(page, row), `${row} row`).toHaveCount(expected.includes(row) ? 1 : 0);
  }
}

/** `YYYY-MM` for the month `n` months before this one. */
export function monthsAgo(n: number): string {
  const d = new Date();
  d.setDate(1);
  d.setMonth(d.getMonth() - n);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}`;
}
