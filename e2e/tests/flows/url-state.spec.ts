/**
 * Flow tests for the query kept in the URL (#338).
 *
 * The palette query lives in the `q` URL parameter. Back and forward undo and
 * redo a palette commit or a chip removal. A top-bar tab keeps the query.
 */
import { browser, $, expect } from '@wdio/globals';

import { freshView, searchParams } from '../support/nav.js';
import { commitTagToken } from '../support/palette.js';

async function chipCount(): Promise<number> {
    return browser.execute(
        () => document.querySelectorAll('[data-testid="filter-chips"] button[aria-label^="remove "]').length,
    );
}

async function openAccount(name: string): Promise<void> {
    const nav = await $('[data-testid="nav-accounts"]');
    await nav.waitForDisplayed();
    await nav.click();
    const sidebar = await $('nav[aria-label="account navigation"]');
    const account = await sidebar.$(`span=${name}`);
    await account.waitForDisplayed();
    await account.click();
    await browser.waitUntil(async () => (await browser.getUrl()).includes('/accounts/'));
    await $('[aria-label="transaction register"]').waitForDisplayed();
}

describe('URL state', () => {
    beforeEach(async () => {
        await freshView();
    });

    it('back undoes a palette commit and forward redoes it', async () => {
        await openAccount('Checking');
        await commitTagToken('recurring');
        await browser.keys('Escape');
        await browser.waitUntil(async () => (await chipCount()) === 1);

        await browser.back();
        await browser.waitUntil(async () => (await chipCount()) === 0, { timeoutMsg: 'back kept the chip' });
        await browser.forward();
        await browser.waitUntil(async () => (await chipCount()) === 1, { timeoutMsg: 'forward lost the chip' });
    });

    it('a top-bar tab keeps the query', async () => {
        await openAccount('Checking');
        await commitTagToken('recurring');
        await browser.keys('Escape');
        await browser.waitUntil(async () => (await chipCount()) === 1);

        await $('[data-testid="nav-budget"]').click();
        await browser.waitUntil(async () => new URL(await browser.getUrl()).pathname === '/budget', {
            timeoutMsg: 'the budget tab did not open /budget',
        });
        expect((await searchParams()).get('q')).toBe('tag:recurring');
        expect(await chipCount()).toBe(1);
    });

    it('back undoes a chip removal', async () => {
        await openAccount('Checking');
        await commitTagToken('recurring');
        await browser.keys('Escape');
        await browser.waitUntil(async () => (await chipCount()) === 1);

        await $('[data-testid="filter-chips"] button[aria-label^="remove "]').click();
        await browser.waitUntil(async () => (await chipCount()) === 0, { timeoutMsg: 'remove kept the chip' });

        await browser.back();
        await browser.waitUntil(async () => (await chipCount()) === 1, { timeoutMsg: 'back did not restore the chip' });
        expect((await searchParams()).get('q')).toBe('tag:recurring');
    });

    it('removing a chip after back and forward clears the address bar', async () => {
        await openAccount('Checking');
        await commitTagToken('recurring');
        await browser.keys('Escape');
        await browser.waitUntil(async () => (await chipCount()) === 1);

        await browser.back();
        await browser.waitUntil(async () => (await chipCount()) === 0, { timeoutMsg: 'back kept the chip' });
        await browser.forward();
        await browser.waitUntil(async () => (await chipCount()) === 1, { timeoutMsg: 'forward lost the chip' });

        const remove = await $('[data-testid="filter-chips"] button[aria-label^="remove "]');
        await remove.click();
        await browser.waitUntil(async () => (await chipCount()) === 0, { timeoutMsg: 'remove kept the chip' });
        expect((await searchParams()).get('q')).toBe(null);

        await browser.refresh();
        await $('nav[aria-label="main navigation"]').waitForDisplayed();
        expect(await chipCount()).toBe(0);
    });
});
