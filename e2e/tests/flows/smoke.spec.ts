import { browser, $, expect } from '@wdio/globals';

describe('Smoke — app launches and shell renders', () => {
  it('window title is "borrow-checker"', async () => {
    const title = await browser.getTitle();
    expect(title).toBe('borrow-checker');
  });

  it('TopBar header is visible', async () => {
    const header = await $('header');
    await expect(header).toBeDisplayed();
  });

  it('main navigation is visible and contains at least 6 links', async () => {
    const nav = await $('nav[aria-label="main navigation"]');
    await expect(nav).toBeDisplayed();

    const links = await nav.$$('a');
    expect(links.length).toBeGreaterThanOrEqual(6);
  });

  it('main content area is visible', async () => {
    const main = await $('main');
    await expect(main).toBeDisplayed();
  });

  it('clicking the accounts link navigates to /accounts', async () => {
    const nav          = await $('nav[aria-label="main navigation"]');
    const accountsLink = await nav.$('a=accounts');
    await accountsLink.click();

    await browser.waitUntil(
      async () => (await browser.getUrl()).includes('/accounts'),
      { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    await expect(await $('main')).toBeDisplayed();
  });
});
