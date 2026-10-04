/**
 * Regression test for #210 — stable posting uid (mid-list delete clobber).
 *
 * Before the fix, deleting a non-last posting from a transaction with 3+
 * postings caused Leptos to reuse component views by index, shifting the vec
 * so retained rows wrote stale data into the wrong slot on the next save.
 *
 * This test:
 *   1. Opens the seeded three-way split.
 *   2. Records the account names and amounts of all three rows.
 *   3. Deletes the *middle* posting (index 1).
 *   4. Asserts that the two surviving rows still display the data that
 *      originally belonged to postings 0 and 2 (not 1).
 *   5. Saves and verifies persistence via SQLite.
 *
 * bc-seed seeds the split under the payee "Costco" in Checking; the test
 * fails if it is missing.
 */
import Database             from 'better-sqlite3';
import { browser, $, $$ }  from '@wdio/globals';
import { DB_PATH, dbTransactionIdByPayee } from '../support/db.js';

// ── DB helpers ──────────────────────────────────────────────────────────────

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

/** Payee of bc-seed's three-way split (`crates/bc-seed/src/fixture.rs`). */
const SPLIT_PAYEE = 'Costco';

/** Opens the Checking register, which holds the seeded split. */
async function openChecking(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();
    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );
    const sidebarNav = await $('nav[aria-label="account navigation"]');
    const checking = await sidebarNav.$('span=Checking');
    await checking.waitForDisplayed();
    await checking.click();
    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );
    await (await $('[aria-label="transaction register"]')).waitForDisplayed();
}

/** Expands the register row whose payee reads exactly `payee`. */
async function expandRow(payee: string): Promise<void> {
    const register = await $('[aria-label="transaction register"]');
    const payeeEl = await register.$(`span=${payee}`);
    await payeeEl.waitForDisplayed();
    await browser.execute((el: Element) => {
        const row = el.closest('[role="button"]');
        if (row instanceof HTMLElement) row.click();
    }, await payeeEl.getElement() as unknown as Element);
    await (await $('[data-testid="status-pill"]')).waitForDisplayed();
}

// ── Test ─────────────────────────────────────────────────────────────────────

describe('Accounts — posting mid-list delete does not clobber remaining rows (#210)', () => {
    it('deleting the middle posting leaves the other rows with their original data', async () => {
        const txId = dbTransactionIdByPayee(SPLIT_PAYEE);
        if (!txId) {
            throw new Error(`bc-seed's fixture has no "${SPLIT_PAYEE}" transaction; the #210 test needs its three-way split`);
        }
        if (dbPostingAccounts(txId).length < 3) {
            throw new Error(`bc-seed's fixture "${SPLIT_PAYEE}" transaction has fewer than 3 postings; the #210 test needs its three-way split`);
        }

        await openChecking();
        await expandRow(SPLIT_PAYEE);

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
        // The split is three months back, so its row sits far down the register.
        await browser.execute(
            (el: Element) => el.scrollIntoView({ block: 'center' }),
            await delBtns[1].getElement() as unknown as Element,
        );
        await delBtns[1].click();

        // Wait for the row count to drop by one.
        await browser.waitUntil(
            async () => {
                const rows = await $$('[data-testid="posting-amount"]');
                return (await rows.length) === initialCount - 1;
            },
            { timeoutMsg: 'Posting row count did not decrease after delete' },
        );

        // ── Save. Deleting a leg leaves the transaction unbalanced, which is a
        //    saveable (flagged) state, so Save must still go through. ──────────
        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed();
        await saveBtn.click();
        await browser.waitUntil(
            async () => !(await saveBtn.isDisplayed().catch(() => false)),
            { timeoutMsg: 'Save bar did not disappear after clicking Save' },
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
