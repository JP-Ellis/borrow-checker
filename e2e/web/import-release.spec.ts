/**
 * A statement row the user deleted, against the real server and csv plugin:
 * the CLI imports a statement and deletes one of its transactions keeping
 * its provenance, the preview reports that row as rejected, and "allow
 * re-import" releases it so a commit brings it back. It writes to the shared
 * database under its own account and profile only.
 */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';

import { DOCS, cli } from './support/env.js';

const PROFILE = 'e2e-release';
const ACCOUNT = 'Assets:Release Test:Spending';
const SOURCE = join(DOCS, 'release-test');
const DELETED = 'Bakery Lane';

test.beforeAll(() => {
  mkdirSync(SOURCE, { recursive: true });
  writeFileSync(
    join(SOURCE, 'statement.csv'),
    ['Date,Amount,Description', `2026-02-03,-6.50,${DELETED}`, '2026-02-10,-3.20,Corner Newsagent', ''].join('\n'),
  );
  const config = join(DOCS, 'e2e-release.toml');
  writeFileSync(
    config,
    [
      `account = "${ACCOUNT}"`,
      'commodity = "AUD"',
      'source_dir = "release-test"',
      'source_glob = "*.csv"',
      'date_column = "Date"',
      'date_format = "%Y-%m-%d"',
      'description_column = "Description"',
      '',
      '[amount_columns]',
      'style = "single"',
      'column = "Amount"',
      '',
    ].join('\n'),
  );
  cli('account', 'create', ACCOUNT, '--opened-on', '2026-01-01');
  cli('profile', 'create', '--name', PROFILE, '--importer', 'csv', '--config', config);
  cli('import', 'run', '--profile', PROFILE);
  const transactions = JSON.parse(cli('transaction', 'list', '--json')) as { id: string; description: string }[];
  const deleted = transactions.find((tx) => tx.description === DELETED);
  if (!deleted) throw new Error(`the import booked no ${DELETED} transaction`);
  cli('transaction', 'delete', deleted.id);
});

test('a deleted row previews as rejected, and allowing re-import brings it back', async ({ page }) => {
  const preview = page.getByTestId('import-preview');
  const chip = page.getByTestId('import-fate-rejected');
  const rejected = preview.locator('tr[data-fate="rejected"]');
  const commit = page.getByTestId('import-commit');

  await test.step('the deleted row is rejected, hidden by default, and writes nothing', async () => {
    await page.goto(`/import?profile=${PROFILE}`);
    await expect(preview).toContainText('Nothing to import · 1 already imported · 1 rejected leg skipped');
    await expect(chip).toHaveText('rejected 1');
    await expect(chip).toHaveAttribute('aria-pressed', 'false');
    await expect(rejected).toHaveCount(0);
    await expect(commit).toBeDisabled();
  });

  await test.step('allowing re-import releases the row and re-previews', async () => {
    await chip.click();
    await expect(rejected).toContainText(DELETED);
    await rejected.getByRole('button', { name: 'allow re-import' }).click();
    await expect(commit).toHaveText('Commit: 1 new');
    await expect(chip).toHaveText('rejected 0');
    await expect(preview).toContainText('1 new · 1 already imported');
  });

  await test.step('the commit books the row again', async () => {
    await commit.click();
    await expect(page.getByTestId('import-outcome')).toContainText('1 new');
    // Other specs may leave rejected rows of their own; none is on this account.
    expect(cli('import', 'rejected', '--json')).not.toContain(ACCOUNT);
  });
});
