import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import Database from 'better-sqlite3';
import { browser, $, $$, expect as wdioExpect } from '@wdio/globals';

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

        // Click the "+ transaction" button on the dashboard action bar.
        // Find the button by its text content since it also contains a <kbd> child.
        const allButtons = await $$('button');
        let addTxBtn: WebdriverIO.Element | undefined;
        for (const btn of allButtons) {
            const text = await btn.getText();
            if (text.includes('+ transaction')) {
                addTxBtn = btn;
                break;
            }
        }
        if (!addTxBtn) {
            throw new Error('"+ transaction" button not found on dashboard');
        }
        await addTxBtn.waitForDisplayed({ timeout: 5_000 });
        await addTxBtn.click();

        // Wait for the AddTransactionForm to appear.
        const form = await $('[data-testid="add-transaction-form"]');
        await form.waitForDisplayed({
            timeout: 5_000,
            timeoutMsg: 'AddTransactionForm did not appear within 5 s',
        });

        // Fill in the date field (type="date", id="atf-date").
        const dateField = await $('#atf-date');
        await dateField.setValue('2026-06-01');

        // Fill in the payee field.
        const payeeField = await $('#atf-payee');
        await payeeField.setValue('E2E Test Payee');

        // Fill in the amount field.
        const amountField = await $('#atf-amount');
        await amountField.setValue('-42.00');

        // Submit the form via the submit button.
        const submitBtn = await $('[data-testid="add-transaction-submit"]');
        await submitBtn.click();

        // UI assertion — "E2E Test Payee" row must appear in the transaction register.
        // Use span= (element-text selector) because the payee renders in a <span>,
        // not an <a>.  The plain ='Text' selector uses WebDriver's link-text
        // strategy and only matches <a> elements.
        const payeeCell = await $('span=E2E Test Payee');
        await payeeCell.waitForDisplayed({
            timeout: 10_000,
            timeoutMsg: '"E2E Test Payee" did not appear in the register within 10 s',
        });

        // DB assertion — row must be persisted in SQLite.
        let row: unknown;
        {
            const db = new Database(TEST_DB_PATH, { readonly: true });
            try {
                row = db
                    .prepare('SELECT id, payee FROM transactions WHERE payee = ?')
                    .get('E2E Test Payee');
            } finally {
                db.close();
            }
        }

        wdioExpect(row).toBeDefined();
    });
});
