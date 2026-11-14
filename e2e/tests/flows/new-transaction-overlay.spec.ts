/**
 * Flow tests for the global "New transaction" overlay and the "View" link a
 * warned save leaves behind.
 *
 * `Expenses :: Dining` is closed in `before`, so a posting into it dated today
 * draws a warning (the seed has no closed account of its own).
 */
import { browser, $, expect } from '@wdio/globals';

import { dbTransactionIdByPayee } from '../support/db.js';
import {
    DIALOG,
    dbAccountId,
    dbCloseAccount,
    expandedCount,
    pickAccount,
    saveButton,
    setAmount,
    setPayee,
    toastText,
    hasToast,
    clickToastAction,
    waitForEditor,
} from '../support/entry.js';
import { freshView, openAccount, searchParams } from '../support/nav.js';

/** Opens the overlay from the top bar. */
async function openOverlay(): Promise<void> {
    await $('[data-testid="new-transaction"]').waitForDisplayed();
    await $('[data-testid="new-transaction"]').click();
    await waitForEditor(DIALOG);
}

async function waitForOverlayGone(): Promise<void> {
    await browser.waitUntil(
        async () => !(await $('[role="dialog"]').isExisting()),
        { timeoutMsg: 'the overlay stayed open' },
    );
}

describe('New transaction overlay', () => {
    before(() => {
        dbCloseAccount('Dining', '2020-01-01');
    });

    beforeEach(async () => {
        await freshView();
    });

    it('the top-bar button opens an unseeded overlay', async () => {
        await openOverlay();

        const rows = await $(DIALOG).$$('[data-testid="posting-row"]').getElements();
        expect(rows.length).toBeGreaterThanOrEqual(2);
        for (const row of rows) {
            expect(await row.$('[data-testid="account-input"]').getValue()).toBe('');
        }
        expect((await $(DIALOG).$('[data-testid="status-pill"]').getText()).toLowerCase())
            .toBe('unreconciled');
    });

    it('a clean save closes the overlay', async () => {
        await openOverlay();
        await setPayee(DIALOG, 'E2E Overlay Clean Payee');
        await pickAccount(DIALOG, 0, 'Checking');
        await setAmount(DIALOG, 0, '-6.00');
        await pickAccount(DIALOG, 1, 'Groceries');
        await saveButton(DIALOG).click();

        expect(await toastText('success')).toContain('Transaction saved');
        await waitForOverlayGone();
        expect(dbTransactionIdByPayee('E2E Overlay Clean Payee')).toBeDefined();
    });

    it('View on a warned save opens the row', async () => {
        await openOverlay();
        await setPayee(DIALOG, 'E2E Overlay Warned Payee');
        await pickAccount(DIALOG, 0, 'Checking');
        await setAmount(DIALOG, 0, '-8.00');
        await pickAccount(DIALOG, 1, 'Dining');
        await saveButton(DIALOG).click();

        expect(await toastText('warn')).toContain('Transaction saved with warnings');
        await waitForOverlayGone();
        // The toast has no band to carry the warnings, so it must outlive the
        // 7 s auto-dismiss.
        await browser.pause(8000);
        expect(await hasToast('warn')).toBe(true);

        const checkingId = dbAccountId('Checking');
        await clickToastAction('View');
        await browser.waitUntil(
            async () => new URL(await browser.getUrl()).pathname === `/accounts/${checkingId}`,
            { timeoutMsg: '"View" did not reach the first leg account' },
        );

        await $('[data-testid="tx-warnings"]').waitForDisplayed({
            timeoutMsg: 'the warning band did not show on the linked row',
        });
        expect(await expandedCount()).toBe(1);
        await browser.waitUntil(
            async () => !(await searchParams()).has('tx'),
            { timeoutMsg: '`tx` stayed in the URL after the row opened' },
        );
        // An all-time window narrows to the month of the date.
        expect((await searchParams()).get('period')).toBe('monthly');
    });

    it('a stale tx link shows the plain register', async () => {
        await openAccount('Checking');
        const path = new URL(await browser.getUrl()).pathname;
        // A history push keeps the app mounted, as following a link does.
        await browser.execute((target: string) => {
            window.history.pushState(null, '', target);
            window.dispatchEvent(new PopStateEvent('popstate'));
        }, `${path}?tx=missing`);
        await $('[aria-label="transaction register"]').waitForDisplayed();

        await browser.waitUntil(
            async () => !(await searchParams()).has('tx'),
            { timeoutMsg: '`tx=missing` stayed in the URL' },
        );
        expect(await expandedCount()).toBe(0);
        expect(await $('[role="alert"]').isExisting()).toBe(false);
    });
});
