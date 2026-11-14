/**
 * Flow tests for the inline new-transaction draft on an account register.
 *
 * The add button opens a draft at the top of the register, seeded with the
 * viewed account as leg 1. Warnings come from the server: `Expenses :: Dining`
 * is closed in `before`, so a posting into it dated today draws a warning
 * anchored to that account. The seed (crates/bc-seed/src/fixture.rs) has no
 * closed account of its own.
 */
import { browser, $, expect } from '@wdio/globals';

import { dbTransactionIdByPayee } from '../support/db.js';
import {
    DRAFT,
    dbCloseAccount,
    dbCountByPayee,
    dbPostings,
    expandedCount,
    leg,
    pickAccount,
    saveButton,
    setAmount,
    setPayee,
    toastText,
    waitForEditor,
} from '../support/entry.js';
import { freshView, openAccount } from '../support/nav.js';

const REGISTER = '[aria-label="transaction register"]';

/** Opens the Checking register and its inline draft. */
async function openDraft(): Promise<void> {
    await freshView();
    await openAccount('Checking');
    await $('[data-testid="add-tx"]').waitForDisplayed();
    await $('[data-testid="add-tx"]').click();
    await waitForEditor(DRAFT);
}

/** Waits for the draft to leave the page. */
async function waitForDraftGone(): Promise<void> {
    await browser.waitUntil(
        async () => !(await $(DRAFT).isExisting()),
        { timeoutMsg: 'the draft stayed on the page' },
    );
}

describe('Accounts — inline transaction draft', () => {
    before(() => {
        dbCloseAccount('Dining', '2020-01-01');
    });

    it('inline draft is seeded with the viewed account', async () => {
        await openDraft();

        const account = (await leg(DRAFT, 0)).$('[data-testid="account-input"]');
        expect(await account.getValue()).toBe('Assets :: Checking');
        expect(await (await leg(DRAFT, 0)).$('[data-testid="posting-amount"]').getValue()).toBe('');
        expect((await $(DRAFT).$('[data-testid="status-pill"]').getText()).toLowerCase())
            .toBe('unreconciled');
        // One blank leg beside the seeded one.
        expect(await $(DRAFT).$$('[data-testid="posting-row"]').getElements()).toHaveLength(2);
    });

    it('a blank second amount saves as elided', async () => {
        await openDraft();
        await setPayee(DRAFT, 'E2E Elided Payee');
        await setAmount(DRAFT, 0, '-12.50');
        await pickAccount(DRAFT, 1, 'Groceries');
        await saveButton(DRAFT).click();

        expect(await toastText('success')).toContain('Transaction saved');
        await waitForDraftGone();

        const id = dbTransactionIdByPayee('E2E Elided Payee');
        expect(id).toBeDefined();
        const postings = dbPostings(id!);
        expect(postings).toHaveLength(2);
        expect(postings.filter(p => p.amount === null)).toHaveLength(1);
        expect(postings.find(p => p.amount !== null)?.account_name).toBe('Checking');
    });

    it('a draft saved twice creates one transaction', async () => {
        await openDraft();
        await setPayee(DRAFT, 'E2E Double Save Payee');
        await setAmount(DRAFT, 0, '-3.00');
        await pickAccount(DRAFT, 1, 'Groceries');
        // Two clicks in one tick, before the first save's round trip can end.
        await browser.execute((el: Element) => {
            (el as HTMLElement).click();
            (el as HTMLElement).click();
        }, await saveButton(DRAFT).getElement() as unknown as Element);

        await waitForDraftGone();
        await browser.pause(500); // a second write would land within this window
        expect(dbCountByPayee('E2E Double Save Payee')).toBe(1);
    });

    it('a warned save keeps the row open with the band', async () => {
        await openDraft();
        await setPayee(DRAFT, 'E2E Warned Inline Payee');
        await setAmount(DRAFT, 0, '-4.00');
        await pickAccount(DRAFT, 1, 'Dining');
        await saveButton(DRAFT).click();

        await $('[data-testid="tx-warnings"]').waitForDisplayed({
            timeoutMsg: 'the warning band did not appear on the saved row',
        });
        expect(await $('[data-testid="tx-warnings"]').getText()).toContain('Dining');
        await waitForDraftGone();

        const warned = $('[data-testid="posting-row"][data-warned="true"]');
        await warned.waitForDisplayed({ timeoutMsg: 'no posting row carries data-warned' });
        expect(await warned.$('[data-testid="account-input"]').getValue()).toContain('Dining');
        // The row is expanded, so the user sees it.
        expect(await expandedCount()).toBe(1);
        expect(await toastText('warn')).toContain('Transaction saved with warnings');
    });

    it('an unbalanced save shows the residual as the balance line does', async () => {
        await openDraft();
        await setPayee(DRAFT, 'E2E Unbalanced Inline Payee');
        await setAmount(DRAFT, 0, '-4.00');
        await pickAccount(DRAFT, 1, 'Groceries');
        await setAmount(DRAFT, 1, '3.00');

        const balance = $(DRAFT).$('span*=unbalanced');
        await balance.waitForDisplayed({ timeoutMsg: 'the balance line did not report unbalanced' });
        const residual = (await balance.getText()).split('= ')[1];
        expect(residual).toBeTruthy();
        await saveButton(DRAFT).click();

        const band = $(`${REGISTER} [data-testid="tx-warnings"]`);
        await band.waitForDisplayed({ timeoutMsg: 'the unbalanced save showed no warning band' });
        expect(await band.getText()).toContain(`unbalanced by ${residual}`);
    });

    it('changing the seeded account takes the overlay path', async () => {
        await openDraft();
        await setPayee(DRAFT, 'E2E Moved Leg Payee');
        await pickAccount(DRAFT, 0, 'Savings');
        await setAmount(DRAFT, 0, '-5.00');
        await pickAccount(DRAFT, 1, 'Dining');
        await saveButton(DRAFT).click();

        expect(await toastText('warn')).toContain('Transaction saved with warnings');
        await $('[popover] [data-kind="warn"]').$('button=View').waitForExist({ timeoutMsg: 'the toast offered no "View"' });
        await waitForDraftGone();
        // No row for this transaction is in the Checking register to carry a band.
        expect(await $('[data-testid="tx-warnings"]').isExisting()).toBe(false);
    });

    it('the draft survives expanding another row', async () => {
        await openDraft();
        await setPayee(DRAFT, 'E2E Survivor Payee');

        const row = $(`${REGISTER} [data-tx-id]`);
        await row.waitForDisplayed({ timeoutMsg: 'no register row to expand' });
        await browser.execute((el: Element) => (el as HTMLElement).click(), await row.getElement() as unknown as Element);

        await browser.waitUntil(
            async () => (await expandedCount()) === 1,
            { timeoutMsg: 'the register row did not expand' },
        );
        expect(await $(DRAFT).isDisplayed()).toBe(true);
        expect(await $(DRAFT).$('[data-testid="meta-value"]').getValue()).toBe('E2E Survivor Payee');
    });

    it('Escape closes the draft from a field', async () => {
        await openDraft();
        await (await leg(DRAFT, 0)).$('[data-testid="posting-amount"]').click();
        await browser.keys(['Escape']);
        await waitForDraftGone();

        // With the account menu open, the first Escape closes only the menu.
        await $('[data-testid="add-tx"]').click();
        await waitForEditor(DRAFT);
        // Typing opens the menu; focus alone does not land under WebDriver.
        await (await leg(DRAFT, 1)).$('[data-testid="account-input"]').setValue('a');
        await (await leg(DRAFT, 1)).$('ul').waitForDisplayed({ timeoutMsg: 'the account menu did not open' });
        await browser.keys(['Escape']);
        await browser.waitUntil(
            async () => !(await (await leg(DRAFT, 1)).$('ul').isExisting()),
            { timeoutMsg: 'Escape did not close the account menu' },
        );
        expect(await $(DRAFT).isDisplayed()).toBe(true);

        await browser.keys(['Escape']);
        await waitForDraftGone();
    });
});
