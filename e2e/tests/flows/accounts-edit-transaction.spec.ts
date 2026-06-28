/**
 * Flow tests for the always-editable transaction detail panel.
 *
 * The seeded database has Coles grocery transactions posted to the Groceries
 * account (debit) and CreditCard (credit). We navigate to the Groceries
 * account and expand the "Coles" row that is seeded as Unreconciled (1 month
 * ago, April fortnightly groceries, 110 AUD).
 *
 * Test sequence:
 *   1. Navigate to Accounts → Groceries (sidebar).
 *   2. Expand the "Coles" row (click; assert detail appears).
 *   3. Toggle the reconciliation status pill (Unreconciled → Flagged).
 *   4. Assert the Save button is visible and enabled.
 *   5. Save (click Save).
 *   6. Verify the new reconciliation status persisted in SQLite.
 */
import { dirname }          from 'node:path';
import { fileURLToPath }    from 'node:url';
import { resolve }          from 'node:path';
import Database             from 'better-sqlite3';
import { browser, $, $$ }  from '@wdio/globals';

const __dirname  = dirname(fileURLToPath(import.meta.url));
const DB_PATH    = resolve(__dirname, '../../fixtures/test.db');

// ── DB helpers ──────────────────────────────────────────────────────────────

interface TxRow {
    id:             string;
    payee:          string | null;
    reconciliation: string;
}

function dbFetchColesTx(): TxRow | undefined {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        return db
            .prepare(
                `SELECT id, payee, reconciliation
                   FROM transactions
                  WHERE payee = 'Coles'
                  ORDER BY date DESC
                  LIMIT 1`,
            )
            .get() as TxRow | undefined;
    } finally {
        db.close();
    }
}

function dbReconciliation(txId: string): string | undefined {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare('SELECT reconciliation FROM transactions WHERE id = ?')
            .get(txId) as { reconciliation: string } | undefined;
        return row?.reconciliation;
    } finally {
        db.close();
    }
}

// ── Navigation helpers ──────────────────────────────────────────────────────

/**
 * Navigate to the accounts page and click on the Groceries account in the
 * sidebar. Coles transactions debit the Groceries account so they appear in
 * that register.
 */
async function openGroceriesAccount(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed({ timeout: 5_000 });
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeout: 5_000, timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed({ timeout: 10_000 });

    const groceriesSpan = await sidebarNav.$('span=Groceries');
    await groceriesSpan.waitForDisplayed({ timeout: 10_000 });
    await groceriesSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeout: 5_000, timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    // Wait for at least one register row.
    const register = await $('[aria-label="transaction register"]');
    await register.waitForDisplayed({ timeout: 10_000 });
    await browser.waitUntil(
        () => browser.execute(
            () => (document.querySelector('[aria-label="transaction register"]')
                ?.querySelectorAll('[role="button"]').length ?? 0) > 0,
        ),
        { timeout: 15_000, timeoutMsg: 'No transaction rows appeared in the Groceries register' },
    );
}

/**
 * Find and click the first "Coles" transaction row in the register to expand it.
 * Returns once the detail panel with the status pill is visible.
 */
async function expandColesRow(): Promise<void> {
    const register = await $('[aria-label="transaction register"]');

    // Coles may appear multiple times — grab the first one.
    const colesSpan = await register.$('span=Coles');
    await colesSpan.waitForDisplayed({
        timeout: 10_000,
        timeoutMsg: '"Coles" row did not appear in the Groceries register',
    });

    // Click the parent [role="button"] row via JS to avoid scrolling issues.
    await browser.execute((el: Element) => {
        const row = el.closest('[role="button"]');
        if (row instanceof HTMLElement) row.click();
    }, await colesSpan.getElement() as unknown as Element);

    // Assert the detail (status pill) is now visible.
    const pill = await $('[data-testid="status-pill"]');
    await pill.waitForDisplayed({
        timeout: 5_000,
        timeoutMsg: 'Status pill did not appear after expanding the Coles row',
    });
}

// ── Tests ───────────────────────────────────────────────────────────────────

describe('Accounts — edit transaction detail', () => {
    it('can toggle the reconciliation status and save the change', async () => {
        const seedTx = dbFetchColesTx();
        if (!seedTx) {
            console.warn('No Coles tx found in DB — skipping edit test');
            return;
        }

        await openGroceriesAccount();
        await expandColesRow();

        // ── Toggle status. ──────────────────────────────────────────────
        const pill = await $('[data-testid="status-pill"]');
        const labelBefore = await pill.getText();

        await pill.click();

        await browser.waitUntil(
            async () => (await pill.getText()) !== labelBefore,
            { timeout: 3_000, timeoutMsg: 'Status pill label did not change after click' },
        );
        const labelAfter = await pill.getText();
        expect(labelAfter).not.toBe(labelBefore);

        // ── Save bar must appear. ────────────────────────────────────────
        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({
            timeout: 3_000,
            timeoutMsg: 'Save button did not appear after toggling reconciliation',
        });

        const isDisabled: boolean = await browser.execute(
            (btn: Element) => (btn as HTMLButtonElement).disabled,
            await saveBtn.getElement() as unknown as Element,
        );
        expect(isDisabled).toBe(false);

        // ── Save. ────────────────────────────────────────────────────────
        await saveBtn.click();

        // Save bar disappears once the IPC write completes.
        await browser.waitUntil(
            async () => !(await saveBtn.isDisplayed().catch(() => false)),
            { timeout: 10_000, timeoutMsg: 'Save bar did not disappear after clicking Save' },
        );

        // ── Verify in SQLite. ────────────────────────────────────────────
        await browser.pause(300);

        const reconAfter = dbReconciliation(seedTx.id);
        expect(reconAfter).toBeDefined();
        // The reconciliation must have changed from its seeded value.
        expect(reconAfter).not.toBe(seedTx.reconciliation);
    });

    it('shows posting-amount and account-input elements inside the expanded detail', async () => {
        await openGroceriesAccount();
        await expandColesRow();

        // At least one posting-amount input must be present in the detail.
        const amountInputs = await $$('[data-testid="posting-amount"]');
        expect(amountInputs.length).toBeGreaterThan(0);

        // At least one account picker input must also be present.
        const accountInputs = await $$('[data-testid="account-input"]');
        expect(accountInputs.length).toBeGreaterThan(0);
    });

    it('does not show the save bar when no changes have been made', async () => {
        await openGroceriesAccount();
        await expandColesRow();

        // The save bar must NOT be present immediately on open (no edits yet).
        const saveBtn = await $('[aria-label="save transaction"]');
        await browser.pause(300);
        const visible = await saveBtn.isDisplayed().catch(() => false);
        expect(visible).toBe(false);
    });
});
