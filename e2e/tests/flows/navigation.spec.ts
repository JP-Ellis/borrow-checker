import { browser, $, expect } from '@wdio/globals';

const ROUTES = [
    { name: 'dashboard' as const, path: '/'         },
    { name: 'accounts'  as const, path: '/accounts' },
    { name: 'budget'    as const, path: '/budget'   },
    { name: 'reports'   as const, path: '/reports'  },
    { name: 'plugins'   as const, path: '/plugins'  },
    { name: 'settings'  as const, path: '/settings' },
];

describe('Shell navigation', () => {
    it('header and main area are visible on every route', async () => {
        for (const { name, path } of ROUTES) {
            const nav  = await $('nav[aria-label="main navigation"]');
            const link = await nav.$(`a=${name}`);
            await link.click();

            if (path !== '/') {
                await browser.waitUntil(
                    async () => (await browser.getUrl()).includes(path),
                    { timeout: 5_000, timeoutMsg: `URL did not reach ${path} within 5 s` },
                );
            }

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

        await browser.waitUntil(
            async () => (await browser.getUrl()).includes('/accounts'),
            { timeout: 5_000, timeoutMsg: 'URL did not reach /accounts within 5 s' },
        );

        await expect(await $('main')).toBeDisplayed();
    });

    it('every route renders without showing the fallback page', async () => {
        for (const { name, path } of ROUTES) {
            const nav  = await $('nav[aria-label="main navigation"]');
            const link = await nav.$(`a=${name}`);
            await link.click();

            if (path !== '/') {
                await browser.waitUntil(
                    async () => (await browser.getUrl()).includes(path),
                    { timeout: 5_000 },
                );
            }

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
    });
});
