/**
 * Lazy loading in the accounts register: `PAGE_SIZE = 100` rows come back per
 * `register_page` IPC call, and the header/sentinel/button only appear while
 * more rows remain (see `TransactionRegister` in
 * crates/bc-ui/src/pages/accounts/components/transaction_register/mod.rs).
 *
 * `Assets:Archive` (crates/bc-seed/src/main.rs) is a synthetic account seeded
 * with exactly 150 transactions for this purpose — every other seeded
 * account fits in a single page, so it never exercises this path.
 */
import { browser, $ } from '@wdio/globals';

// ── Navigation helpers (copied from accounts-period-view.spec.ts) ──────────

/**
 * Navigate to Accounts → `name` via the top-bar nav and sidebar. Always goes
 * through the bare `/accounts` route first, which unmounts/remounts the
 * Accounts page component — so every call starts from a freshly mounted
 * page, open in all time (the default window, so every one of Archive's 150
 * rows is in scope).
 */
async function openAccount(name: string): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();

    const accountSpan = await sidebarNav.$(`span=${name}`);
    await accountSpan.waitForDisplayed();
    await accountSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    const register = await $('[aria-label="transaction register"]');
    await register.waitForDisplayed();
}

/** Reads the number of transaction rows currently rendered in the register. */
async function registerRowCount(): Promise<number> {
    return browser.execute(
        () => document.querySelector('[aria-label="transaction register"]')
            ?.querySelectorAll('[role="button"]').length ?? 0,
    );
}

/** Waits until the register has at least one transaction row. */
async function waitForRegisterRows(): Promise<void> {
    await browser.waitUntil(
        async () => (await registerRowCount()) > 0,
        { timeoutMsg: 'No transaction rows appeared in the register within 15 s' },
    );
}

/**
 * Waits until the register shows exactly `count` rows, polling rather than a
 * one-shot check — every page fetch is an async IPC round-trip.
 */
async function waitForRowCount(count: number): Promise<void> {
    await browser.waitUntil(
        async () => (await registerRowCount()) === count,
        {
            timeout: 15_000,
            timeoutMsg: `register did not reach ${count} rows within 15 s`,
        },
    );
}

/**
 * The register header's `register [ N / total ]` (or `[ total ]` once fully
 * loaded) count. Found by text rather than a Stylance class name, which is
 * scoped and may be renamed on recompilation: the bracket span's own text is
 * always exactly `[`, so its next sibling is the count.
 */
async function registerCountText(): Promise<string> {
    return browser.execute(() => {
        const register = document.querySelector('[aria-label="transaction register"]');
        const spans = Array.from(register?.querySelectorAll('span') ?? []);
        const bracketIdx = spans.findIndex(s => s.textContent?.trim() === '[');
        if (bracketIdx === -1 || !spans[bracketIdx + 1]) return '';
        return spans[bracketIdx + 1].textContent?.trim() ?? '';
    });
}

/** Waits until the register's header count reads exactly `text`. */
async function waitForCountText(text: string): Promise<void> {
    await browser.waitUntil(
        async () => (await registerCountText()) === text,
        {
            timeout: 15_000,
            timeoutMsg: `register count did not read "${text}" within 15 s`,
        },
    );
}

describe('Accounts register — lazy loading', () => {
    it('loads the next page via the load-more button', async () => {
        await openAccount('Archive');
        await waitForRegisterRows();

        // First page: exactly PAGE_SIZE rows, the header reads "loaded /
        // total", and the sentinel + button are present.
        await waitForRowCount(100);
        await waitForCountText('100 / 150');

        expect(await $('[data-testid="register-sentinel"]').isExisting()).toBe(true);
        const loadMore = await $('[data-testid="load-more"]');
        expect(await loadMore.isExisting()).toBe(true);
        expect(await loadMore.getText()).toBe('load more · 50 remaining');

        await loadMore.click();

        // Second (final) page: every row loaded, header collapses to just
        // the total, sentinel and button are gone.
        await waitForRowCount(150);
        await waitForCountText('150');
        expect(await $('[data-testid="register-sentinel"]').isExisting()).toBe(false);
        expect(await $('[data-testid="load-more"]').isExisting()).toBe(false);
    });

    it('loads the next page by scrolling near the bottom', async () => {
        // Re-opening remounts the page, so this starts from a fresh
        // first-page load rather than depending on the previous test.
        await openAccount('Archive');
        await waitForRegisterRows();
        await waitForRowCount(100);

        // The page's own scroll handler asks for the next page once the
        // container is within two viewports of the bottom (see
        // crates/bc-ui/src/pages/accounts/mod.rs `on_scroll`). Driving that
        // from a synthetic `scrollTop` write is the flakiest route in this
        // spec: it depends on the container actually being the overflow
        // element `scrollHeight`/`clientHeight` are measured against, and a
        // headless/undersized WebView viewport can make "near the bottom"
        // trivially true or never true. If this proves unreliable in CI,
        // the load-more-button test above already covers the same paging
        // logic and can stand alone.
        await browser.execute(() => {
            const el = document.querySelector('[data-testid="accounts-main-scroll"]');
            if (!el) return;
            el.scrollTop = el.scrollHeight;
            el.dispatchEvent(new Event('scroll'));
        });

        await waitForRowCount(150);
    });
});
