/**
 * The import page against the real server and csv plugin, in one run: a
 * blocker the CLI clears, a write-time warning, a source that changes before commit,
 * the batch in History, a re-preview of already-imported rows, and a discard
 * whose snapshot link lands on the highlighted backup. It writes to the
 * shared database under its own account and profile only.
 */
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test } from '@playwright/test';

import { DOCS, cli } from './support/env.js';

const PROFILE = 'e2e-csv';
const ACCOUNT = 'Assets:Import Test:Everyday';
const SOURCE = join(DOCS, 'import-test');
const STATEMENT = join(SOURCE, 'statement.csv');

test.beforeAll(() => {
  mkdirSync(SOURCE, { recursive: true });
  writeFileSync(
    STATEMENT,
    [
      'Date,Amount,Description',
      '2026-01-05,-42.50,Woolworths Metro',
      '2026-01-12,-18.00,Opal top-up',
      '2026-01-20,1500.00,Salary ACME Pty Ltd',
      '',
    ].join('\n'),
  );
  const config = join(DOCS, 'e2e-csv.toml');
  writeFileSync(
    config,
    [
      `account = "${ACCOUNT}"`,
      'commodity = "AUD"',
      'source_dir = "import-test"',
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
  cli('profile', 'create', '--name', PROFILE, '--importer', 'csv', '--config', config);
});

test('preview, fix, commit, re-preview and discard one profile', async ({ page }) => {
  test.setTimeout(120_000);
  const preview = page.getByTestId('import-preview');
  const rows = preview.getByRole('table', { name: 'parsed rows' }).locator('tbody tr[data-fate]');
  const commit = page.getByTestId('import-commit');

  await test.step('?profile= previews and names the missing account', async () => {
    await page.goto(`/import?profile=${PROFILE}`);
    await expect(preview).toContainText('Nothing to import · 3 legs skipped');
    await expect(page.getByTestId('import-blockers')).toContainText(
      `borrow-checker account create '${ACCOUNT}'`,
    );
    await expect(commit).toBeDisabled();
    await expect(commit).toHaveText('Nothing to import');
    await expect(rows).toHaveCount(3);
    await expect(rows.first()).toContainText('skipped: unresolved account');
  });

  await test.step('creating the account and re-previewing clears the blocker', async () => {
    cli('account', 'create', ACCOUNT, '--opened-on', '2026-01-10');
    await page.getByTestId('import-repreview').click();
    await expect(page.getByTestId('import-blockers')).toHaveCount(0);
    // A preview's warnings are a lower bound: the date check runs at write time.
    await expect(page.getByTestId('import-warnings')).toHaveCount(0);
    await expect(commit).toHaveText('Commit: 3 new');
  });

  await test.step('a source changed since the preview swaps in a fresh one', async () => {
    appendFileSync(STATEMENT, '2026-01-25,-7.90,Coles Express\n');
    await commit.click();
    await expect(preview).toContainText('The source changed since you previewed. Review again.');
    await expect(commit).toHaveText('Commit: 4 new');
  });

  let batchTestId = '';
  await test.step('commit reports the outcome and the batch lands in History', async () => {
    await commit.click();
    const outcome = page.getByTestId('import-outcome');
    await expect(outcome).toContainText('4 new');
    await expect(outcome).toContainText('pre-import');
    await expect(outcome).toContainText('2026-01-05 but the account opened on 2026-01-10');
    await page.getByTestId('import-view-history').click();
    await expect(page).toHaveURL(/\/import\?batch=/);
    const highlighted = page.locator('[data-testid^="import-batch-row-"][data-highlighted="true"]');
    await expect(highlighted).toHaveCount(1);
    await expect(highlighted).toContainText('complete');
    batchTestId = (await highlighted.getAttribute('data-testid')) ?? '';
  });

  await test.step('a re-preview hides the already-imported rows until asked', async () => {
    await page.getByTestId(`import-profile-row-${PROFILE}`).getByRole('button', { name: 'Preview' }).click();
    await expect(preview).toContainText('Nothing to import · 4 already imported');
    const chip = page.getByTestId('import-fate-already_imported');
    await expect(chip).toHaveText('already imported 4');
    await expect(chip).toHaveAttribute('aria-pressed', 'false');
    await expect(rows).toHaveCount(0);
    await chip.click();
    await expect(rows).toHaveCount(4);
    await expect(rows.first()).toContainText('already imported');
  });

  await test.step('discard states its consequences, then removes the batch', async () => {
    const batch = page.getByTestId(batchTestId);
    await batch.getByTestId('discard-arm').click();
    const consequences = batch.getByTestId('discard-consequences');
    await expect(consequences).toContainText('Removes 4 postings across 4 transactions.');
    await expect(consequences).toContainText('A snapshot is taken first.');
    await expect(batch.getByTestId('discard-confirm')).toHaveText('Discard: remove 4 transactions');
    await batch.getByTestId('discard-confirm').click();
    await expect(batch).toContainText('discarded');
    await expect(batch).toContainText('Removed 4 postings across 4 transactions.');
    await expect(batch.getByTestId('restore-snapshot-link')).toBeVisible();
  });

  await test.step('the snapshot link opens Settings → Backup on that backup', async () => {
    await page.getByTestId(batchTestId).getByTestId('restore-snapshot-link').click();
    await expect(page).toHaveURL(/\/settings\?section=backup&backup=/);
    await expect(page.getByTestId('backup-list').locator('li[data-highlighted="true"]')).toContainText(
      'pre-discard',
    );
  });
});
