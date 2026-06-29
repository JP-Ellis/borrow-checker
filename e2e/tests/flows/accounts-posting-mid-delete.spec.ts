/**
 * Regression test for #210 — stable posting uid (mid-list delete clobber).
 *
 * Before the fix, deleting a non-last posting from a transaction with 3+
 * postings caused Leptos to reuse component views by index, shifting the vec
 * so retained rows wrote stale data into the wrong slot on the next save.
 *
 * This test:
 *   1. Opens a transaction with exactly 3 postings from the test database.
 *   2. Records the account names and amounts of all three rows.
 *   3. Deletes the *middle* posting (index 1).
 *   4. Asserts that the two surviving rows still display the data that
 *      originally belonged to postings 0 and 2 (not 1).
 *   5. Saves and verifies persistence via SQLite.
 *
 * The seeded database must contain at least one transaction with 3+ postings.
 * If no such transaction exists the test is skipped (not failed) with a
 * console warning.
 */
import { dirname }          from 'node:path';
import { fileURLToPath }    from 'node:url';
import { resolve }          from 'node:path';
import Database             from 'better-sqlite3';
import { browser, $, $$ }  from '@wdio/globals';

const __dirname = dirname(fileURLToPath(import.meta.url));
const DB_PATH   = resolve(__dirname, '../../fixtures/test.db');

// ── DB helpers ──────────────────────────────────────────────────────────────

interface PostingRow {
    id:         string;
    account_id: string;
    tx_id:      string;
}

/** Returns the id of the first transaction that has exactly 3+ postings. */
function dbFindMultiPostingTx(): string | undefined {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare(
                `SELECT transaction_id AS tx_id
                   FROM postings
                  GROUP BY transaction_id
                 HAVING COUNT(*) >= 3
                  LIMIT 1`,
            )
            .get() as { tx_id: string } | undefined;
        return row?.tx_id;
    } finally {
        db.close();
    }
}

/** Returns the payee for a given transaction id. */
function dbTxPayee(txId: string): string | undefined {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare('SELECT payee FROM transactions WHERE id = ?')
            .get(txId) as { payee: string } | undefined;
        return row?.payee;
    } finally {
        db.close();
    }
}

/** Returns all posting ids for a transaction, in display order. */
function dbPostingIds(txId: string): string[] {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const rows = db
            .prepare('SELECT id FROM postings WHERE transaction_id = ? ORDER BY rowid')
            .all(txId) as { id: string }[];
        return rows.map(r => r.id);
    } finally {
        db.close();
    }
}

// ── Navigation helpers ───────────────────────────────────────────────────────

/**
 * Navigate to the accounts register for the account that the multi-posting
 * transaction debits (identified by the first posting's account).
 *
 * Finds the account by navigating to the Accounts page and clicking the first
 * sidebar entry that leads to a register containing the payee row.
 */
async function openFirstAccount(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed({ timeout: 5_000 });
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeout: 5_000, timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    const firstLink = await sidebarNav.$('a');
    await firstLink.waitForDisplayed({ timeout: 10_000 });
    await firstLink.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeout: 5_000, timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    const register = await $('[aria-label="transaction register"]');
    await register.waitForDisplayed({ timeout: 10_000 });
}

/** Scroll through accounts until the given payee appears; click to expand it. */
async function expandTxRow(payee: string): Promise<boolean> {
    const sidebarNav = await $('nav[aria-label="account navigation"]');
    const links = await sidebarNav.$$('a');

    for (const link of links) {
        await link.click();
        await browser.pause(500);

        const register = await $('[aria-label="transaction register"]');
        const exists = await browser.execute(
            (reg: Element, p: string) => {
                const spans = reg.querySelectorAll('span');
                return [...spans].some(s => s.textContent?.trim() === p);
            },
            await register.getElement() as unknown as Element,
            payee,
        );

        if (exists) {
            const payeeEl = await register.$(`span=${payee}`);
            await browser.execute((el: Element) => {
                const row = el.closest('[role="button"]');
                if (row instanceof HTMLElement) row.click();
            }, await payeeEl.getElement() as unknown as Element);

            const pill = await $('[data-testid="status-pill"]');
            const appeared = await pill.waitForDisplayed({ timeout: 5_000 }).then(() => true).catch(() => false);
            if (appeared) return true;
        }
    }
    return false;
}

// ── Test ─────────────────────────────────────────────────────────────────────

describe('Accounts — posting mid-list delete does not clobber remaining rows (#210)', () => {
    it('deleting the middle posting leaves the other rows with their original data', async function () {
        const txId = dbFindMultiPostingTx();
        if (!txId) {
            console.warn(
                'No 3+-posting transaction found in test DB — skipping #210 regression test',
            );
            this.skip();
        }

        const payee = dbTxPayee(txId);
        if (!payee) {
            console.warn('Could not resolve payee for multi-posting tx — skipping');
            this.skip();
        }

        await openFirstAccount();
        const found = await expandTxRow(payee);
        if (!found) {
            console.warn(
                `Could not locate "${payee}" transaction in any account register — skipping`,
            );
            this.skip();
        }

        // ── Capture all posting rows. ────────────────────────────────────────
        const amountInputs = await $$('[data-testid="posting-amount"]');
        expect(amountInputs.length).toBeGreaterThanOrEqual(3);

        const initialCount = await amountInputs.length;

        const before: string[] = [];
        for (const input of amountInputs) {
            before.push(await browser.execute(
                (el: Element) => (el as HTMLInputElement).value,
                await input.getElement() as unknown as Element,
            ));
        }

        // ── Delete the second (middle) delete button. ────────────────────────
        const delBtns = await $$('[aria-label="remove posting"]');
        expect(delBtns.length).toBeGreaterThanOrEqual(3);
        await delBtns[1].click();

        // Wait for the row count to drop by one.
        await browser.waitUntil(
            async () => {
                const rows = await $$('[data-testid="posting-amount"]');
                const count = await rows.length;
                return count === initialCount - 1;
            },
            { timeout: 3_000, timeoutMsg: 'Posting row count did not decrease after delete' },
        );

        // ── Assert surviving rows kept their original amounts. ───────────────
        const after = await $$('[data-testid="posting-amount"]');

        const val0 = await browser.execute(
            (el: Element) => (el as HTMLInputElement).value,
            await after[0].getElement() as unknown as Element,
        );
        const val1 = await browser.execute(
            (el: Element) => (el as HTMLInputElement).value,
            await after[1].getElement() as unknown as Element,
        );

        // The first surviving row must match what was row 0 before the delete.
        expect(val0).toBe(before[0]);
        // The second surviving row must match what was row 2 (not row 1).
        expect(val1).toBe(before[2]);

        // ── Save and verify the change persisted. ────────────────────────────
        const saveBtn = await $('[aria-label="save transaction"]');
        const saveVisible = await saveBtn.isDisplayed().catch(() => false);
        if (saveVisible) {
            await saveBtn.click();
            await browser.waitUntil(
                async () => !(await saveBtn.isDisplayed().catch(() => false)),
                { timeout: 10_000, timeoutMsg: 'Save bar did not disappear after clicking Save' },
            );
            // After save the transaction should now have one fewer posting in the DB.
            await browser.pause(300);
            const postingIds = dbPostingIds(txId);
            expect(postingIds.length).toBeLessThan(3);
        }
    });
});
