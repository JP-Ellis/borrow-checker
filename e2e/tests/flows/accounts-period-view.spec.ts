/**
 * Flow tests for the period-scoped accounts register and dashboard.
 *
 * The accounts page shares a single display window (`window_start` +
 * granularity) between the transaction register and the per-account
 * dashboard, driven by the `WindowNav` control rendered in the register
 * header (`aria-label="previous period"` / `"next period"` buttons, plus a
 * granularity `<select>` whose first option is "all time"). The page opens
 * in all time, where the step buttons are absent; selecting a period from
 * there lands on the period containing today.
 *
 * Seed data (see crates/bc-seed/src/main.rs) is generated relative to
 * "today" — Checking has transactions in every one of the last 6 months
 * plus the current month, so stepping the window always crosses a
 * transaction boundary. Transport, however, has transactions in every
 * historical month but NONE in the current month — a real seeded account
 * with no current-period activity.
 */
import { browser, $ } from '@wdio/globals';

// ── Navigation helpers ───────────────────────────────────────────────────────

/**
 * Navigate to Accounts → `name` via the top-bar nav and sidebar. Always goes
 * through the bare `/accounts` route first, which unmounts/remounts the
 * Accounts page component — so every test starts from a freshly seeded
 * window (the page seeds it once per mount from the ledger's latest
 * activity).
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
 * Reads a dashboard stat-card value by its eyebrow label (e.g. "income",
 * "expenses", "transactions"). `StatCard` renders `<span>{label}</span>`
 * immediately followed by `<span>{value}</span>` under the same parent, so
 * this walks sibling spans rather than relying on Stylance class names.
 */
async function statValue(label: string): Promise<string> {
    return browser.execute((lbl: string) => {
        const spans = Array.from(document.querySelectorAll('span'));
        const idx = spans.findIndex(s => s.textContent?.trim() === lbl);
        if (idx === -1 || !spans[idx + 1]) return '';
        return spans[idx + 1].textContent?.trim() ?? '';
    }, label);
}

/**
 * Reads the dashboard's closing-balance headline. It is rendered as
 * `<span>{balance}</span><span>"// closing"</span>` — the value precedes its
 * label, so this walks backwards from the `"// closing"` marker span.
 */
async function closingBalance(): Promise<string> {
    return browser.execute(() => {
        const spans = Array.from(document.querySelectorAll('span'));
        const idx = spans.findIndex(s => s.textContent?.trim() === '// closing');
        if (idx <= 0) return '';
        return spans[idx - 1].textContent?.trim() ?? '';
    });
}

/**
 * Reads the shared `WindowNav` window label — the `<span>` that is the first
 * child of the granularity select's parent `div`. In all time the step
 * buttons are absent, so the label can't be read by walking between them.
 */
async function periodNavLabel(): Promise<string> {
    return browser.execute(() => {
        const select = document.querySelector('[aria-label="transaction register"] select');
        const row = select?.parentElement;
        const label = row ? Array.from(row.children).find(el => el.tagName === 'SPAN') : null;
        return label?.textContent?.trim() ?? '';
    });
}

/**
 * Selects a granularity in the register header's `WindowNav`. The page opens
 * in all time, where the step buttons are absent, so every stepping test
 * enters a period first.
 */
async function selectGranularity(value: string): Promise<void> {
    const select = await $('[aria-label="transaction register"] select');
    await select.waitForDisplayed();
    await select.selectByAttribute('value', value);
    await browser.waitUntil(
        async () => (await periodNavLabel()) !== 'all time' && (await periodNavLabel()) !== '',
        { timeoutMsg: `Window label did not leave "all time" after selecting ${value}` },
    );
}

/** Full English month name + year, matching bc-ui's `window_label` format for `Period::Monthly`. */
function currentMonthLabel(): string {
    const names = [
        'January', 'February', 'March', 'April', 'May', 'June',
        'July', 'August', 'September', 'October', 'November', 'December',
    ];
    const now = new Date();
    return `${names[now.getMonth()]} ${now.getFullYear()}`;
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Accounts — period view', () => {
    it('steps the register and dashboard together when the period changes', async () => {
        await openAccount('Checking');
        await waitForRegisterRows();
        await selectGranularity('monthly');
        await browser.waitUntil(
            async () => (await statValue('transactions')) !== '',
            { timeoutMsg: 'Dashboard tx-count did not populate after selecting monthly' },
        );

        const initialRows = await registerRowCount();
        const initialTxCount = await statValue('transactions');
        const initialIncome = await statValue('income');
        const initialExpenses = await statValue('expenses');
        const initialClosing = await closingBalance();
        const initialLabel = await periodNavLabel();

        // The register row count should reflect the same window as the
        // dashboard's "transactions" stat (both derive from the same
        // shared window_start/period).
        expect(initialRows.toString()).toBe(initialTxCount);

        const prevBtn = await $('[aria-label="previous period"]');
        await prevBtn.waitForDisplayed();
        await prevBtn.click();

        // Wait for the shared window label to update — confirms the step
        // actually propagated before reading anything else.
        await browser.waitUntil(
            async () => (await periodNavLabel()) !== initialLabel,
            { timeoutMsg: 'Period label did not change after clicking "previous period"' },
        );

        // Give the async IPC re-fetches (register + stats) a moment to land.
        await browser.waitUntil(
            async () => (await statValue('transactions')) !== initialTxCount,
            { timeoutMsg: 'Dashboard tx-count did not change after stepping to the previous period' },
        );
        // The register and dashboard re-fetch independently; wait for them to
        // agree so a stale register row mid-re-render can't race the reads.
        await browser.waitUntil(
            async () => (await registerRowCount()).toString() === (await statValue('transactions')),
            { timeoutMsg: 'Register row count did not settle to match the dashboard after stepping' },
        );

        const steppedRows = await registerRowCount();
        const steppedTxCount = await statValue('transactions');
        const steppedIncome = await statValue('income');
        const steppedExpenses = await statValue('expenses');
        const steppedClosing = await closingBalance();

        expect(steppedRows.toString()).toBe(steppedTxCount);
        expect(steppedClosing).not.toBe(initialClosing);
        expect(steppedIncome !== initialIncome || steppedExpenses !== initialExpenses).toBe(true);

        // Step forward again — should land back on the original window.
        const nextBtn = await $('[aria-label="next period"]');
        await nextBtn.click();

        await browser.waitUntil(
            async () => (await periodNavLabel()) === initialLabel,
            { timeoutMsg: 'Period label did not return to the original window after stepping forward' },
        );
        await browser.waitUntil(
            async () => (await statValue('transactions')) === initialTxCount,
            { timeoutMsg: 'Dashboard tx-count did not return to its original value' },
        );
        // The register re-fetches independently of the dashboard stat, so wait
        // for its row count to settle back too before asserting — otherwise a
        // stale row lingering mid-re-render races the read below.
        await browser.waitUntil(
            async () => (await registerRowCount()).toString() === initialTxCount,
            { timeoutMsg: 'Register row count did not return to its original value' },
        );

        expect((await registerRowCount()).toString()).toBe(initialTxCount);
        expect(await closingBalance()).toBe(initialClosing);
    });

    it('re-scopes the register and dashboard when the granularity select changes', async () => {
        await openAccount('Checking');
        await waitForRegisterRows();
        await selectGranularity('monthly');

        const initialLabel = await periodNavLabel();
        const initialTxCount = await statValue('transactions');

        // The granularity select lives inside the register header; scope the
        // selector so we don't hit the dashboard's (unrelated) sparkline selects.
        const periodSelect = await $('[aria-label="transaction register"] select');
        await periodSelect.waitForDisplayed();
        await periodSelect.selectByAttribute('value', 'quarterly');

        await browser.waitUntil(
            async () => (await periodNavLabel()) !== initialLabel,
            { timeoutMsg: 'Period label did not change after switching to quarterly granularity' },
        );

        // Monthly labels look like "June 2026"; quarterly labels look like "Q2 2026".
        const quarterlyLabel = await periodNavLabel();
        expect(quarterlyLabel).toMatch(/^Q[1-4] \d{4}$/);

        // A calendar quarter spans (at least) the same days as the calendar
        // month it was snapped from, so the tx-count/register row count for
        // the quarter must be at least as large as the original month's.
        await browser.waitUntil(
            async () => {
                const tx = await statValue('transactions');
                return tx !== '' && tx !== '—';
            },
            { timeoutMsg: 'Dashboard tx-count did not refresh after the granularity change' },
        );
        const quarterlyTxCount = Number(await statValue('transactions'));
        const quarterlyRows = await registerRowCount();
        expect(quarterlyRows.toString()).toBe(quarterlyTxCount.toString());
        expect(quarterlyTxCount).toBeGreaterThanOrEqual(Number(initialTxCount));
    });

    it('opens in all time and shows every transaction', async () => {
        // Transport has no current-month activity; in all time its whole
        // history is on screen without any stepping.
        await openAccount('Transport');
        await waitForRegisterRows();

        expect(await periodNavLabel()).toBe('all time');
        expect(await $('[aria-label="previous period"]').isExisting()).toBe(false);

        await browser.waitUntil(
            async () => (await registerRowCount()).toString() === (await statValue('transactions')),
            { timeoutMsg: 'Register row count did not match the all-time transaction stat' },
        );
        expect(await registerRowCount()).toBeGreaterThan(0);

        // Leaving all time lands on the current month; Transport is empty there.
        await selectGranularity('monthly');
        expect(await periodNavLabel()).toBe(currentMonthLabel());
        await browser.waitUntil(
            async () => (await statValue('transactions')) === '0',
            { timeoutMsg: 'Dashboard tx-count did not settle on 0 for the current month' },
        );
    });
});
