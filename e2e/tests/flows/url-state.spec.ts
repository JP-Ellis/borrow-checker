/**
 * Flow tests for the query and display window kept in the URL (#338).
 *
 * The palette query lives in the `q` URL parameter and the accounts window in
 * `period` and `start`. Reloads and back/forward restore them silently. A
 * palette commit, a chip removal and a granularity choice push a history
 * entry; stepping and jump to latest replace it. A top-bar tab keeps the
 * query. Jump to latest exists only in a period window and is disabled while
 * the query sets dates. A load at bare `/` replays the last location from
 * localStorage and announces it with a toast.
 *
 * Seed data (crates/bc-seed/src/fixture.rs) is relative to today: Checking has
 * transactions in the current month. Transport has none this month; its
 * newest is last month, and its only `commute`-tagged transaction is six
 * months ago.
 */
import { browser, $, expect } from '@wdio/globals';

import { freshView, searchParams } from '../support/nav.js';
import { commitDateFromToken, commitTagToken, commitTextToken } from '../support/palette.js';

const MONTHS = [
    'January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December',
];

/** `window_label` for `Period::Monthly`, `offset` months from today. */
function monthLabel(offset: number): string {
    const d = new Date();
    d.setDate(1);
    d.setMonth(d.getMonth() + offset);
    return `${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

/** The register header's window label; `''` in all time. */
async function windowLabel(): Promise<string> {
    return browser.execute(() => {
        const select = document.querySelector('[aria-label="transaction register"] select');
        const row = select?.parentElement;
        const label = row ? Array.from(row.children).find(el => el.tagName === 'SPAN') : null;
        return label?.textContent?.trim() ?? '';
    });
}

async function waitForLabel(expected: string): Promise<void> {
    await browser.waitUntil(async () => (await windowLabel()) === expected, {
        timeoutMsg: `window label did not become "${expected}"`,
    });
}

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

async function selectGranularity(value: string): Promise<void> {
    const select = await $('[aria-label="transaction register"] select');
    await select.waitForDisplayed();
    await select.selectByAttribute('value', value);
}

async function step(direction: 'previous' | 'next' | 'latest'): Promise<void> {
    await $(`button[aria-label="${direction} period"]`).click();
}

function toastWithText(text: string) {
    return $(`//*[contains(normalize-space(.), "${text}") and .//button[normalize-space()="Clear"]]`);
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

    it('a reload restores the query and window without a toast', async () => {
        await openAccount('Checking');
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        await commitTagToken('recurring');
        await browser.keys('Escape');

        await browser.refresh();
        await $('[aria-label="transaction register"]').waitForDisplayed();

        await waitForLabel(monthLabel(0));
        expect(await chipCount()).toBe(1);
        expect((await searchParams()).get('q')).toBe('tag:recurring');
        expect(await (await toastWithText('Restored filter')).isExisting()).toBe(false);
    });

    it('steps replace and a granularity choice pushes', async () => {
        await openAccount('Checking');
        const accountUrl = await browser.getUrl();
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        await step('previous');
        await step('previous');
        await step('previous');
        await waitForLabel(monthLabel(-3));

        await browser.back();
        await browser.waitUntil(async () => (await browser.getUrl()) === accountUrl, {
            timeoutMsg: 'back did not undo the granularity choice in one step',
        });
        await waitForLabel('');
    });

    it('switching accounts keeps the query and window, and back returns', async () => {
        await openAccount('Checking');
        const checkingUrl = new URL(await browser.getUrl()).pathname;
        await selectGranularity('monthly');
        await step('previous');
        await waitForLabel(monthLabel(-1));
        await commitTagToken('recurring');
        await browser.keys('Escape');

        const sidebar = await $('nav[aria-label="account navigation"]');
        await (await sidebar.$('span=Transport')).click();
        await browser.waitUntil(async () => new URL(await browser.getUrl()).pathname !== checkingUrl);
        await waitForLabel(monthLabel(-1));
        expect(await chipCount()).toBe(1);

        await browser.back();
        await browser.waitUntil(async () => new URL(await browser.getUrl()).pathname === checkingUrl);
        await waitForLabel(monthLabel(-1));
    });

    it('jump to latest lands on the newest transaction month', async () => {
        await openAccount('Transport');
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        await step('previous');
        await step('previous');
        await step('previous');
        await waitForLabel(monthLabel(-3));

        await step('latest');
        await waitForLabel(monthLabel(-1));
    });

    it('jump to latest under a query lands on the newest matching month', async () => {
        await openAccount('Transport');
        await commitTagToken('commute');
        await browser.keys('Escape');
        await browser.waitUntil(async () => (await chipCount()) === 1);
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        await step('previous');
        await waitForLabel(monthLabel(-1));

        await step('latest');
        await waitForLabel(monthLabel(-6));
    });

    it('jump to latest with no match shows a toast and keeps the window', async () => {
        await openAccount('Transport');
        await commitTextToken('zzzqxnomatch');
        await browser.waitUntil(async () => (await chipCount()) === 1);
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        await step('previous');
        await waitForLabel(monthLabel(-1));

        await step('latest');
        const toast = await $('//*[contains(normalize-space(text()), "No matching transactions")]');
        await toast.waitForDisplayed({ timeoutMsg: 'no "No matching transactions" toast appeared' });
        await waitForLabel(monthLabel(-1));
    });

    it('jump to latest is absent in all time', async () => {
        await openAccount('Checking');
        await waitForLabel('');
        expect(await $('button[aria-label="latest period"]').isExisting()).toBe(false);
    });

    it('jump to latest is disabled while the query sets dates', async () => {
        await openAccount('Checking');
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        const latest = await $('button[aria-label="latest period"]');
        await latest.waitForDisplayed();
        expect(await latest.isEnabled()).toBe(true);

        const from = new Date();
        from.setDate(1);
        from.setMonth(from.getMonth() - 3);
        const iso = `${from.getFullYear()}-${String(from.getMonth() + 1).padStart(2, '0')}-01`;
        await commitDateFromToken(iso);
        await browser.waitUntil(async () => (await chipCount()) === 1);

        await browser.waitUntil(async () => !(await $('button[aria-label="latest period"]').isEnabled()), {
            timeoutMsg: 'jump to latest stayed enabled under a date term',
        });
    });

    it('a cold start replays the last location, announces it, and Clear undoes it', async () => {
        await openAccount('Checking');
        await selectGranularity('monthly');
        await waitForLabel(monthLabel(0));
        await commitTagToken('recurring');
        await browser.keys('Escape');
        const restoredUrl = await browser.getUrl();

        await browser.execute(() => window.history.replaceState(null, '', '/'));
        await browser.refresh();

        await browser.waitUntil(async () => (await browser.getUrl()) === restoredUrl, {
            timeoutMsg: 'the saved location was not replayed',
        });
        const toast = await toastWithText(`Restored filter: tag:recurring · ${monthLabel(0)}`);
        await toast.waitForDisplayed();
        await (await toast.$('button=Clear')).click();

        await browser.waitUntil(async () => (await chipCount()) === 0, { timeoutMsg: 'Clear kept the chip' });
        await waitForLabel('');

        await browser.back();
        await browser.waitUntil(async () => (await chipCount()) === 1, { timeoutMsg: 'back did not undo Clear' });
        await waitForLabel(monthLabel(0));
        await browser.pause(1000);
        expect(await (await toastWithText('Restored filter')).isExisting()).toBe(false);

        await $('[data-testid="nav-dashboard"]').click();
        await browser.waitUntil(async () => new URL(await browser.getUrl()).pathname === '/');
        await browser.pause(1000);
        expect(await (await toastWithText('Restored filter')).isExisting()).toBe(false);
    });
});
