import Database from 'better-sqlite3';
import { browser, $, $$, expect as wdioExpect } from '@wdio/globals';
import { DB_PATH as TEST_DB_PATH, dbTransactionIdByPayee } from '../support/db.js';

// ── Helpers ────────────────────────────────────────────────────────────────

/**
 * Navigate to the accounts page and click on the Checking account in the
 * sidebar.  Returns once the URL reflects the selected account and the
 * dashboard "+ transaction" button is visible.
 */
async function openCheckingAccount(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    // Accounts are loaded via async IPC — wait for any link to appear.
    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();

    const checkingSpan = await sidebarNav.$('span=Checking');
    await checkingSpan.waitForDisplayed();
    await checkingSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    // Wait for the dashboard to render (triggered once account IPC data arrives).
    await browser.waitUntil(
        async () => {
            for (const btn of await $$('button')) {
                if ((await btn.getText()).includes('+ transaction')) return true;
            }
            return false;
        },
        { timeoutMsg: '"+ transaction" button did not appear within 10 s' },
    );
}

/** Click the first "+ transaction" button on the dashboard action bar. */
async function clickAddTransactionButton(): Promise<void> {
    for (const btn of await $$('button')) {
        if ((await btn.getText()).includes('+ transaction')) {
            await btn.click();
            return;
        }
    }
    throw new Error('"+ transaction" button not found');
}

/** Click the first "+ posting" button on the open form. */
async function clickAddPostingButton(): Promise<void> {
    for (const btn of await $$('button')) {
        if ((await btn.getText()).includes('+ posting')) {
            await btn.click();
            return;
        }
    }
    throw new Error('"+ posting" button not found');
}

/** Wait until the add-transaction form is visible and return it. */
async function waitForForm(): Promise<WebdriverIO.Element> {
    const form = await $('[data-testid="add-transaction-form"]');
    await form.waitForDisplayed({ timeoutMsg: 'AddTransactionForm did not appear within 5 s',
    });
    return form;
}

// ── DB helpers ─────────────────────────────────────────────────────────────

interface TxRow {
    id: string;
    date: string;
    reconciliation: string;
}

interface PostingRow {
    account_name: string;
    amount: string;
    commodity: string;
}

/**
 * Finds a transaction by its `payee` metadata entry. A payee is an ordinary
 * metadata entry under an ordinary key, so there is no payee column to select.
 */
function dbQueryTransaction(payee: string): TxRow | undefined {
    const id = dbTransactionIdByPayee(payee);
    if (id === undefined) return undefined;
    const db = new Database(TEST_DB_PATH, { readonly: true });
    try {
        return db
            .prepare('SELECT id, date, reconciliation FROM transactions WHERE id = ?')
            .get(id) as TxRow | undefined;
    } finally {
        db.close();
    }
}

function dbQueryPostings(transactionId: string): PostingRow[] {
    const db = new Database(TEST_DB_PATH, { readonly: true });
    try {
        return db
            .prepare(`
                SELECT a.name AS account_name, p.amount, p.commodity
                FROM postings p
                JOIN accounts a ON a.id = p.account_id
                WHERE p.transaction_id = ?
                ORDER BY p.position
            `)
            .all(transactionId) as PostingRow[];
    } finally {
        db.close();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Accounts — add transaction', () => {
    /**
     * Basic flow: open form via button, fill a two-posting transaction, assert
     * the payee appears in the register and the transaction + postings are
     * persisted in SQLite with the correct amounts and zero-sum balance.
     */
    it('creates a two-posting transaction and reflects it in the register and database', async () => {
        await openCheckingAccount();
        await clickAddTransactionButton();
        await waitForForm();

        // Leave the date field at its default (today, per js_sys::Date) so the
        // transaction lands in the account's auto-jumped current-period window.
        const expectedDate: string = await browser.execute(
            () => new Date().toISOString().slice(0, 10),
        );
        // The payee is an ordinary metadata row on the form, not an input of
        // its own.
        await $('[data-testid="atf-meta-key-0"]').setValue('payee');
        await $('[data-testid="atf-meta-value-0"]').setValue('E2E Test Payee');

        // Primary posting amount (Checking account debited).
        await $('#atf-primary-amount').setValue('-42.00');

        // First offset posting is pre-populated; fill its amount.
        await $('[data-testid="atf-offset-amount-0"]').setValue('42.00');

        await $('[data-testid="add-transaction-submit"]').click();

        // ── UI assertion ──────────────────────────────────────────────────
        // Wait for the payee to appear as a cell in the transaction register.
        const payeeCell = await $('span=E2E Test Payee');
        await payeeCell.waitForDisplayed({ timeoutMsg: '"E2E Test Payee" did not appear in the register within 10 s',
        });

        // ── DB assertions ─────────────────────────────────────────────────
        const tx = dbQueryTransaction('E2E Test Payee');
        wdioExpect(tx).toBeDefined();
        wdioExpect(tx!.date).toBe(expectedDate);
        wdioExpect(tx!.reconciliation).toBe('flagged');

        const postings = dbQueryPostings(tx!.id);
        // A balanced double-entry transaction has exactly two postings.
        wdioExpect(postings.length).toBe(2);

        // Both postings use the Checking account's commodity (AUD).
        const allAud = postings.every(p => p.commodity === 'AUD');
        wdioExpect(allAud).toBe(true);

        // The amounts must sum to zero (double-entry invariant).
        // Amounts are stored as decimal strings; round to avoid float noise.
        const rawSum = postings.reduce((acc, p) => acc + parseFloat(p.amount), 0);
        const centSum = Math.round(rawSum * 100);
        wdioExpect(centSum).toBe(0);

        // The primary (Checking) posting is -42.00.
        const checkingPosting = postings.find(p => p.account_name === 'Checking');
        wdioExpect(checkingPosting).toBeDefined();
        const checkingCents = Math.round(parseFloat(checkingPosting!.amount) * 100);
        wdioExpect(checkingCents).toBe(-4200);
    });

    /**
     * Split-transaction flow: open form via ↵ keyboard shortcut, add a second
     * offset posting, and verify the three-posting zero-sum entry in SQLite.
     */
    it('creates a split transaction via keyboard shortcut and persists all postings', async () => {
        await openCheckingAccount();

        // Press ↵ while no input has focus to trigger the keyboard shortcut.
        // Click a non-interactive area first so no button or input is focused.
        await browser.execute(() => {
            (document.activeElement as HTMLElement | null)?.blur();
        });
        await browser.keys(['Enter']);

        await waitForForm();

        // Leave the date field at its default (today) so the split transaction
        // lands in the account's auto-jumped current-period window.
        const expectedDate: string = await browser.execute(
            () => new Date().toISOString().slice(0, 10),
        );
        await $('[data-testid="atf-meta-key-0"]').setValue('payee');
        await $('[data-testid="atf-meta-value-0"]').setValue('E2E Split Payee');

        // Primary posting: Checking debited -50.00.
        await $('#atf-primary-amount').setValue('-50.00');

        // First offset posting: 30.00 (e.g. Groceries, the default).
        await $('[data-testid="atf-offset-amount-0"]').setValue('30.00');

        // Add a second offset posting via the "+ posting" button.
        await clickAddPostingButton();

        // Wait for the second posting row to appear.
        const secondOffsetAmount = await $('[data-testid="atf-offset-amount-1"]');
        await secondOffsetAmount.waitForDisplayed();

        // Change the second offset account to a different account if possible.
        const secondOffsetAccount = await $('[data-testid="atf-offset-account-1"]');
        try {
            await secondOffsetAccount.selectByVisibleText('Dining');
        } catch {
            // Dining may not be available; keep the default selection.
        }

        // Second offset posting: 20.00 → total -50 + 30 + 20 = 0.
        await secondOffsetAmount.setValue('20.00');

        await $('[data-testid="add-transaction-submit"]').click();

        // ── UI assertion ──────────────────────────────────────────────────
        const payeeCell = await $('span=E2E Split Payee');
        await payeeCell.waitForDisplayed({ timeoutMsg: '"E2E Split Payee" did not appear in the register within 10 s',
        });

        // ── DB assertions ─────────────────────────────────────────────────
        const tx = dbQueryTransaction('E2E Split Payee');
        wdioExpect(tx).toBeDefined();
        wdioExpect(tx!.date).toBe(expectedDate);

        const postings = dbQueryPostings(tx!.id);
        // A split transaction has three postings: one primary + two offsets.
        wdioExpect(postings.length).toBe(3);

        // All amounts must sum to zero.
        const rawSum = postings.reduce((acc, p) => acc + parseFloat(p.amount), 0);
        const centSum = Math.round(rawSum * 100);
        wdioExpect(centSum).toBe(0);

        // The Checking posting carries -50.00.
        const checkingPosting = postings.find(p => p.account_name === 'Checking');
        wdioExpect(checkingPosting).toBeDefined();
        const checkingCents = Math.round(parseFloat(checkingPosting!.amount) * 100);
        wdioExpect(checkingCents).toBe(-5000);
    });

    /**
     * Keyboard-shortcut gate: the ↵ shortcut must NOT open the form when an
     * input element has keyboard focus (e.g. the user is typing in a search box
     * or another form).  Verify by focusing a button, pressing Enter, and
     * confirming the add-transaction form does not appear.
     */
    it('does not open the form when Enter is pressed with a button focused', async () => {
        await openCheckingAccount();

        // Focus the first button (e.g. "reconcile" or "import") via Tab.
        // WebdriverIO's Tab key moves focus into the first interactive element.
        const firstBtn = (await $$('button'))[0];
        await firstBtn.click(); // click gives it focus without triggering the shortcut

        // Press Enter — with a button focused the handler should suppress the shortcut.
        await browser.keys(['Enter']);

        // The form must NOT appear.
        const form = await $('[data-testid="add-transaction-form"]');
        await browser.pause(600); // brief settle time
        const visible = await form.isDisplayed().catch(() => false);
        wdioExpect(visible).toBe(false);
    });
});
