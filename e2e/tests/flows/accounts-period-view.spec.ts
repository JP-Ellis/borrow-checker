/**
 * Flow tests for the period-scoped accounts register and dashboard.
 *
 * The accounts page now shares a single display window (`window_start` +
 * granularity) between the transaction register and the per-account
 * dashboard, driven by the `PeriodNav` stepper rendered in the register
 * header (`aria-label="previous period"` / `"next period"` buttons, plus a
 * granularity `<select>`).
 *
 * Seed data (see crates/bc-seed/src/main.rs) is generated relative to
 * "today" — Checking has transactions in every one of the last 6 months
 * plus the current month, so stepping the window always crosses a
 * transaction boundary. Transport, however, has transactions in every
 * historical month but NONE in the current month — a real seeded account
 * with no current-period activity — which is used to verify the
 * auto-jump-to-latest-activity behaviour on first load.
 */
import { browser, $ } from '@wdio/globals';

// ── Navigation helpers ───────────────────────────────────────────────────────

/**
 * Navigate to Accounts → `name` via the top-bar nav and sidebar. Always goes
 * through the bare `/accounts` route first, which unmounts/remounts the
 * Accounts page component — this is what makes the "first selection since
 * mount" auto-jump behaviour observable for whichever account is clicked.
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
 * Reads the shared `PeriodNav` window label — the text node rendered between
 * the "previous period" and "next period" buttons.
 */
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

    it('auto-jumps to the most recent active period on first load for an account with no current-period activity', async () => {
        // Transport has transactions in every historical seeded month but NONE
        // in the current month (see crates/bc-seed/src/main.rs) — the
        // default "today" window would show an empty register were it not
        // for the account_latest_activity auto-jump on first selection.
        await openAccount('Transport');

        await waitForRegisterRows();
        const rows = await registerRowCount();
        expect(rows).toBeGreaterThan(0);

        const label = await periodNavLabel();
        expect(label).not.toBe(currentMonthLabel());

        // Dashboard should reflect the same non-empty, jumped-to window.
        const txCount = await statValue('transactions');
        expect(txCount).not.toBe('0');
        expect(txCount).not.toBe('—');
        expect(rows.toString()).toBe(txCount);
    });
});
