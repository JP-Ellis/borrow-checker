/**
 * The reload after a restore. A real restore exits the server this suite
 * shares, so `page.route` stands in for it: the restore succeeds, then the
 * server answers nothing until it "comes back".
 */
import { expect, test } from '@playwright/test';

const BACKUP = {
  file_name: '20000101-000000000.manual.sqlite',
  path: '/backups/manual-2000-01-01.db',
  kind: 'manual',
  created_at: '2000-01-01T00:00:00',
  size_bytes: 1024,
};

test('a restore reloads the page once the server answers again', async ({ page }) => {
  await page.route('**/rpc/list_backups', (route) => route.fulfill({ json: [BACKUP] }));
  await page.goto('/settings');
  await page.getByTestId('settings-nav-backup').click();

  await page.route('**/rpc/restore_database', (route) => route.fulfill({ json: null }));
  // A restarting server behind a proxy answers 502 until it is back.
  let polls = 0;
  await page.route('**/rpc/get_settings', (route) => {
    polls += 1;
    return polls <= 2 ? route.fulfill({ status: 502, body: 'Bad Gateway' }) : route.continue();
  });

  await page.getByTestId('backup-restore').click();
  const reloaded = page.waitForEvent('load');
  await page.getByTestId('backup-restore-confirm').click();
  await expect(page.getByText('Restoring… the page will reload')).toBeVisible();

  await reloaded;
  expect(polls).toBeGreaterThanOrEqual(3);
});
