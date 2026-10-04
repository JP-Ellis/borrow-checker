/**
 * Deleting a backup from Settings → Backup: a two-step gate, then the list
 * refreshes. `page.route` stands in for the server's pool.
 */
import { expect, test } from '@playwright/test';

const BACKUP = {
  file_name: '20000101-000000000.manual.sqlite',
  path: '/backups/ledger_x/20000101-000000000.manual.sqlite',
  kind: 'manual',
  created_at: '2000-01-01T00:00:00',
  size_bytes: 1024,
};

test('a delete needs a confirm, then refreshes the list', async ({ page }) => {
  let rows = [BACKUP];
  await page.route('**/rpc/list_backups', (route) => route.fulfill({ json: rows }));
  let deleted: unknown = null;
  await page.route('**/rpc/delete_backup', async (route) => {
    deleted = route.request().postDataJSON();
    rows = [];
    await route.fulfill({ json: null });
  });
  await page.goto('/settings');
  await page.getByTestId('settings-nav-backup').click();

  await page.getByTestId('backup-delete').click();
  await page.getByTestId('backup-delete-cancel').click();
  expect(deleted).toBeNull();

  await page.getByTestId('backup-delete').click();
  await page.getByTestId('backup-delete-confirm').click();

  await expect(page.getByTestId('backup-list').locator('li')).toHaveCount(0);
  expect(deleted).toEqual({ file_name: BACKUP.file_name });
});

test('the retain-count hint says manual backups are kept', async ({ page }) => {
  await page.goto('/settings');
  await page.getByTestId('settings-nav-backup').click();

  await expect(page.getByTestId('backup-retain-hint')).toHaveText(
    'Per automatic kind. Manual backups are kept until you delete them.',
  );
});
