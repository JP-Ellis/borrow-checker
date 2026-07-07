import { browser, $, expect } from '@wdio/globals';

describe('Command palette filter builder', () => {
    it('typing tag: jumps to the tag picker and committing shows a chip', async () => {
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
        await input.setValue('tag:');

        /* The prefix jump replaces the combobox's aria-label with the dimension name. */
        await expect(await dialog.$('input[aria-label="Tag"]')).toBeDisplayed();

        const listbox = await $('#palette-listbox');
        const firstOption = await listbox.$('div[role="option"]');
        await firstOption.waitForDisplayed();
        await firstOption.click();

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
