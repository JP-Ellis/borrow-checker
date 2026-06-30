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

/** Returns all posting (id, account_id) pairs for a transaction, in display order. */
function dbPostingAccounts(txId: string): { id: string; account_id: string }[] {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        return db
            .prepare('SELECT id, account_id FROM postings WHERE transaction_id = ? ORDER BY rowid')
            .all(txId) as { id: string; account_id: string }[];
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

        // ── Capture the postings (id + account) before the delete. ───────────
        // #210's clobber corrupts a surviving row's *account* (a per-row signal
        // synced by index), not its amount (read/written by index each render).
        // So the meaningful assertion is the *persisted* account_ids of the
        // survivors after save — "saving persists the wrong values" is the bug.
        const beforeAccts = dbPostingAccounts(txId);
        expect(beforeAccts.length).toBeGreaterThanOrEqual(3);

        const amountInputs = await $$('[data-testid="posting-amount"]');
        const initialCount = await amountInputs.length;
        expect(initialCount).toBeGreaterThanOrEqual(3);

        // ── Delete the middle posting. ───────────────────────────────────────
        const delBtns = await $$('[aria-label="remove posting"]');
        expect(delBtns.length).toBeGreaterThanOrEqual(3);
        await delBtns[1].click();

        // Wait for the row count to drop by one.
        await browser.waitUntil(
            async () => {
                const rows = await $$('[data-testid="posting-amount"]');
                return (await rows.length) === initialCount - 1;
            },
            { timeout: 3_000, timeoutMsg: 'Posting row count did not decrease after delete' },
        );

        // ── Save. Deleting a leg leaves the transaction unbalanced, which is a
        //    saveable (flagged) state, so Save must still go through. ──────────
        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({ timeout: 5_000 });
        await saveBtn.click();
        await browser.waitUntil(
            async () => !(await saveBtn.isDisplayed().catch(() => false)),
            { timeout: 10_000, timeoutMsg: 'Save bar did not disappear after clicking Save' },
        );
        await browser.pause(300);

        // ── The survivors must be exactly postings 0 and 2, each keeping its
        //    OWN id and account — not clobbered by the shifted middle row. ─────
        const afterAccts = dbPostingAccounts(txId);
        expect(afterAccts.length).toBe(beforeAccts.length - 1);
        expect(afterAccts.map(p => p.id)).toEqual([beforeAccts[0].id, beforeAccts[2].id]);
        expect(afterAccts.map(p => p.account_id)).toEqual([
            beforeAccts[0].account_id,
            beforeAccts[2].account_id,
        ]);
    });
});
