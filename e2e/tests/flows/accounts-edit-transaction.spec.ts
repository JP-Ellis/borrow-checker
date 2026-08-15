/**
 * Flow tests for the always-editable transaction detail panel.
 *
 * The seeded database has a "Supermarket" grocery transaction posted to the
 * Groceries account (debit) and Checking (credit), dated in the current
 * month (day 3, 95 AUD, Reconciled). We navigate to the Groceries account and
 * expand this row — it's used instead of the seed's historical "Coles"
 * transactions because the register is now scoped to the account's
 * auto-jumped current period, and "Coles" only appears in past months.
 *
 * Test sequence:
 *   1. Navigate to Accounts → Groceries (sidebar).
 *   2. Expand the "Supermarket" row (click; assert detail appears).
 *   3. Toggle the reconciliation status pill (Reconciled → Flagged).
 *   4. Assert the Save button is visible and enabled.
 *   5. Save (click Save).
 *   6. Verify the new reconciliation status persisted in SQLite.
 */
import Database             from 'better-sqlite3';
import { browser, $, $$ }  from '@wdio/globals';
import {
    DB_PATH,
    dbMetadataKeyType,
    dbTransactionIdByPayee,
    dbTransactionMetadata,
} from '../support/db.js';

// ── DB helpers ──────────────────────────────────────────────────────────────

interface TxRow {
    id:             string;
    reconciliation: string;
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

/**
 * The seed's "Supermarket" transaction, found through its `payee` metadata
 * entry — a payee is an ordinary key, not a column.
 */
function dbFetchSupermarketTx(): TxRow | undefined {
    const id = dbTransactionIdByPayee('Supermarket');
    if (id === undefined) return undefined;
    const reconciliation = dbReconciliation(id);
    if (reconciliation === undefined) return undefined;
    return { id, reconciliation };
}

// ── Navigation helpers ──────────────────────────────────────────────────────

/**
 * Navigate to the accounts page and click on the Groceries account in the
 * sidebar. The "Supermarket" transaction debits the Groceries account so it
 * appears in that register.
 */
async function openGroceriesAccount(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();

    const groceriesSpan = await sidebarNav.$('span=Groceries');
    await groceriesSpan.waitForDisplayed();
    await groceriesSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    // Wait for at least one register row.
    const register = await $('[aria-label="transaction register"]');
    await register.waitForDisplayed();
    await browser.waitUntil(
        () => browser.execute(
            () => (document.querySelector('[aria-label="transaction register"]')
                ?.querySelectorAll('[role="button"]').length ?? 0) > 0,
        ),
        { timeoutMsg: 'No transaction rows appeared in the Groceries register' },
    );
}

/**
 * Find and click the first "Supermarket" transaction row in the register to expand it.
 * Returns once the detail panel with the status pill is visible.
 */
async function expandSupermarketRow(): Promise<void> {
    const register = await $('[aria-label="transaction register"]');

    // Supermarket may appear multiple times — grab the first one.
    const supermarketSpan = await register.$('span=Supermarket');
    await supermarketSpan.waitForDisplayed({ timeoutMsg: '"Supermarket" row did not appear in the Groceries register',
    });

    // Click the parent [role="button"] row via JS to avoid scrolling issues.
    await browser.execute((el: Element) => {
        const row = el.closest('[role="button"]');
        if (row instanceof HTMLElement) row.click();
    }, await supermarketSpan.getElement() as unknown as Element);

    // Assert the detail (status pill) is now visible.
    const pill = await $('[data-testid="status-pill"]');
    await pill.waitForDisplayed({ timeoutMsg: 'Status pill did not appear after expanding the Supermarket row',
    });
}

// ── Tests ───────────────────────────────────────────────────────────────────

describe('Accounts — edit transaction detail', () => {
    it('can toggle the reconciliation status and save the change', async function () {
        const seedTx = dbFetchSupermarketTx();
        if (!seedTx) {
            console.warn('No Supermarket tx found in DB — skipping edit test');
            this.skip();
        }

        await openGroceriesAccount();
        await expandSupermarketRow();

        // ── Toggle status. ──────────────────────────────────────────────
        const pill = await $('[data-testid="status-pill"]');
        const labelBefore = await pill.getText();

        await pill.click();

        await browser.waitUntil(
            async () => (await pill.getText()) !== labelBefore,
            { timeoutMsg: 'Status pill label did not change after click' },
        );
        const labelAfter = await pill.getText();
        expect(labelAfter).not.toBe(labelBefore);

        // ── Save bar must appear. ────────────────────────────────────────
        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({ timeoutMsg: 'Save button did not appear after toggling reconciliation',
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
            { timeoutMsg: 'Save bar did not disappear after clicking Save' },
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
        await expandSupermarketRow();

        // At least one posting-amount input must be present in the detail.
        const amountInputs = await $$('[data-testid="posting-amount"]');
        expect(amountInputs.length).toBeGreaterThan(0);

        // At least one account picker input must also be present.
        const accountInputs = await $$('[data-testid="account-input"]');
        expect(accountInputs.length).toBeGreaterThan(0);
    });

    it('does not show the save bar when no changes have been made', async () => {
        await openGroceriesAccount();
        await expandSupermarketRow();

        // The save bar must NOT be present immediately on open (no edits yet).
        const saveBtn = await $('[aria-label="save transaction"]');
        await browser.pause(300);
        const visible = await saveBtn.isDisplayed().catch(() => false);
        expect(visible).toBe(false);
    });

    it('can add a metadata entry and persist it on save', async function () {
        const seedTx = dbFetchSupermarketTx();
        if (!seedTx) {
            console.warn('No Supermarket tx found in DB — skipping metadata add test');
            this.skip();
        }

        await openGroceriesAccount();
        await expandSupermarketRow();

        // Add an empty metadata row to the editor.
        const addMetaBtn = await $('[data-testid="meta-add"]');
        await addMetaBtn.waitForDisplayed({
            timeoutMsg: 'metadata "+" button did not appear in the transaction detail',
        });
        await addMetaBtn.click();

        // The new row is the last one, and both of its inputs start empty.
        const keyInputs = await $$('[data-testid="meta-key"]');
        const valueInputs = await $$('[data-testid="meta-value"]');
        const keyCount = await keyInputs.length;
        const valueCount = await valueInputs.length;
        expect(keyCount).toBeGreaterThan(0);
        expect(valueCount).toBe(keyCount);

        await keyInputs[keyCount - 1].setValue('invoice');
        await valueInputs[valueCount - 1].setValue('1502');

        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({
            timeoutMsg: 'Save button did not appear after adding a metadata entry',
        });
        await saveBtn.click();
        await browser.waitUntil(
            async () => !(await saveBtn.isDisplayed().catch(() => false)),
            { timeoutMsg: 'Save bar did not disappear after saving the metadata entry' },
        );

        await browser.pause(300);

        const entries = dbTransactionMetadata(seedTx.id);
        const added = entries.find(row => row.key === 'invoice');
        expect(added).toBeDefined();
        expect(added!.value_text).toBe('1502');
        // Nothing about `1502` fails to read as a number, so it stores unflagged.
        expect(added!.mismatched).toBe(0);
        // A key enters the registry on first write, typed by its first value.
        expect(dbMetadataKeyType('invoice')).toBe('number');
    });

    it('can edit an existing metadata entry and persist it on save', async function () {
        const seedTx = dbFetchSupermarketTx();
        if (!seedTx) {
            console.warn('No Supermarket tx found in DB — skipping metadata edit test');
            this.skip();
        }

        await openGroceriesAccount();
        await expandSupermarketRow();

        // The seeded transaction carries one `payee` entry; find its row by the
        // key input holding that key, and rewrite the value beside it.
        const keyInputs = await $$('[data-testid="meta-key"]');
        const valueInputs = await $$('[data-testid="meta-value"]');
        const keyCount = await keyInputs.length;
        let payeeIndex = -1;
        for (let i = 0; i < keyCount; i++) {
            if ((await keyInputs[i].getValue()) === 'payee') {
                payeeIndex = i;
                break;
            }
        }
        expect(payeeIndex).toBeGreaterThan(-1);

        await valueInputs[payeeIndex].setValue('Corner Store');

        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({
            timeoutMsg: 'Save button did not appear after editing a metadata entry',
        });
        await saveBtn.click();
        await browser.waitUntil(
            async () => !(await saveBtn.isDisplayed().catch(() => false)),
            { timeoutMsg: 'Save bar did not disappear after saving the metadata edit' },
        );

        await browser.pause(300);

        const entries = dbTransactionMetadata(seedTx.id);
        const payees = entries.filter(row => row.key === 'payee');
        expect(payees.length).toBe(1);
        expect(payees[0]!.value_text).toBe('Corner Store');
    });
});
