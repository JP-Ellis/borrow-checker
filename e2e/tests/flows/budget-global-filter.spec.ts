/**
 * Flow test proving the budget page consumes the app-wide global filter:
 * applying a filter dimension (free payee/narration text) recomputes a known
 * budget row's actual spend, and clearing it reverts to the unfiltered
 * baseline. A date bound is inert for budgets (they are period-gridded via
 * `PeriodNav`, not the filter's date range) and instead surfaces a hint.
 *
 * Reuses the ⌘K palette harness (imports, dialog/listbox selectors, token
 * commit flow, chip helpers) from `palette-filter-builder.spec.ts` /
 * `register-global-filter.spec.ts`, and the budget navigation helpers from
 * `budget.spec.ts`.
 *
 * Seed data (`crates/bc-seed/src/main.rs`): every historical month has three
 * Groceries transactions — "Woolworths" $140.00, "Coles" $110.00, "IGA"
 * $55.00 — totalling $305.00. Stepping the budget page's `PeriodNav` back one
 * month from the current month (which has only a single, non-representative
 * Groceries transaction) lands on such a month. Filtering free text
 * "Woolworths" isolates the $140.00 leg — a proper subset, not zero — proving
 * the budget recomputes with the filter applied.
 */
import { browser, $, expect } from '@wdio/globals';

// ── Navigation helpers (mirrors budget.spec.ts) ─────────────────────────────

/** Navigate to /budget and wait for IPC data to populate the tree. */
async function navigateToBudget(): Promise<void> {
    const nav = await $('nav[aria-label="main navigation"]');
    await (await nav.$('a=budget')).click();

    await browser.waitUntil(
        () => browser.execute(() => window.location.pathname === '/budget'),
        { timeoutMsg: 'URL did not reach /budget within 5 s' },
    );

    const tree = await $('[aria-label="budget tree"]');
    await tree.waitForDisplayed();

    await browser.waitUntil(
        async () => (await tree.getText()).includes('Groceries'),
        { timeoutMsg: 'Budget tree did not populate within 15 s' },
    );
}

/** Extract the current "Month YYYY" period label from main content. */
async function getMonthLabel(): Promise<string | null> {
    const text = await (await $('main')).getText();
    const m = text.match(
        /(?:January|February|March|April|May|June|July|August|September|October|November|December) \d{4}/,
    );
    return m?.[0] ?? null;
}

/**
 * Snapshot of the budget tree's rendered content, used as a settle signal for
 * the async `overview` resource re-fetch (period step, or filter apply/clear).
 */
async function treeSignature(): Promise<string> {
    return browser.execute(
        () => document.querySelector('[aria-label="budget tree"]')?.textContent ?? '',
    );
}

/** Waits until the budget tree's rendered content stops changing across polls. */
async function waitForTreeSettled(): Promise<void> {
    let previous = await treeSignature();
    let stableReads = 0;
    await browser.waitUntil(
        async () => {
            const current = await treeSignature();
            stableReads = current === previous ? stableReads + 1 : 0;
            previous = current;
            return stableReads >= 2;
        },
        {
            interval: 150,
            timeout: 10000,
            timeoutMsg: 'Budget tree did not settle after a period/filter change',
        },
    );
}

/** Steps the budget page's period window back one step and waits for it to settle. */
async function stepToPreviousPeriod(): Promise<void> {
    const before = await getMonthLabel();
    await (await $('button=◀')).click();
    await browser.waitUntil(
        async () => (await getMonthLabel()) !== before,
        { timeoutMsg: 'Period label did not change after clicking "◀"' },
    );
    await waitForTreeSettled();
}

/**
 * Reads the "SPENT / TARGET" amounts text for the Groceries leaf row.
 *
 * The row's amounts span carries a Stylance-hashed class, so it cannot be
 * targeted by a stable selector. Instead, find the span whose text is exactly
 * the row name ("Groceries"), then read its sibling: for a leaf row the DOM
 * order is `<span name> <div bar_track> <span amounts>`, so the amounts span
 * is always the row's last element child.
 */
async function groceriesRowAmounts(): Promise<string> {
    return browser.execute(() => {
        const tree = document.querySelector('[aria-label="budget tree"]');
        if (!tree) return '';
        const nameSpan = Array.from(tree.querySelectorAll('span')).find(
            s => s.textContent?.trim() === 'Groceries',
        );
        const row = nameSpan?.parentElement;
        return row?.lastElementChild?.textContent ?? '';
    });
}

// ── Palette helpers (mirrors palette-filter-builder.spec.ts / register-global-filter.spec.ts) ──

/** Opens the ⌘K palette and returns the dialog element. */
async function openPalette() {
    const openButton = await $('button[aria-label="open command palette (⌘K)"]');
    await openButton.click();

    const dialog = await $('div[role="dialog"][aria-label="Command palette"]');
    await expect(dialog).toBeDisplayed();
    return dialog;
}

/**
 * Types free payee/narration `text` into the palette's inline search box and
 * commits it with Enter (free text has no listbox suggestions — the listbox
 * shows only a "↵ search payee/narration" hint). Closes the palette
 * afterwards — chips sit behind the z-900 palette overlay, so the overlay
 * must be dismissed before any other top-bar interaction (chip removal).
 */
async function commitTextToken(text: string): Promise<void> {
    const dialog = await openPalette();

    const input = await dialog.$('input[role="combobox"]');
    await input.waitForDisplayed();
    await input.setValue(text);
    await browser.keys('Enter');

    await browser.waitUntil(async () => (await input.getValue()) === '', {
        timeoutMsg: 'expected the input to clear after committing the token',
    });

    await browser.keys('Escape');
    await expect(dialog).not.toBeDisplayed();
}

/**
 * Types an `after:<date>` token into the palette and commits it with Enter.
 * Closes the palette afterwards, same rationale as `commitTextToken`.
 */
async function commitAfterToken(date: string): Promise<void> {
    const dialog = await openPalette();

    const input = await dialog.$('input[role="combobox"]');
    await input.waitForDisplayed();
    await input.setValue(`after:${date}`);
    await browser.keys('Enter');

    await browser.waitUntil(async () => (await input.getValue()) === '', {
        timeoutMsg: 'expected the input to clear after committing the token',
    });

    await browser.keys('Escape');
    await expect(dialog).not.toBeDisplayed();
}

// ── Filter chip helpers (mirrors register-global-filter.spec.ts) ───────────

/** Clicks a chip's ✕ by its exact label (e.g. `"text: Woolworths"`). */
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

describe('Budget — global filter', () => {
    it('recomputes a budget actual when a filter is applied and cleared', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await clearAllChips();

        // 1. Navigate to the budget page (defaults to the current month, which
        // has only a single non-representative Groceries transaction).
        await navigateToBudget();

        // Step back one month to a window with the seed's steady-state three
        // Groceries transactions (Woolworths $140.00 + Coles $110.00 + IGA
        // $55.00 = $305.00).
        await stepToPreviousPeriod();

        // 2. Read the baseline (unfiltered) actual for the Groceries row.
        const baseline = await groceriesRowAmounts();
        expect(baseline).toContain('305');

        // 3. Open the ⌘K palette and commit free text "Woolworths" — matches
        // only the Woolworths $140.00 leg, a proper subset of the month's
        // Groceries spend.
        await commitTextToken('Woolworths');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('text: Woolworths');

        // 4. Assert the row's actual dropped to the filtered subset total.
        await browser.waitUntil(
            async () => (await groceriesRowAmounts()) !== baseline,
            { timeoutMsg: 'Groceries actual did not change after applying the text filter' },
        );
        await waitForTreeSettled();
        const filtered = await groceriesRowAmounts();
        expect(filtered).toContain('140');
        expect(filtered).not.toContain('305');

        // 5. Remove the chip via the top-bar ✕ (palette already closed above).
        await removeChip('text: Woolworths');
        await expect($('[data-testid="filter-chips"]')).not.toBeDisplayed();

        // 6. Assert the actual reverts to the baseline.
        await browser.waitUntil(
            async () => (await groceriesRowAmounts()) === baseline,
            { timeoutMsg: 'Groceries actual did not revert to baseline after removing the chip' },
        );
    });

    it('shows the inert-date hint when a date bound is set', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await clearAllChips();

        // 1. Navigate to the budget page.
        await navigateToBudget();

        // The hint is absent while no date bound is active.
        await expect($('span*=Date filter doesn')).not.toBeDisplayed();

        // 2. Add an `after:` date chip via the palette.
        await commitAfterToken('2026-01-01');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('after: 2026-01-01');

        // 3. Assert the inert-date hint is visible with the exact copy.
        const hint = await $('span*=Date filter doesn');
        await hint.waitForDisplayed();
        expect(await hint.getAttribute('textContent')).toBe(
            'Date filter doesn’t apply to budgets — using the selected period.',
        );

        // Round-trip: removing the chip hides the hint again.
        await removeChip('after: 2026-01-01');
        await expect($('span*=Date filter doesn')).not.toBeDisplayed();
    });
});
