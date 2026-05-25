import { browser, $, expect } from '@wdio/globals';

const ROUTES = [
    { name: 'dashboard' as const, path: '/'         },
    { name: 'accounts'  as const, path: '/accounts' },
    { name: 'budget'    as const, path: '/budget'   },
    { name: 'reports'   as const, path: '/reports'  },
    { name: 'plugins'   as const, path: '/plugins'  },
    { name: 'settings'  as const, path: '/settings' },
];

/**
 * Wait until the WebView's `window.location.pathname` matches `path`.
 *
 * Using `window.location` rather than `browser.getUrl()` sidesteps the
 * Tauri/WRY custom-protocol prefix (`tauri://localhost/…`) and gives us
 * exactly the path that the Leptos router reads.
 */
async function waitForPath(path: string, timeout = 5_000): Promise<void> {
    await browser.waitUntil(
        () => browser.execute(
            (expected: string) => window.location.pathname === expected,
            path,
        ),
        { timeout, timeoutMsg: `Pathname did not reach ${path} within ${timeout} ms` },
    );
}

describe('Shell navigation', () => {
    it('header and main area are visible on every route', async () => {
        for (const { name, path } of ROUTES) {
            const nav  = await $('nav[aria-label="main navigation"]');
            const link = await nav.$(`a=${name}`);
            await link.click();

            await waitForPath(path);

            await expect(await $('header')).toBeDisplayed();
            await expect(await $('main')).toBeDisplayed();
        }
    });

    it('nav contains all six named route links', async () => {
        const nav = await $('nav[aria-label="main navigation"]');
        await expect(nav).toBeDisplayed();

        for (const { name } of ROUTES) {
            await expect(await nav.$(`a=${name}`)).toBeDisplayed();
        }
    });

    it('clicking a nav link updates the URL', async () => {
        const nav          = await $('nav[aria-label="main navigation"]');
        const accountsLink = await nav.$('a=accounts');
        await accountsLink.click();

        await waitForPath('/accounts');

        await expect(await $('main')).toBeDisplayed();
    });

    it('every route renders without showing the fallback page', async () => {
        for (const { name, path } of ROUTES) {
            const nav  = await $('nav[aria-label="main navigation"]');
            const link = await nav.$(`a=${name}`);
            await link.click();

            await waitForPath(path);

            await expect(await $('main')).toBeDisplayed();
            const bodyText = (await (await $('body')).getText()).toLowerCase();
            expect(bodyText).not.toContain('page not found');
        }
    });

    it('unknown route shows "page not found" fallback', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/this-does-not-exist');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });

        /* Allow the Leptos router one tick to process the popstate event. */
        await browser.pause(300);

        const bodyText = (await (await $('body')).getText()).toLowerCase();
        expect(bodyText).toContain('page not found');

        /* Restore to a known-good route so later specs are not order-dependent. */
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await waitForPath('/');
    });
});
