/**
 * Flow test proving the accounts-page transaction register consumes the
 * app-wide global filter (rather than the removed local all/pending/
 * uncategorised bar): selecting an account shows its transactions
 * intersected with the active filter's non-account dimensions, and a
 * partially-matching transaction dims its non-matching legs in the expanded
 * detail editor.
 *
 * Reuses the ⌘K palette harness (imports, dialog/listbox selectors, token
 * commit flow) from `palette-filter-builder.spec.ts` verbatim.
 */
import { browser, $, $$, expect } from '@wdio/globals';
import { commitTagToken } from '../support/palette.js';

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

/** Reads the shared `PeriodNav` window label (between the step buttons). */
async function periodNavLabel(): Promise<string> {
    return browser.execute(() => {
        const prev = document.querySelector('[aria-label="previous period"]');
        const next = document.querySelector('[aria-label="next period"]');
        if (!prev || !next) return '';
        let node = prev.nextElementSibling;
        while (node && node !== next) {
            const text = node.textContent?.trim();
            if (text) return text;
            node = node.nextElementSibling;
        }
        return '';
    });
}

/** Snapshot of the register's rendered content, used as a settle signal. */
async function registerSignature(): Promise<string> {
    return browser.execute(
        () => document.querySelector('[aria-label="transaction register"]')?.textContent ?? '',
    );
}

/**
 * Waits until the register's rendered content stops changing.
 *
 * The register refetches asynchronously when the period window changes, so its
 * rows lag the (synchronously-updated) `PeriodNav` label. Requiring the content
 * to be identical across several consecutive polls ensures any in-flight
 * refetch has landed before the caller inspects the rows — otherwise a lagging
 * refetch reads as "this window has no matching row" and the step-back loop
 * overshoots the target month.
 */
async function waitForRegisterSettled(): Promise<void> {
    let previous = await registerSignature();
    let stableReads = 0;
    await browser.waitUntil(
        async () => {
            const current = await registerSignature();
            stableReads = current === previous ? stableReads + 1 : 0;
            previous = current;
            return stableReads >= 2;
        },
        {
            interval: 150,
            timeout: 10000,
            timeoutMsg: 'Register did not settle after the period change',
        },
    );
}

/** Steps the shared period window back one step and waits for it to settle. */
async function stepToPreviousPeriod(): Promise<void> {
    const before = await periodNavLabel();
    const prevBtn = await $('[aria-label="previous period"]');
    await prevBtn.waitForDisplayed();
    await prevBtn.click();
    await browser.waitUntil(
        async () => (await periodNavLabel()) !== before,
        { timeoutMsg: 'Period label did not change after clicking "previous period"' },
    );
    // The label updates synchronously but the register refetch does not — wait
    // for the rows to settle before the caller decides whether this window
    // holds the target row, so a lagging refetch can't cause an overshoot.
    await waitForRegisterSettled();
}

/** True when a row containing `payee` is currently rendered in the register. */
async function registerContainsPayee(payee: string): Promise<boolean> {
    return browser.execute(
        (p: string) => (document.querySelector('[aria-label="transaction register"]')
            ?.textContent ?? '').includes(p),
        payee,
    );
}

/**
 * Expands the register row whose text contains `payee` by clicking it.
 *
 * Row lookup runs while all rows are collapsed, so `[role="button"]` inside the
 * register matches only the transaction rows (the expanded detail's own
 * `role="button"` controls would otherwise inflate the set).
 */
async function expandRowByPayee(payee: string): Promise<void> {
    const rowIndex: number = await browser.execute((p: string) => {
        const rows = Array.from(
            document.querySelectorAll('[aria-label="transaction register"] [role="button"]'),
        );
        return rows.findIndex(r => r.textContent?.includes(p));
    }, payee);
    expect(rowIndex).not.toBe(-1);

    const rows = await $$('[aria-label="transaction register"] [role="button"]');
    await rows[rowIndex].click();
}

/**
 * Reads the `data-dimmed` attribute of every posting row currently rendered in
 * an expanded transaction detail (`"true"` / `"false"`).
 */
async function expandedPostingDimStates(): Promise<string[]> {
    return browser.execute(
        () => Array.from(document.querySelectorAll('[data-testid="posting-row"]'))
            .map(el => el.getAttribute('data-dimmed') ?? ''),
    );
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

describe('Accounts register — global filter', () => {
    it('intersects the register with the active filter and dims non-matching legs', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await clearAllChips();

        // 1-2. Select CreditCard (a heavily-used account) and capture the
        // unfiltered baseline row count.
        await openAccount('CreditCard');
        await waitForRegisterRows();
        const baseline = await registerRowCount();

        // 3. Commit `tag:recurring` (Netflix recurs across every seeded
        // month on CreditCard) — assert the chip renders and the row count
        // drops below baseline, proving the register intersects with the
        // filter's non-account dimensions.
        await commitTagToken('recurring');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('tag: recurring');

        await browser.waitUntil(
            async () => (await registerRowCount()) < baseline,
            { timeoutMsg: 'Register row count did not drop after applying tag:recurring' },
        );
        const filteredCount = await registerRowCount();
        expect(filteredCount).toBeLessThan(baseline);

        // Round-trip: removing the chip returns the register to baseline.
        await removeChip('tag: recurring');
        await browser.waitUntil(
            async () => (await registerRowCount()) === baseline,
            { timeoutMsg: 'Register row count did not return to baseline after removing the chip' },
        );

        // 4. Non-matching-leg dimming.
        //
        // The seed's one genuine partial-match case comes from a posting-level
        // tag. The seed creates a "The Local Bistro" dinner on the 8th of EVERY
        // historical month, but only the earliest one (6 months ago —
        // 2026-01-08 in this seed) carries the posting-level `reimbursable` tag
        // on its Dining leg via
        // `posting_tagged(&dining_id, aud(85.00), vec![tag_reimbursable])`
        // (`crates/bc-seed/src/main.rs`); its CreditCard leg is untagged, and
        // the later monthly bistros carry no posting tags at all. Filtering by
        // `tag:reimbursable` therefore isolates that single transaction and
        // matches the Dining posting but not the CreditCard posting — a real
        // partial leg match. Expanding the row must render the matched Dining
        // leg lit and the unmatched CreditCard leg dimmed.
        //
        // Apply the filter FIRST, then step back: with `reimbursable` active
        // only the one January transaction survives, so stepping the shared
        // period window back lands exactly on it (the other monthly bistros
        // are filtered out) rather than on an untagged later-month bistro.
        await openAccount('Dining');
        await waitForRegisterRows();

        await commitTagToken('reimbursable');
        const chips2 = await $('[data-testid="filter-chips"]');
        await expect(chips2).toBeDisplayed();
        expect(await chips2.getText()).toContain('tag: reimbursable');

        // `reimbursable` sets no date bounds, so the register's PeriodNav stays
        // enabled — step back until the sole matching (January) bistro appears.
        for (let i = 0; i < 10 && !(await registerContainsPayee('The Local Bistro')); i += 1) {
            await stepToPreviousPeriod();
        }
        await browser.waitUntil(
            async () => registerContainsPayee('The Local Bistro'),
            { timeoutMsg: 'The reimbursable "The Local Bistro" row never came into view' },
        );

        // Expand the partial-match row: the detail lists all legs, with the
        // unmatched CreditCard leg dimmed and the matched Dining leg lit.
        await expandRowByPayee('The Local Bistro');
        await browser.waitUntil(
            async () => (await expandedPostingDimStates()).length > 0,
            { timeoutMsg: 'Expanded transaction detail rendered no posting rows' },
        );
        const dimStates = await expandedPostingDimStates();
        expect(dimStates).toContain('true');
        expect(dimStates).toContain('false');

        // 5. Remove the remaining chip — the filter deactivates and the chip
        // strip disappears.
        await removeChip('tag: reimbursable');
        await expect($('[data-testid="filter-chips"]')).not.toBeDisplayed();
    });
});
