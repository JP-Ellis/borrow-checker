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

        // Wait for the sidebar to populate with account links before clicking.
        // Accounts are loaded via async IPC so the nav may lag behind the URL change.
        // We wait for any <a> inside the nav to appear, which means the list is ready.
        const sidebarNav = await $('nav[aria-label="account navigation"]');
        await sidebarNav.$('a').waitForDisplayed({ timeout: 10_000 });

        // Click on the "Checking" account in the sidebar.
        // "Checking" comes from the seed DB.  Each sidebar row is an <a> containing
        // a name <span> and a balance <span>, so $('=Checking') (exact link text)
        // would not match the full "Checking$3,500" text.  Chain a text selector on
        // the nav element to scope the search without mixing CSS + text in one string
        // (WebdriverIO does not support 'css-selector span=Text' combined selectors).
        const checkingSpan = await sidebarNav.$('span=Checking');
        await checkingSpan.waitForDisplayed({ timeout: 10_000 });
        await checkingSpan.click();

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
        // Use span= (element-text selector) because the payee renders in a <span>,
        // not an <a>.  The plain ='Text' selector uses WebDriver's link-text
        // strategy and only matches <a> elements.
        const payeeCell = await $('span=Test Payee');
        await payeeCell.waitForDisplayed({
            timeout: 10_000,
            timeoutMsg: '"Test Payee" did not appear in the register within 10 s',
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
