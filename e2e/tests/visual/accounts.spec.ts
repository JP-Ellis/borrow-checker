/**
 * Visual regression tests for the Accounts shell.
 * Covers four variants: {light, dark} × {1440 px, 375 px}.
 * Visual specs run before flow tests (which mutate the DB).
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

                /* Force colour scheme via data-theme, independent of host GTK setting. */
                await browser.execute((s: string) => {
                    document.documentElement.setAttribute('data-theme', s);
                }, scheme);

                const navAccounts = await $('[data-testid="nav-accounts"]');
                await navAccounts.waitForDisplayed({ timeout: 10_000 });
                await navAccounts.click();

                /* Guard on URL — the sidebar nav is in the DOM on all routes. */
                await browser.waitUntil(
                    async () => (await browser.getUrl()).includes('/accounts'),
                    { timeout: 10_000, timeoutMsg: 'URL did not reach /accounts within 10 s' },
                );

                if (!isNarrow) {
                    const sidebarNav = await $('nav[aria-label="account navigation"]');
                    await sidebarNav.$('span=Assets').waitForDisplayed({ timeout: 15_000 });
                    await sidebarNav.$('span=Checking').click();
                } else {
                    /* Wait for IPC to settle before opening the drawer: the reactive
                     * re-mount that fires when list_accounts resolves would dismiss
                     * an already-open popover and cause waitForDisplayed to time out. */
                    const trigger = await $('[aria-label="Open account navigation"]');
                    await trigger.waitForDisplayed({ timeout: 15_000 });

                    const drawer = await $('#bc-sidebar-drawer');
                    await drawer.$('span=Assets').waitForExist({ timeout: 15_000 });

                    /* Re-fetch trigger: IPC re-mount may have replaced the element. */
                    await (await $('[aria-label="Open account navigation"]')).click();
                    await drawer.waitForDisplayed({ timeout: 10_000 });
                    const checkingInDrawer = await drawer.$('span=Checking');
                    await checkingInDrawer.waitForDisplayed({ timeout: 10_000 });
                    await checkingInDrawer.click();
                }

                await browser.waitUntil(
                    async () => (await browser.getUrl()).includes('/accounts/'),
                    { timeout: 10_000, timeoutMsg: 'URL did not update to account route' },
                );

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
        await browser.setWindowSize(1440, 900);
        await browser.execute(() => {
            document.documentElement.removeAttribute('data-theme');
        });
    });
});
