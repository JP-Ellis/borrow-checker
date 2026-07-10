import { browser, $, expect } from '@wdio/globals';

describe('Command palette filter builder', () => {
    it('searches a seeded tag and commits it as a filter chip', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });

        const openButton = await $('button[aria-label="open command palette (⌘K)"]');
        await openButton.click();

        const dialog = await $('div[role="dialog"][aria-label="Command palette"]');
        await expect(dialog).toBeDisplayed();

        const input = await dialog.$('input[role="combobox"]');
        await input.waitForDisplayed();

        /* The `tag:` prefix jumps to the tag picker, which recreates the input, so
         * re-select it before narrowing to a specific seeded tag. */
        await input.setValue('tag:');
        const tagInput = await dialog.$('input[aria-label="Tag"]');
        await tagInput.waitForDisplayed();

        /* Unfiltered, the seeded tag taxonomy yields several options. */
        const listbox = await $('#palette-listbox');
        await browser.waitUntil(
            async () => (await listbox.$$('div[role="option"]').length) > 1,
            { timeoutMsg: 'expected multiple seeded tags in the picker' },
        );

        /* Narrowing to `recurring` (a seeded tag) leaves it as the sole match. */
        await tagInput.setValue('recurring');
        await browser.waitUntil(
            async () => (await listbox.$$('div[role="option"]').length) === 1,
            { timeoutMsg: 'expected the tag search to narrow to `recurring`' },
        );

        const only = await listbox.$('div[role="option"]');
        /* getText() is unreliable for these option rows under WebKitWebDriver, so
         * read the DOM text directly. */
        expect(await only.getAttribute('textContent')).toContain('recurring');
        await only.click();

        /* Committing a value returns to the root dimension menu, closing the tag list. */
        await expect(await dialog.$('input[aria-label="Search filters"]')).toBeDisplayed();

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        const chipsText = await chips.getText();
        expect(chipsText).toContain('tag:');

        /* Escape from the root screen closes the palette. */
        await browser.keys('Escape');
        await expect(dialog).not.toBeDisplayed();
    });
});
