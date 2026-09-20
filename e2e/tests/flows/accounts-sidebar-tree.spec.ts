/**
 * Flow tests for the recursive account sidebar and the "include
 * sub-accounts" toggle.
 *
 * Seed data (see crates/bc-seed/src/main.rs) nests
 * `Expenses > Utilities > Water > Sewer`, so Sewer sits at depth 3. Roots
 * open on first load; every deeper branch is reached through its chevron
 * (`button[aria-expanded]` labelled `toggle <name>`). Utilities holds no
 * postings of its own, so its figure is the visible difference between the
 * toggle's two states.
 *
 * Expansion and the toggle persist in the WebView's localStorage, which
 * lives under the app's data directory and so outlives a session and is
 * shared by every worker. Each test clears both keys before it mounts the
 * page and again afterwards, so no state crosses between tests or specs.
 */
import { browser, $ } from '@wdio/globals';

const EXPANDED_KEY = 'bc.sidebar.expanded';
const ROLLUP_KEY = 'bc.accounts.rollup';

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Drops the persisted expansion set and toggle so the next mount seeds fresh. */
async function resetSidebarPrefs(): Promise<void> {
    await browser.execute((keys: string[]) => {
        for (const key of keys) {
            window.localStorage.removeItem(key);
        }
    }, [EXPANDED_KEY, ROLLUP_KEY]);
}

/**
 * Mounts the bare `/accounts` route fresh. The page is left first, via the
 * budget tab, so the accounts route is unmounted even when the session is
 * already on it; a nav click that lands on the current route changes
 * nothing, and one from `/accounts/:id` swaps the DOM under a handle taken
 * too early. Each hop waits for its exact URL before the next.
 */
async function openAccountsPage(): Promise<void> {
    const mainNav = await $('nav[aria-label="main navigation"]');
    await (await mainNav.$('a=budget')).click();
    await browser.waitUntil(
        async () => (await browser.getUrl()).endsWith('/budget'),
        { timeoutMsg: 'URL did not reach /budget within 5 s' },
    );

    await (await $('[data-testid="nav-accounts"]')).click();
    await browser.waitUntil(
        async () => (await browser.getUrl()).endsWith('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();
}

/** The chevron button for the branch named `name`. */
function chevron(name: string): ReturnType<typeof $> {
    return $(`button[aria-label="toggle ${name}"]`);
}

/** Whether an account named `name` is currently rendered in the sidebar. */
async function sidebarShows(name: string): Promise<boolean> {
    const sidebarNav = await $('nav[aria-label="account navigation"]');
    return sidebarNav.$(`span=${name}`).isExisting();
}

/**
 * Reads the balance figure rendered beside the sidebar account `name`.
 * `SidebarRow` renders `<span>{name}</span><span>{figure}</span>` under the
 * same link, so this reads the name span's next sibling.
 */
async function sidebarFigure(name: string): Promise<string> {
    return browser.execute((account: string) => {
        const nav = document.querySelector('nav[aria-label="account navigation"]');
        const spans = Array.from(nav?.querySelectorAll('span') ?? []);
        const nameSpan = spans.find(s => s.textContent?.trim() === account);
        return nameSpan?.nextElementSibling?.textContent?.trim() ?? '';
    }, name);
}

/**
 * Reads the dashboard's closing-balance headline; see
 * `accounts-period-view.spec.ts` for the markup this walks.
 */
async function closingBalance(): Promise<string> {
    return browser.execute(() => {
        const spans = Array.from(document.querySelectorAll('span'));
        const idx = spans.findIndex(s => s.textContent?.trim() === '// closing');
        if (idx <= 0) return '';
        return spans[idx - 1].textContent?.trim() ?? '';
    });
}

/** The "include sub-accounts" checkbox in the dashboard header. */
function rollupToggle(): ReturnType<typeof $> {
    return $('label*=include sub-accounts').$('input[type="checkbox"]');
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Accounts — sidebar tree', () => {
    beforeEach(async () => {
        await resetSidebarPrefs();
    });

    afterEach(async () => {
        await resetSidebarPrefs();
    });

    it('reaches a depth-3 account through the branch chevrons', async () => {
        await openAccountsPage();

        // Roots open on first load, so depth 1 is visible and depth 2 is not.
        // The root seed is an effect off the account list, so it can land a
        // tick after the first sidebar link renders.
        await browser.waitUntil(
            async () => sidebarShows('Utilities'),
            { timeoutMsg: 'Utilities did not appear under the seeded Expenses root' },
        );
        expect(await sidebarShows('Water')).toBe(false);
        expect(await sidebarShows('Sewer')).toBe(false);

        const utilities = await chevron('Utilities');
        await utilities.waitForDisplayed();
        expect(await utilities.getAttribute('aria-expanded')).toBe('false');
        await utilities.click();

        await browser.waitUntil(
            async () => (await chevron('Utilities').getAttribute('aria-expanded')) === 'true',
            { timeoutMsg: 'Utilities chevron did not report aria-expanded="true" after a click' },
        );
        expect(await sidebarShows('Water')).toBe(true);
        expect(await sidebarShows('Sewer')).toBe(false);

        const water = await chevron('Water');
        await water.waitForDisplayed();
        await water.click();
        await browser.waitUntil(
            async () => sidebarShows('Sewer'),
            { timeoutMsg: 'Sewer did not appear after expanding Water' },
        );

        const sidebarNav = await $('nav[aria-label="account navigation"]');
        const sewer = await sidebarNav.$('span=Sewer');
        await sewer.click();
        await browser.waitUntil(
            async () => (await browser.getUrl()).includes('/accounts/'),
            { timeoutMsg: 'URL did not update to the Sewer account route within 5 s' },
        );
        await $('[aria-label="transaction register"]').waitForDisplayed();

        // Collapsing the outer branch hides the whole subtree beneath it.
        await chevron('Utilities').click();
        await browser.waitUntil(
            async () => (await chevron('Utilities').getAttribute('aria-expanded')) === 'false',
            { timeoutMsg: 'Utilities chevron did not report aria-expanded="false" after a click' },
        );
        expect(await sidebarShows('Water')).toBe(false);
        expect(await sidebarShows('Sewer')).toBe(false);
    });

    it('persists an expanded branch across a remount', async () => {
        await openAccountsPage();

        const utilities = await chevron('Utilities');
        await utilities.waitForDisplayed();
        await utilities.click();
        await browser.waitUntil(
            async () => sidebarShows('Water'),
            { timeoutMsg: 'Water did not appear after expanding Utilities' },
        );

        await openAccountsPage();
        await browser.waitUntil(
            async () => sidebarShows('Water'),
            { timeoutMsg: 'Water was not visible after remounting with Utilities persisted open' },
        );
        expect(await chevron('Utilities').getAttribute('aria-expanded')).toBe('true');
    });

    it('keeps every root collapsed across a remount', async () => {
        await openAccountsPage();
        await browser.waitUntil(
            async () => sidebarShows('Checking'),
            { timeoutMsg: 'Checking did not appear under the seeded Assets root' },
        );

        // Collapse each open root in turn. A collapsed root takes its subtree
        // out of the DOM, so this loop ends with no open chevron anywhere.
        for (;;) {
            const open = await $('button[aria-expanded="true"]');
            if (!(await open.isExisting())) break;
            await open.click();
            await browser.waitUntil(
                async () => (await open.getAttribute('aria-expanded')) === 'false',
                { timeoutMsg: 'A root chevron did not collapse after a click' },
            );
        }
        expect(await sidebarShows('Checking')).toBe(false);

        // The empty set is a choice, not an unseeded sidebar: a remount must
        // not put the roots back.
        await openAccountsPage();
        expect(await sidebarShows('Assets')).toBe(true);
        expect(await sidebarShows('Checking')).toBe(false);
        expect(await $('button[aria-expanded="true"]').isExisting()).toBe(false);
    });

    it('folds sub-account balances into the parent when the toggle is on', async () => {
        await openAccountsPage();

        const sidebarNav = await $('nav[aria-label="account navigation"]');
        const utilities = await sidebarNav.$('span=Utilities');
        await utilities.waitForDisplayed();
        await utilities.click();
        await $('[aria-label="transaction register"]').waitForDisplayed();

        // Default is on: Utilities shows its subtree's figure.
        const toggle = await rollupToggle();
        await toggle.waitForDisplayed();
        expect(await toggle.isSelected()).toBe(true);
        await browser.waitUntil(
            async () => {
                const figure = await sidebarFigure('Utilities');
                return figure !== '' && figure !== '—';
            },
            { timeoutMsg: 'Utilities did not show a rolled-up figure with the toggle on' },
        );
        const rolledFigure = await sidebarFigure('Utilities');
        await browser.waitUntil(
            async () => {
                const closing = await closingBalance();
                return closing !== '' && closing !== '—';
            },
            { timeoutMsg: 'Dashboard closing balance did not resolve with the toggle on' },
        );
        const rolledClosing = await closingBalance();

        // Off: Utilities has no postings of its own, so its figure is the
        // em-dash placeholder and the dashboard closing balance changes.
        await toggle.click();
        await browser.waitUntil(
            async () => (await sidebarFigure('Utilities')) === '—',
            { timeoutMsg: 'Utilities figure did not fall back to "—" with the toggle off' },
        );
        await browser.waitUntil(
            async () => (await closingBalance()) !== rolledClosing,
            { timeoutMsg: 'Dashboard closing balance did not change with the toggle off' },
        );

        // On again: the rolled-up figure returns.
        await rollupToggle().click();
        await browser.waitUntil(
            async () => (await sidebarFigure('Utilities')) === rolledFigure,
            { timeoutMsg: 'Utilities figure did not return to its rolled-up value' },
        );
    });
});
