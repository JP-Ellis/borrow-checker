/**
 * The account picker's "manage accounts ↗" link on the desktop build.
 *
 * The link is a `_blank` link, which the webview drops unless `bc-app`
 * answers it. `bc-app` opens it in a second app window, so the half-filled
 * transaction in the first one survives. An account created in the second
 * window reaches the first window's picker when the first window regains
 * focus. The spec adds one account, Quokka Fund, a name no other spec
 * searches for.
 */
import { browser, $, expect } from '@wdio/globals';

import { DIALOG, leg, setAmount, setPayee, waitForEditor } from '../support/entry.js';
import { freshView } from '../support/nav.js';

const NO_MATCH = '// no account matches —';
const TYPED = 'quokka fund';
const PAYEE = 'E2E Picker Refetch Payee';

/** The first leg's account input, queried afresh. */
async function accountInput(): Promise<WebdriverIO.Element> {
    return (await leg(DIALOG, 0)).$('[data-testid="account-input"]').getElement();
}

/** Creates `Assets :: <name>` from Settings → Accounts in the current window. */
async function createAssetAccount(name: string): Promise<void> {
    await $('button=+ add account').click();
    const draft = $('[data-testid="account-draft"]');
    await draft.waitForDisplayed();
    await draft.$('input[aria-label="name"]').setValue(name);
    await draft.$('select[aria-label="parent"]').selectByVisibleText('Assets');
    await $('button=save 1 change').click();
    await browser.waitUntil(
        async () => !(await $('[data-testid="account-draft"]').isExisting()),
        { timeoutMsg: `the ${name} draft row did not save` },
    );
}

describe('Account picker "manage accounts" link', () => {
    beforeEach(async () => {
        await freshView();
    });

    it('opens Settings → Accounts in a new window and keeps the draft', async () => {
        const opener = await browser.getWindowHandle();

        await $('[data-testid="new-transaction"]').waitForDisplayed();
        await $('[data-testid="new-transaction"]').click();
        await waitForEditor(DIALOG);
        await setPayee(DIALOG, PAYEE);
        await setAmount(DIALOG, 0, '-4.50');
        await (await accountInput()).setValue(TYPED);
        const link = (await leg(DIALOG, 0)).$('a=manage accounts ↗');
        await link.waitForDisplayed({ timeoutMsg: `"${NO_MATCH}" menu did not show` });
        expect(await link.getAttribute('target')).toBe('_blank');

        await link.click();
        await browser.waitUntil(
            async () => (await browser.getWindowHandles()).length === 2,
            { timeoutMsg: 'the link opened no second window' },
        );
        const opened = (await browser.getWindowHandles()).find(handle => handle !== opener);
        if (opened === undefined) throw new Error('no second window handle');

        await browser.switchToWindow(opened);
        await browser.waitUntil(
            async () => {
                const url = new URL(await browser.getUrl());
                return url.pathname === '/settings' && url.searchParams.get('section') === 'accounts';
            },
            { timeoutMsg: 'the new window did not open Settings → Accounts' },
        );
        await $('[data-testid="accounts-table"]').waitForDisplayed();
        await createAssetAccount('Quokka Fund');
        await browser.closeWindow();

        await browser.switchToWindow(opener);
        // A WebDriver window switch does not raise the OS window, so the
        // window manager may never send `focus`. The event is sent here.
        await browser.execute(() => window.dispatchEvent(new FocusEvent('focus')));

        expect(await (await accountInput()).getValue()).toBe(TYPED);
        expect(await $(DIALOG).$('[data-testid="meta-value"]').getValue()).toBe(PAYEE);
        expect(await (await leg(DIALOG, 0)).$('[data-testid="posting-amount"]').getValue())
            .toBe('-4.50');

        // Reopening the menu on the same text now offers the new account.
        await (await accountInput()).click();
        await browser.keys(['End', ' ', 'Backspace']);
        const option = (await leg(DIALOG, 0)).$('li*=Quokka Fund');
        await option.waitForDisplayed({ timeoutMsg: 'the picker never offered Quokka Fund' });
        expect(await option.getText()).toContain('Assets');
        expect(await (await accountInput()).getValue()).toBe(TYPED);
    });
});
