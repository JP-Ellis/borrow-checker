/**
 * Flow test proving the accounts-page transaction register consumes the
 * app-wide global filter (rather than the removed local all/pending/
 * uncategorised bar): selecting an account shows its transactions
 * intersected with the active filter's non-account dimensions, and the
 * lenient/strict toggle changes how a partially-matching transaction's legs
 * render.
 *
 * Reuses the ⌘K palette harness (imports, dialog/listbox selectors, token
 * commit flow) from `palette-filter-builder.spec.ts` verbatim.
 */
import { browser, $, $$, expect } from '@wdio/globals';

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
 * Reads the Category column cell's `textContent` for the row whose text
 * contains `payee`. Row children are always [date, payee cell, tags cell,
 * category cell, amount, chevron] in DOM order regardless of Stylance's
 * hashed class names, so the category cell is reliably the 4th child.
 *
 * Uses `getAttribute('textContent')` rather than `getText()` — WebKitWebDriver
 * returns `""` from `getText()` on these register rows (see the project note
 * in `palette-filter-builder.spec.ts`).
 */
async function categoryCellTextForPayee(payee: string): Promise<string> {
    const rowIndex: number = await browser.execute((p: string) => {
        const rows = Array.from(
            document.querySelectorAll('[aria-label="transaction register"] [role="button"]'),
        );
        return rows.findIndex(r => r.textContent?.includes(p));
    }, payee);
    if (rowIndex === -1) return '';

    const rows = await $$('[aria-label="transaction register"] [role="button"]');
    const categoryCell = await rows[rowIndex].$('*:nth-child(4)');
    return (await categoryCell.getAttribute('textContent')) ?? '';
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
 * must be dismissed before any other top-bar interaction (chip removal,
 * strictness toggle).
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

// ── Filter chip / strictness helpers ─────────────────────────────────────────

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
    it('intersects the register with the active filter and toggles strictness', async () => {
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
        // drops below baseline.
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

        // 4. The strictness toggle is hidden with no active filter; it must
        // now be visible, defaulting to lenient.
        const toggle = await $('[data-testid="strictness-toggle"]');
        await expect(toggle).toBeDisplayed();
        await expect(toggle).toHaveAttribute('aria-pressed', 'false');

        // Round-trip: removing the chip returns the register to baseline and
        // hides the toggle again.
        await removeChip('tag: recurring');
        await browser.waitUntil(
            async () => (await registerRowCount()) === baseline,
            { timeoutMsg: 'Register row count did not return to baseline after removing the chip' },
        );
        await expect(toggle).not.toBeDisplayed();

        // 5. Strict-mode leg rendering.
        //
        // The brief's `over:100` scenario requires a transaction with a
        // genuine PARTIAL leg match — some postings meeting the amount bound,
        // some not. Every seeded transaction is a plain two-posting
        // debit/credit pair with equal-and-opposite magnitudes
        // (`crates/bc-seed/src/main.rs`), and `over`/`under` match on
        // `amount.abs()` per posting (`bc-core/src/search.rs`), so an
        // amount bound always matches both legs or neither — never a partial
        // match — for every seeded transaction. No `over:`/`under:` value
        // can demonstrate strict-mode leg hiding with this seed.
        //
        // The seed's ONE genuine partial-match case instead comes from a
        // posting-level tag. The seed creates a "The Local Bistro" dinner on
        // the 8th of EVERY historical month, but only the earliest one
        // (6 months ago — 2026-01-08 in this seed) carries the posting-level
        // `reimbursable` tag on its Dining leg via
        // `posting_tagged(&dining_id, aud(85.00), vec![tag_reimbursable])`
        // (`crates/bc-seed/src/main.rs`); its CreditCard leg is untagged, and
        // the later monthly bistros carry no posting tags at all. Filtering by
        // `tag:reimbursable` therefore isolates that single transaction and
        // matches the Dining posting but not the CreditCard posting — a real
        // partial leg match — so this substitutes for `over:100` while keeping
        // the strict-mode assertion real rather than dropping it.
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

        const toggle2 = await $('[data-testid="strictness-toggle"]');
        await expect(toggle2).toBeDisplayed();
        await expect(toggle2).toHaveAttribute('aria-pressed', 'false');

        // `reimbursable` sets no date bounds, so the register's PeriodNav stays
        // enabled — step back until the sole matching (January) bistro appears.
        for (let i = 0; i < 10 && !(await registerContainsPayee('The Local Bistro')); i += 1) {
            await stepToPreviousPeriod();
        }
        await browser.waitUntil(
            async () => registerContainsPayee('The Local Bistro'),
            { timeoutMsg: 'The reimbursable "The Local Bistro" row never came into view' },
        );

        // Lenient: the CreditCard counterpart leg still renders even though
        // it didn't match the tag filter itself.
        const lenientCategory = await categoryCellTextForPayee('The Local Bistro');
        expect(lenientCategory).toContain('CreditCard');

        // Strict: the unmatched CreditCard leg is hidden, so the Dining row
        // has no counterpart left and the category cell collapses to "—".
        await toggle2.click();
        await expect(toggle2).toHaveAttribute('aria-pressed', 'true');
        // The row re-renders on the strictness change — wait for the collapsed
        // "—" label (the CreditCard counterpart dropping out) before asserting.
        await browser.waitUntil(
            async () => (await categoryCellTextForPayee('The Local Bistro')).includes('—'),
            { timeoutMsg: 'Category cell did not collapse to "—" after switching to strict' },
        );
        const strictCategory = await categoryCellTextForPayee('The Local Bistro');
        expect(strictCategory).not.toContain('CreditCard');
        expect(strictCategory).toContain('—');

        // 6. Remove the remaining chip — the filter deactivates and the
        // strictness toggle disappears again.
        await removeChip('tag: reimbursable');
        await expect($('[data-testid="filter-chips"]')).not.toBeDisplayed();
        await expect($('[data-testid="strictness-toggle"]')).not.toBeDisplayed();
    });
});
