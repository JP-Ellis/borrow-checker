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
