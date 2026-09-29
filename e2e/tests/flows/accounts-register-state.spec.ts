/**
 * Register state across account switches, saves and appends (#580, #581).
 * `Assets:Archive` (crates/bc-seed/src/main.rs) holds 150 transactions, so
 * it pages once and is long enough to scroll.
 */
import { browser, $ } from '@wdio/globals';

const ROWS = '[aria-label="transaction register"] [role="button"][aria-expanded]';
const MAIN = '[data-testid="accounts-main-scroll"]';
const EXPANDED_KEY = 'bc.sidebar.expanded';

/**
 * Opens Accounts → `name` from a freshly mounted Accounts page. Clears the
 * persisted sidebar expansion set first: it lives in the WebView's
 * localStorage and outlives a remount, so a collapsed state from an earlier
 * test can hide `Assets` (Archive's parent) and every other root behind it.
 * Roots open on first load once the key is gone.
 */
async function openAccount(name: string): Promise<void> {
    await browser.execute((key) => window.localStorage.removeItem(key), EXPANDED_KEY);
    await (await $('[data-testid="nav-accounts"]')).click();
    await browser.waitUntil(async () => (await browser.getUrl()).includes('/accounts'));
    await selectAccount(name);
}

/**
 * Clicks `name` in the sidebar without remounting the page. Waits for the
 * URL to change first: `waitSettled` alone can match a register that had
 * already settled for the *previous* account, since a fetch that lands
 * between the click and the wait leaves `aria-busy="false"` throughout.
 */
async function selectAccount(name: string): Promise<void> {
    const nav = await $('nav[aria-label="account navigation"]');
    await nav.$('a').waitForDisplayed();
    const urlBefore = await browser.getUrl();
    const span = await nav.$(`span=${name}`);
    await span.waitForDisplayed();
    await span.click();
    await browser.waitUntil(async () => (await browser.getUrl()) !== urlBefore, {
        timeoutMsg: `URL did not change after selecting ${name}`,
    });
    await waitSettled();
}

/** Waits until the register has rows and no request in flight. */
async function waitSettled(): Promise<void> {
    await browser.waitUntil(
        async () => browser.execute((rows) => {
            const reg = document.querySelector('[aria-label="transaction register"]');
            return !!reg && reg.getAttribute('aria-busy') === 'false'
                && document.querySelectorAll(rows).length > 0;
        }, ROWS),
        { timeout: 15_000, timeoutMsg: 'register did not settle within 15 s' },
    );
}

async function rowCount(): Promise<number> {
    return browser.execute((rows) => document.querySelectorAll(rows).length, ROWS);
}

async function scrollTop(): Promise<number> {
    return browser.execute((main) => document.querySelector(main)!.scrollTop, MAIN);
}

/** Every row's `data-tx-id`, in display order. */
async function rowIds(): Promise<string[]> {
    return browser.execute(
        (rows) => Array.from(document.querySelectorAll<HTMLElement>(rows))
            .map((row) => row.getAttribute('data-tx-id')!),
        ROWS,
    );
}

describe('Accounts register — state', () => {
    it('never shows the old account\'s rows during a switch (#581)', async () => {
        await openAccount('Archive');
        await browser.execute((rows) => document.querySelectorAll<HTMLElement>(rows)[2].click(), ROWS);
        await browser.waitUntil(async () =>
            browser.execute((rows) => document.querySelectorAll(`${rows}[aria-expanded="true"]`).length === 1, ROWS));

        const archiveIds = await rowIds();

        const nav = await $('nav[aria-label="account navigation"]');
        const span = await nav.$('span=Checking');
        await span.waitForDisplayed();
        await span.click();

        // Sample tightly through the switch rather than waiting once at the
        // end: the bug this guards (#581) is Archive's rows staying on
        // screen, under Checking's summary, while the fetch is in flight —
        // a single post-settle check would miss that window entirely.
        let staleSeen = false;
        for (let i = 0; i < 300; i++) {
            const sample = await rowIds();
            if (sample.some((id) => archiveIds.includes(id))) {
                staleSeen = true;
                break;
            }
            const reg = await browser.execute(() =>
                document.querySelector('[aria-label="transaction register"]')?.getAttribute('aria-busy'));
            if (reg === 'false' && sample.length > 0) break;
        }
        expect(staleSeen).toBe(false);

        await waitSettled();
        const expanded = await browser.execute(() =>
            document.querySelectorAll('[aria-label="transaction register"] [role="button"][aria-expanded="true"]').length);
        expect(expanded).toBe(0);
    });

    it('keeps the edited row in view after a save', async () => {
        await openAccount('Archive');
        await browser.execute(() =>
            document.querySelector<HTMLButtonElement>('[data-testid="load-more"]')?.click());
        await browser.waitUntil(async () => (await rowCount()) === 150, { timeout: 15_000 });

        const id = await browser.execute((rows) => {
            const row = document.querySelectorAll<HTMLElement>(rows)[120];
            row.scrollIntoView({ block: 'center' });
            row.click();
            return row.getAttribute('data-tx-id')!;
        }, ROWS);
        const desc = await $('input[placeholder="description"]');
        await desc.waitForDisplayed();
        await desc.addValue(' edited');
        const before = await scrollTop();
        expect(before).toBeGreaterThan(0);

        await (await $('[aria-label="save transaction"]')).click();
        await browser.waitUntil(async () =>
            !(await (await $('[aria-label="save transaction"]')).isDisplayed().catch(() => false)));
        await waitSettled();
        await browser.pause(100); // one frame for the anchor restore

        const inView = await browser.execute((main, txId) => {
            const container = document.querySelector(main)!.getBoundingClientRect();
            const row = document.querySelector(`[data-tx-id="${txId}"]`);
            if (!row) return false;
            const r = row.getBoundingClientRect();
            return r.top >= container.top && r.bottom <= container.bottom;
        }, MAIN, id);
        expect(inView).toBe(true);
    });

    it('keeps a re-sorted row in view after a save', async () => {
        await openAccount('Archive');
        await browser.execute(() =>
            document.querySelector<HTMLButtonElement>('[data-testid="load-more"]')?.click());
        await browser.waitUntil(async () => (await rowCount()) === 150, { timeout: 15_000 });

        const id = await browser.execute((rows) => {
            const row = document.querySelectorAll<HTMLElement>(rows)[120];
            row.scrollIntoView({ block: 'center' });
            row.click();
            return row.getAttribute('data-tx-id')!;
        }, ROWS);

        // Archive's seed data (crates/bc-seed/src/main.rs) never dates later
        // than today, and the register sorts date DESC, so a date one day
        // past today is guaranteed newer than every other loaded row: the
        // edited row moves to the very top of the list, off the viewport
        // this scroll was anchored to.
        const tomorrow = new Date();
        tomorrow.setDate(tomorrow.getDate() + 1);
        const isoTomorrow = tomorrow.toISOString().slice(0, 10);

        const dateInput = await $('input[placeholder="YYYY-MM-DD"]');
        await dateInput.waitForDisplayed();
        await dateInput.clearValue();
        await dateInput.setValue(isoTomorrow);

        const before = await scrollTop();
        expect(before).toBeGreaterThan(0);

        await (await $('[aria-label="save transaction"]')).click();
        await browser.waitUntil(async () =>
            !(await (await $('[aria-label="save transaction"]')).isDisplayed().catch(() => false)));
        await waitSettled();
        await browser.pause(100); // one frame for the anchor restore

        const after = await browser.execute((rows, txId) =>
            Array.from(document.querySelectorAll<HTMLElement>(rows)).findIndex(
                (row) => row.getAttribute('data-tx-id') === txId,
            ), ROWS, id);
        expect(after).toBe(0);

        const inView = await browser.execute((main, txId) => {
            const container = document.querySelector(main)!.getBoundingClientRect();
            const row = document.querySelector(`[data-tx-id="${txId}"]`);
            if (!row) return false;
            const r = row.getBoundingClientRect();
            return r.top >= container.top && r.bottom <= container.bottom;
        }, MAIN, id);
        expect(inView).toBe(true);
    });

    it('keeps the scroll position across the first append', async () => {
        await openAccount('Archive');
        await browser.waitUntil(async () => (await rowCount()) === 100, { timeout: 15_000 });

        const before = await browser.execute((main) => {
            const el = document.querySelector(main)!;
            el.scrollTop = el.scrollHeight - el.clientHeight * 2.5;
            el.dispatchEvent(new Event('scroll'));
            return el.scrollTop;
        }, MAIN);
        expect(before).toBeGreaterThan(0);

        await browser.waitUntil(async () => (await rowCount()) === 150, { timeout: 15_000 });
        await browser.pause(100);
        expect(Math.abs((await scrollTop()) - before)).toBeLessThan(5);
    });
});
