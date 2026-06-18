/**
 * Visual regression tests for the Accounts shell.
 *
 * Covers four variants: {light, dark} × {wide (1 440 px), narrow (375 px)}.
 * Each variant navigates to the "Checking" account (a seed account with
 * historical transactions) so the register shows real data.  Visual specs
 * run before flow tests (which mutate the DB) — see the `specs` ordering in
 * wdio.conf.ts.
 */
import { browser, $ } from '@wdio/globals';

const VARIANTS = [
    { scheme: 'light', width: 1440, height: 900 },
    { scheme: 'dark',  width: 1440, height: 900 },
    { scheme: 'light', width:  375, height: 812 },
    { scheme: 'dark',  width:  375, height: 812 },
] as const;

describe('Visual — accounts shell', () => {
    for (const { scheme, width, height } of VARIANTS) {
        const tag = `${scheme}-${width}`;
        const isNarrow = width < 768;

        describe(`${scheme} / ${width}px`, () => {
            before(async () => {
                await browser.setWindowSize(width, height);

                /* Force colour scheme via the data-theme attribute so the test
                 * is independent of the host OS / container GTK theme setting. */
                await browser.execute((s: string) => {
                    document.documentElement.setAttribute('data-theme', s);
                }, scheme);

                /* Navigate to the accounts page. */
                const navAccounts = await $('[data-testid="nav-accounts"]');
                await navAccounts.waitForDisplayed({ timeout: 10_000 });
                await navAccounts.click();

                /* Guard on the URL first — the sidebar nav is part of the global
                 * layout and its elements are in the DOM even on other routes. */
                await browser.waitUntil(
                    async () => (await browser.getUrl()).includes('/accounts'),
                    { timeout: 10_000, timeoutMsg: 'URL did not reach /accounts within 10 s' },
                );

                if (!isNarrow) {
                    /* Wide viewport — full sidebar is always visible.
                     * Wait for a known seed account name (not just any <a>) to
                     * confirm list_accounts IPC has completed, then select Checking. */
                    const sidebarNav = await $('nav[aria-label="account navigation"]');
                    await sidebarNav.$('span=Assets').waitForDisplayed({ timeout: 15_000 });
                    await sidebarNav.$('span=Checking').click();
                } else {
                    /* Narrow viewport — sidebar collapses to a dot-rail trigger.
                     * Wait for accounts to populate the drawer before opening it:
                     * the drawer is rendered inside a reactive branch that re-mounts
                     * when the list_accounts IPC resolves, which would dismiss an
                     * already-open popover and cause waitForDisplayed to time out. */
                    const trigger = await $('[aria-label="Open account navigation"]');
                    await trigger.waitForDisplayed({ timeout: 15_000 });

                    const drawer = await $('#bc-sidebar-drawer');
                    await drawer.$('span=Assets').waitForExist({ timeout: 15_000 });

                    /* Re-fetch trigger after IPC settle: the reactive re-mount that
                     * fires when list_accounts resolves may replace the element in
                     * the DOM, making the earlier reference stale and causing the
                     * click to be silently swallowed. */
                    await (await $('[aria-label="Open account navigation"]')).click();
                    await drawer.waitForDisplayed({ timeout: 10_000 });
                    const checkingInDrawer = await drawer.$('span=Checking');
                    await checkingInDrawer.waitForDisplayed({ timeout: 10_000 });
                    await checkingInDrawer.click();
                }

                /* Confirm navigation reached the selected account route. */
                await browser.waitUntil(
                    async () => (await browser.getUrl()).includes('/accounts/'),
                    { timeout: 10_000, timeoutMsg: 'URL did not update to account route' },
                );

                /* Wait for the transaction register to appear and for at least
                 * one transaction row to render (confirms list_transactions IPC
                 * completed and the register has real data). */
                const register = await $('[aria-label="transaction register"]');
                await register.waitForDisplayed({ timeout: 15_000 });
                await browser.waitUntil(
                    async () => (await register.$$('[role="button"]')).length > 0,
                    {
                        timeout: 15_000,
                        timeoutMsg: 'No transaction rows appeared in the register',
                    },
                );
            });

            it(`accounts shell matches baseline [${tag}]`, async () => {
                /* Allow up to 1 % pixel difference to absorb minor rendering
                 * variation across WebKit patch releases between image rebuilds. */
                const mismatch = await browser.checkScreen(`accounts-shell-${tag}`);
                expect(mismatch).toBeLessThanOrEqual(1);
            });

            it(`transaction rows have aria-expanded="false" by default [${tag}]`, async () => {
                const register = await $('[aria-label="transaction register"]');
                const rows     = await register.$$('[role="button"][aria-expanded]');
                expect(rows.length).toBeGreaterThan(0);
                await expect(rows[0]).toHaveAttribute('aria-expanded', 'false');
            });
        });
    }

    after(async () => {
        /* Restore defaults so subsequent flow tests start from a clean state. */
        await browser.setWindowSize(1440, 900);
        await browser.execute(() => {
            document.documentElement.removeAttribute('data-theme');
        });
    });
});
