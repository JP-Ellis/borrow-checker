/**
 * The account picker's "manage accounts ↗" link on the desktop build.
 *
 * The link is a `_blank` link, which the webview drops unless `bc-app`
 * answers it. `bc-app` opens it in a second app window, so the half-filled
 * transaction in the first one survives.
 */
import { browser, $, expect } from '@wdio/globals';

import { DIALOG, leg, waitForEditor } from '../support/entry.js';
import { freshView } from '../support/nav.js';

const NO_MATCH = '// no account matches —';

describe('Account picker "manage accounts" link', () => {
    beforeEach(async () => {
        await freshView();
    });

    it('opens Settings → Accounts in a new window and keeps the draft', async () => {
        const opener = await browser.getWindowHandle();

        await $('[data-testid="new-transaction"]').waitForDisplayed();
        await $('[data-testid="new-transaction"]').click();
        await waitForEditor(DIALOG);
        const input = (await leg(DIALOG, 0)).$('[data-testid="account-input"]');
        await input.setValue('zzqx nothing');
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
        await browser.closeWindow();

        await browser.switchToWindow(opener);
        const kept = (await leg(DIALOG, 0)).$('[data-testid="account-input"]');
        expect(await kept.getValue()).toBe('zzqx nothing');
        expect(await $(`*=${NO_MATCH}`).isExisting()).toBe(false);
    });
});
