/**
 * A reversal's expanded detail is read-only. The transaction-row QA page
 * renders one beside editable rows, so the checks scope to the detail that
 * holds the reversal note.
 */
import { expect, test } from '@playwright/test';

test('a reversal detail links its original and offers no editing', async ({ page }) => {
  await page.goto('/__test/component/transaction-row');

  const note = page.getByTestId('reversal-of');
  await expect(note).toBeVisible();
  const detail = note.locator('..');

  await expect(detail.getByTestId('reversal-postings')).toBeVisible();
  await expect(detail.getByRole('button', { name: 'Reverse', exact: true })).toHaveCount(0);
  await expect(detail.getByPlaceholder('YYYY-MM-DD')).toHaveAttribute('readonly', '');
});
