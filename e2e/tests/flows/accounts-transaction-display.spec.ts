/**
 * Flow tests asserting that the transaction register displays human-readable
 * account names rather than raw database IDs.
 *
 * The seed database populates the Checking account with transactions that have
 * counterpart postings from named accounts (Salary, Savings, CreditCard, …).
 * Before the fix, each counterpart showed its raw prefixed UUID; after the fix,
 * it shows the hierarchical path resolved at the bc-app layer.
 */
import { browser, $ } from '@wdio/globals';

// ── Helpers ────────────────────────────────────────────────────────────────

async function openCheckingAccount(): Promise<void> {
    const navAccounts = $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();

    const checkingSpan = sidebarNav.$('span=Checking');
    await checkingSpan.waitForDisplayed();
    await checkingSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Accounts — transaction register display', () => {
    it('shows human-readable account names instead of raw IDs in the register', async () => {
        await openCheckingAccount();

        const register = $('[aria-label="transaction register"]');
        await register.waitForDisplayed();

        // Wait for at least one transaction row to appear (confirms IPC round-trip).
        await browser.waitUntil(
            () => browser.execute(
                () => (document.querySelector('[aria-label="transaction register"]')
                    ?.querySelectorAll('[role="button"]').length ?? 0) > 0,
            ),
            { timeoutMsg: 'No transaction rows appeared in the register' },
        );

        // Use textContent (not innerText/getText) to capture all DOM text including
        // content that may be CSS-clipped by the grid layout in register rows.
        const registerText: string = await browser.execute(
            () => document.querySelector('[aria-label="transaction register"]')?.textContent ?? '',
        );

        // The envelope column must not expose raw account IDs.
        // Account IDs in this app are prefixed: "account_<ulid>".
        expect(registerText).not.toContain('account_');

        // Hierarchical paths should appear — the " :: " separator confirms that
        // build_account_path walked up the parent chain, not just returned a leaf name.
        expect(registerText).toContain(' :: ');

        // The seed data has Checking ↔ Salary, Savings, and CreditCard transfers.
        // After the fix, the full hierarchical path must be visible.
        const knownPaths = [
            'Income :: Salary',
            'Assets :: Savings',
            'Liabilities :: CreditCard',
        ];
        const hasPath = knownPaths.some(path => registerText.includes(path));
        if (!hasPath) {
            // Emit the first 500 chars so the CI output tells us what IS shown.
            throw new Error(
                `Expected one of [${knownPaths.join(', ')}] in the register.\n` +
                `Actual register text (first 500 chars): ${registerText.substring(0, 500)}`,
            );
        }
    });
});
