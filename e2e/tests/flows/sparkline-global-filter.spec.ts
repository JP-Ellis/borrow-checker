/**
 * Flow test proving the per-account dashboard sparkline recomputes against
 * the app-wide global filter: applying a `tag:` token changes the rendered
 * cash-flow geometry (a membership-scoped subset of postings), and clearing
 * the chip restores the original unfiltered shape.
 *
 * Reuses the ⌘K palette harness (imports, dialog/listbox selectors, token
 * commit flow) and chip helpers from `register-global-filter.spec.ts` /
 * `dashboard-global-filter.spec.ts` verbatim.
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

// ── Sparkline helpers ─────────────────────────────────────────────────────

/** Reads the concatenated `points` of the sparkline polylines as a change signal. */
async function sparklineSignature(): Promise<string> {
    return browser.execute(() => {
        const root = document.querySelector('[data-testid="dashboard-sparkline"]');
        if (!root) return '';
        return Array.from(root.querySelectorAll('polyline, polygon'))
            .map(el => el.getAttribute('points') ?? '')
            .join('|');
    });
}

/** Waits until the dashboard sparkline wrapper is rendered. */
async function waitForSparkline(): Promise<void> {
    const el = await $('[data-testid="dashboard-sparkline"]');
    await el.waitForDisplayed();
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

describe('Account dashboard — sparkline global filter', () => {
    it('re-renders the sparkline under a tag filter and restores on clear', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await clearAllChips();

        // 1-2. Select CreditCard and capture the unfiltered sparkline geometry.
        // The seed gives CreditCard current-month `recurring`-tagged activity
        // (a membership charge and a refund, both inside the sparkline's
        // default trailing window) alongside other unrelated current-month
        // postings, so narrowing to `tag:recurring` changes at least one
        // bucket's net cash flow relative to the full unfiltered set.
        await openAccount('CreditCard');
        await waitForSparkline();
        await browser.waitUntil(
            async () => (await sparklineSignature()) !== '',
            { timeoutMsg: 'Sparkline never rendered any geometry' },
        );
        const before = await sparklineSignature();

        // 3. Commit `tag:recurring` — the sparkline resource re-fetches
        // against the filter's membership set and the rendered geometry must
        // differ from the unfiltered baseline.
        await commitTagToken('recurring');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('tag: recurring');

        await browser.waitUntil(
            async () => (await sparklineSignature()) !== before,
            {
                timeout: 15000,
                timeoutMsg: 'Sparkline geometry did not change after applying tag:recurring',
            },
        );

        // 4. Remove the chip — the sparkline resource re-fetches unfiltered
        // and the geometry must return to the original baseline.
        await removeChip('tag: recurring');
        await expect($('[data-testid="filter-chips"]')).not.toBeDisplayed();

        await browser.waitUntil(
            async () => (await sparklineSignature()) === before,
            {
                timeout: 15000,
                timeoutMsg: 'Sparkline geometry did not restore after removing the chip',
            },
        );
    });
});
