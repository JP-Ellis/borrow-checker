/**
 * Flow test for Settings → Transfers (transfer-suggestion review panel).
 *
 * The seed (`bc-seed`) creates three mergeable pairs of single-posting,
 * interim-unbalanced transactions. This test navigates to the panel, merges
 * the first suggestion, and asserts (a) the merged card disappears and (b) the
 * fusion is reflected in the DB: merging collapses two single-posting legs into
 * one two-posting transaction, so the number of single-posting transactions
 * drops by exactly two. The DB check is a before/after delta bracketing the
 * merge, so it is independent of ids and of any state other flow tests leave.
 */
import Database           from 'better-sqlite3';
import { browser, $, $$ } from '@wdio/globals';
import { DB_PATH } from '../support/db.js';

/** Count transactions that currently have exactly one posting. */
function singlePostingCount(): number {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare(
                'SELECT COUNT(*) AS n FROM ' +
                '(SELECT transaction_id FROM postings GROUP BY transaction_id HAVING COUNT(*) = 1)',
            )
            .get() as { n: number };
        return row.n;
    } finally {
        db.close();
    }
}

/** Navigate to Settings → Transfers and wait for suggestion cards to render. */
async function openSettingsTransfers(): Promise<void> {
    const nav = await $('nav[aria-label="main navigation"]');
    await nav.waitForExist();
    const settingsLink = await nav.$('a=settings');
    await settingsLink.waitForDisplayed();
    await settingsLink.click();

    await browser.waitUntil(
        () => browser.execute(() => window.location.pathname === '/settings'),
        { timeoutMsg: 'Pathname did not reach /settings within 5 s' },
    );

    const transfersNav = await $('[data-testid="settings-nav-transfers"]');
    await transfersNav.waitForDisplayed();
    await transfersNav.click();

    await browser.waitUntil(
        async () => (await $$('[data-testid="transfer-suggestion"]').length) > 0,
        { timeoutMsg: 'No transfer suggestion cards appeared' },
    );
}

describe('Settings → Transfers', () => {
    it('merges a suggested pair and fuses the two legs', async () => {
        await openSettingsTransfers();

        const before = await $$('[data-testid="transfer-suggestion"]');
        expect(await before.length).toBe(3);

        // Snapshot the single-posting-leg count before merging. Merging fuses
        // two single-posting legs into one two-posting transaction, so exactly
        // two single-posting legs disappear.
        const singlesBefore = singlePostingCount();

        // Merge the first card.
        const firstCard = before[0];
        const mergeBtn = await firstCard.$('[data-testid="transfer-merge"]');
        await mergeBtn.click();

        // The card disappears — two remain.
        await browser.waitUntil(
            async () => (await $$('[data-testid="transfer-suggestion"]').length) === 2,
            { timeoutMsg: 'Merged card did not disappear' },
        );

        // The DB reflects the fusion: the merged pair's two single-posting legs
        // collapse into one two-posting transaction, so the single-posting count
        // drops by exactly two. Poll to allow the backend write to land.
        await browser.waitUntil(
            () => singlePostingCount() === singlesBefore - 2,
            { timeoutMsg: 'Merged legs were not fused in the DB' },
        );
    });
});
