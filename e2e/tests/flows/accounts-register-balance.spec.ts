/**
 * The register's running-balance column: real balance by default, a mode
 * toggle that cycles real → sum → off and persists per browser, and a column
 * that hides its values in `off`.
 */
import { browser, $, $$ } from '@wdio/globals';

// ── Navigation helpers (copied from accounts-period-view.spec.ts) ──────────

/**
 * Navigate to Accounts → `name` via the top-bar nav and sidebar. Always goes
 * through the bare `/accounts` route first, which unmounts/remounts the
 * Accounts page component — so every call starts from a freshly mounted
 * page, open in all time.
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

// ── Balance-column helpers ───────────────────────────────────────────────

/**
 * Text of the sticky bar's balance, scrolling the register's scroll
 * container so the bar is visible. In all-time (no filter), `AccountStats`
 * carries no `real_closing`, so `closing_balance` reads the actual balance
 * at the window end — the same figure the newest register row's balance
 * cell shows.
 */
async function stickyBalance(): Promise<string> {
    await browser.execute(() => {
        document.querySelector('[data-testid="accounts-main-scroll"]')?.scrollTo({ top: 400 });
    });
    const el = await $('[data-testid="sticky-balance"]');
    await el.waitForDisplayed();
    return (await el.getText()).trim();
}

/**
 * aria-label of the first (newest) row's balance cell, e.g. "balance A$ 1,234.00 AUD".
 * The formatter joins symbol and number with U+00A0; `getAttribute` keeps it
 * while `getText` (used for the sticky bar) folds it to a space, so it is
 * normalised here to let the two compare.
 */
async function firstRowBalanceLabel(): Promise<string> {
    const cells = await $$('[data-testid="balance-cell"]');
    return ((await cells[0].getAttribute('aria-label')) ?? '').replace(/\u00a0/g, ' ');
}

describe('Accounts — register balance column', () => {
    it('shows the real balance on the newest row', async () => {
        await openAccount('Checking');
        await waitForRegisterRows();
        const sticky = await stickyBalance();
        const label = await firstRowBalanceLabel();
        expect(label.startsWith('balance ')).toBe(true);
        expect(label).toContain(sticky);
    });

    it('cycles the mode and persists it across a reload', async () => {
        await openAccount('Checking');
        await waitForRegisterRows();
        const toggle = await $('[data-testid="balance-mode"]');
        await toggle.waitForDisplayed();
        expect(await toggle.getText()).toBe('balance: real');
        await toggle.click();
        expect(await toggle.getText()).toBe('balance: sum');
        await toggle.click();
        expect(await toggle.getText()).toBe('balance: off');

        const cells = await $$('[data-testid="balance-cell"]');
        expect(await cells[0].getAttribute('data-empty')).toBe('true');

        // A full reload re-mounts the whole app (not just the Accounts page),
        // so the persisted mode must come back from localStorage rather than
        // from any in-memory page state.
        await browser.refresh();
        await openAccount('Checking');
        await waitForRegisterRows();
        const toggleAfterReload = await $('[data-testid="balance-mode"]');
        await toggleAfterReload.waitForDisplayed();
        expect(await toggleAfterReload.getText()).toBe('balance: off');
        await toggleAfterReload.click(); // back to real for later tests/specs
    });

    it('has no sentinel when everything fits in one page', async () => {
        await openAccount('Checking');
        await waitForRegisterRows();
        expect(await $('[data-testid="register-sentinel"]').isExisting()).toBe(false);
        expect(await $('[data-testid="load-more"]').isExisting()).toBe(false);
    });
});
