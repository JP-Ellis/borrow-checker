import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import Database from 'better-sqlite3';
import { browser, $, expect as wdioExpect } from '@wdio/globals';

const __dirname = dirname(fileURLToPath(import.meta.url));
const TEST_DB_PATH = resolve(__dirname, '../../fixtures/test.db');

describe('Accounts — add transaction', () => {
    it('creates a transaction and reflects it in the register and database', async () => {
        // Navigate to the accounts page via the top-bar nav.
        const navAccounts = await $('[data-testid="nav-accounts"]');
        await navAccounts.waitForDisplayed({ timeout: 5_000 });
        await navAccounts.click();

        await browser.waitUntil(
            async () => (await browser.getUrl()).includes('/accounts'),
            { timeout: 5_000, timeoutMsg: 'URL did not reach /accounts within 5 s' },
        );

        // Click on the "Checking" account in the sidebar.
        // "Checking" comes from the seed DB — seed-test-db creates this account.
        const checkingLink = await $('=Checking');
        await checkingLink.waitForDisplayed({ timeout: 5_000 });
        await checkingLink.click();

        // Wait for the URL to update to the selected account.
        await browser.waitUntil(
            async () => (await browser.getUrl()).includes('/accounts/'),
            { timeout: 5_000, timeoutMsg: 'URL did not update to account route within 5 s' },
        );

        // Wait for the test button to appear (only renders once an account is selected).
        const addBtn = await $('[data-testid="add-test-transaction"]');
        await addBtn.waitForDisplayed({ timeout: 5_000 });

        // Fire the placeholder create button.
        await addBtn.click();

        // UI assertion — "Test Payee" row must appear in the transaction register.
        const payeeCell = await $('=Test Payee');
        await payeeCell.waitForDisplayed({
            timeout: 5_000,
            timeoutMsg: '"Test Payee" did not appear in the register within 5 s',
        });

        // DB assertion — row must be persisted in SQLite.
        let row: unknown;
        {
            const db = new Database(TEST_DB_PATH, { readonly: true });
            try {
                row = db
                    .prepare('SELECT id, payee FROM transactions WHERE payee = ?')
                    .get('Test Payee');
            } finally {
                db.close();
            }
        }

        wdioExpect(row).toBeDefined();
    });
});
