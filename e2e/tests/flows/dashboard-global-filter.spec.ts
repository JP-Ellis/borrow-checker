/**
 * Flow test proving the per-account dashboard (and its sticky-bar mirror)
 * recompute against the app-wide global filter: applying a `tag:` token
 * narrows the `transactions` stat tile and surfaces a muted "real" closing
 * balance alongside the filtered headline; clearing the chip restores the
 * unfiltered baseline.
 *
 * Reuses the ⌘K palette harness (imports, dialog/listbox selectors, token
 * commit flow) and chip helpers from `register-global-filter.spec.ts`
 * verbatim.
 */
import { browser, $, expect } from '@wdio/globals';

// ── Navigation helpers ───────────────────────────────────────────────────────

/** Navigate to Accounts → `name` via the top-bar nav and sidebar. */
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

    const balance = await $('[data-testid="dashboard-balance"]');
    await balance.waitForDisplayed();
}

// ── Dashboard stat helpers ───────────────────────────────────────────────────

/** Reads the dashboard headline closing-balance text. */
async function dashboardBalance(): Promise<string> {
    const el = await $('[data-testid="dashboard-balance"]');
    return (await el.getAttribute('textContent')) ?? '';
}

/**
 * Parses the transaction count out of the `transactions` StatCard tile.
 *
 * The testid sits on the whole tile (label + sublabel + value), not the
 * numeric value alone, so the count is regexed out of the tile's full text.
 */
async function dashboardTxCount(): Promise<number> {
    const el = await $('[data-testid="dashboard-tx-count"]');
    const text = (await el.getAttribute('textContent')) ?? '';
    const match = text.match(/\d+/);
    // During the async stats re-fetch the tile briefly renders an em-dash (no
    // digits). Return a sentinel 0 so the caller's `waitUntil` keeps polling
    // rather than throwing — the waiting logic lives in `waitForTxCount`.
    return match ? Number(match[0]) : 0;
}

/** Whether the dashboard's muted real-balance span is currently rendered. */
async function dashboardRealBalanceVisible(): Promise<boolean> {
    return browser.execute(
        () => document.querySelector('[data-testid="dashboard-real-balance"]') !== null,
    );
}

/** Reads the dashboard's muted real (unfiltered) closing-balance text. */
async function dashboardRealBalance(): Promise<string> {
    const el = await $('[data-testid="dashboard-real-balance"]');
    return (await el.getAttribute('textContent')) ?? '';
}

/** Reads a sticky-bar span's text by testid via the DOM (re-queried each call). */
async function stickyText(testid: string): Promise<string> {
    return browser.execute(
        id => document.querySelector(`[data-testid="${id}"]`)?.textContent ?? '',
        testid,
    );
}

/**
 * Waits until the dashboard's `transactions` tile count satisfies `predicate`.
 *
 * Applying/clearing a filter triggers an async stats re-fetch that lags the
 * (synchronously-updated) chip strip, so the tile count must be polled rather
 * than asserted immediately after the chip mutation.
 */
async function waitForTxCount(predicate: (count: number) => boolean, timeoutMsg: string): Promise<void> {
    await browser.waitUntil(async () => predicate(await dashboardTxCount()), { timeoutMsg, timeout: 15000 });
}

// ── Palette helpers (mirrors palette-filter-builder.spec.ts) ────────────────

/** Opens the ⌘K palette and returns the dialog element. */
async function openPalette() {
    const openButton = await $('button[aria-label="open command palette (⌘K)"]');
    await openButton.click();

    const dialog = await $('div[role="dialog"][aria-label="Command palette"]');
    await expect(dialog).toBeDisplayed();
    return dialog;
}

/**
 * Types `tag:<tagName>` into the palette's single inline search box, narrows
 * to the sole matching option, and commits it as a chip. Closes the palette
 * afterwards — chips sit behind the z-900 palette overlay, so the overlay
 * must be dismissed before any other top-bar interaction (chip removal).
 */
async function commitTagToken(tagName: string): Promise<void> {
    const dialog = await openPalette();

    const input = await dialog.$('input[role="combobox"]');
    await input.waitForDisplayed();
    await input.setValue(`tag:${tagName}`);

    const listbox = await $('#palette-listbox');
    await browser.waitUntil(
        async () => (await listbox.$$('div[role="option"]').length) === 1,
        { timeoutMsg: `expected the tag search to narrow to \`${tagName}\`` },
    );
    const only = await listbox.$('div[role="option"]');
    expect(await only.getAttribute('textContent')).toContain(tagName);
    await only.click();

    await browser.waitUntil(async () => (await input.getValue()) === '', {
        timeoutMsg: 'expected the input to clear after committing the token',
    });

    await browser.keys('Escape');
    await expect(dialog).not.toBeDisplayed();
}

// ── Filter chip helpers ──────────────────────────────────────────────────────

/** Clicks a chip's ✕ by its exact label (e.g. `"tag: recurring"`). */
async function removeChip(label: string): Promise<void> {
    const btn = await $(`[data-testid="filter-chips"] button[aria-label="remove ${label} filter"]`);
    await btn.waitForDisplayed();
    await btn.click();
}

/** Number of remove buttons currently rendered inside the chip strip. */
async function chipButtonCount(): Promise<number> {
    return browser.execute(
        () => document.querySelectorAll('[data-testid="filter-chips"] button').length,
    );
}

/**
 * Clears any filter chips left over from a spec that ran earlier in the same
 * suite (specs share one long-lived app session — see `wdio.conf.ts`; the
 * global filter store is provided once at the shell root and outlives route
 * changes). Keeps this spec self-contained regardless of run order.
 */
async function clearAllChips(): Promise<void> {
    for (let i = 0; i < 10; i += 1) {
        const count = await chipButtonCount();
        if (count === 0) return;
        const btn = await $('[data-testid="filter-chips"] button');
        await btn.click();
        await browser.waitUntil(
            async () => (await chipButtonCount()) < count,
            { timeoutMsg: 'chip removal did not reduce the chip count' },
        );
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Account dashboard — global filter', () => {
    it('recomputes stats against the active filter and mirrors them in the sticky bar', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await clearAllChips();

        // 1-2. Select CreditCard and capture the unfiltered baseline closing
        // balance + transaction count. The seed gives CreditCard current-month
        // `recurring`-tagged activity (a membership charge and a refund) plus a
        // pre-window tagged leg, so `tag:recurring` resolves to a smaller, live
        // in-window set — not an empty one.
        await openAccount('CreditCard');
        await waitForTxCount(count => count > 0, 'Dashboard tx-count tile never populated');
        const baselineBalance = await dashboardBalance();
        const baselineCount = await dashboardTxCount();
        expect(await dashboardRealBalanceVisible()).toBe(false);

        // 3-4. Commit `tag:recurring` — the tile count must drop to a positive
        // (non-empty) in-window subset and a muted real-balance span must
        // appear alongside the now-filtered headline.
        await commitTagToken('recurring');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('tag: recurring');

        await waitForTxCount(
            count => count > 0 && count < baselineCount,
            'Dashboard tx-count tile did not drop to a non-empty subset after tag:recurring',
        );
        await browser.waitUntil(
            async () => dashboardRealBalanceVisible(),
            { timeoutMsg: 'Muted real balance did not appear after applying the filter' },
        );

        // The filtered headline is a running total over the tagged set, so it
        // must differ from the real (unfiltered) closing. The muted span
        // restates that real closing, which equals the pre-filter baseline
        // headline (same window, no filter).
        const filteredBalance = await dashboardBalance();
        const mutedReal = await dashboardRealBalance();
        expect(filteredBalance).not.toBe(baselineBalance);
        // The muted span reads `real <amount>`; the amount is the unfiltered
        // closing, i.e. the pre-filter baseline headline.
        expect(mutedReal).toContain(baselineBalance);

        // 5. Scroll the main column past the dashboard so the sticky bar takes
        // over; while still filtered, it must mirror the FILTERED headline and
        // the muted real — not the all-time balance.
        //
        // The bar is toggled visible once the scroll container passes 180px
        // (see `on_scroll` in accounts/mod.rs). Scroll to the very bottom to
        // clear that threshold unconditionally, and dispatch a synthetic
        // scroll event so the Leptos handler reads the new offset immediately.
        const scrollContainer = await $('[data-testid="accounts-main-scroll"]');
        await browser.execute((el: HTMLElement) => {
            el.scrollTop = el.scrollHeight;
            el.dispatchEvent(new Event('scroll'));
        }, scrollContainer);

        // The sticky bar stays mounted at all times and only animates its
        // `max-height` between 0 and its resting size, so a plain "displayed"
        // check would pass even while collapsed. Re-query inside `execute`
        // each poll (its inner content re-renders in a `move ||` closure, so a
        // held element reference goes stale) and require a non-zero rendered
        // height — the reliable signal that the bar has expanded.
        await browser.waitUntil(
            async () => browser.execute(() => {
                const el = document.querySelector('[data-testid="sticky-balance"]');
                return el !== null && el.getBoundingClientRect().height > 0;
            }),
            { timeoutMsg: 'Sticky bar did not become visible after scrolling' },
        );

        expect(await stickyText('sticky-balance')).toBe(filteredBalance);
        expect(await stickyText('sticky-real-balance')).toBe(mutedReal);

        // 6. Clear the filter — the tile count returns to baseline, the muted
        // real balance disappears, and both the headline and the (still-
        // visible) sticky balance revert to the unfiltered baseline.
        await removeChip('tag: recurring');
        await expect($('[data-testid="filter-chips"]')).not.toBeDisplayed();

        await waitForTxCount(
            count => count === baselineCount,
            'Dashboard tx-count tile did not return to baseline after removing the chip',
        );
        await browser.waitUntil(
            async () => !(await dashboardRealBalanceVisible()),
            { timeoutMsg: 'Muted real balance did not disappear after removing the filter' },
        );
        expect(await dashboardBalance()).toBe(baselineBalance);
        await browser.waitUntil(
            async () => (await stickyText('sticky-balance')) === baselineBalance,
            { timeoutMsg: 'Sticky balance did not revert to baseline after clearing the filter' },
        );
    });
});
