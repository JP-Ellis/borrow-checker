import { browser } from '@wdio/globals';

/**
 * Returns the app to a clean start inside one spec file: no query, all time,
 * nothing to replay. The window and query live in the URL and carry across
 * tabs, so a spec that expects a fresh page must ask for one.
 */
export async function freshView(): Promise<void> {
    await browser.execute(() => {
        window.localStorage.removeItem('bc.last_location');
        window.history.replaceState(null, '', '/');
    });
    await browser.refresh();
    await browser.$('nav[aria-label="main navigation"]').waitForDisplayed();
}

/** The current URL's search parameters. */
export async function searchParams(): Promise<URLSearchParams> {
    return new URL(await browser.getUrl()).searchParams;
}

/** The current URL's path, without query or hash. */
async function pathname(): Promise<string> {
    return new URL(await browser.getUrl()).pathname;
}

/**
 * Opens Accounts → `name` through the top-bar nav and the sidebar. It goes by
 * way of the bare `/accounts` route, which remounts the Accounts page, so the
 * account opens in all time.
 *
 * The router writes the address bar only once the new route has rendered, so
 * a URL read straight after a click still shows the page being left. Each
 * step therefore waits for the exact path it expects. The last wait also
 * catches a click that a re-render steered onto a neighbouring row.
 */
export async function openAccount(name: string): Promise<void> {
    const nav = browser.$('[data-testid="nav-accounts"]');
    await nav.waitForDisplayed();
    await nav.click();
    await browser.waitUntil(
        async () => (await pathname()) === '/accounts',
        { timeoutMsg: 'URL did not reach /accounts' },
    );

    const account = browser.$('nav[aria-label="account navigation"]').$(`span=${name}`);
    await account.waitForDisplayed();
    const href = await account.parentElement().getAttribute('href');
    if (href === null) throw new Error(`Sidebar row for ${name} has no link`);
    const target = new URL(href, await browser.getUrl()).pathname;
    await account.click();
    await browser.waitUntil(
        async () => (await pathname()) === target,
        { timeoutMsg: `URL did not reach ${name} at ${target}` },
    );

    await browser.$('[aria-label="transaction register"]').waitForDisplayed();
}
