import { browser, $, expect } from '@wdio/globals';

describe('Command palette filter builder', () => {
    it('searches a seeded tag inline and commits it as a named chip', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });

        const openButton = await $('button[aria-label="open command palette (⌘K)"]');
        await openButton.click();

        const dialog = await $('div[role="dialog"][aria-label="Command palette"]');
        await expect(dialog).toBeDisplayed();

        /* The palette is a single inline search box; the input is never recreated,
         * so a whole `tag:recurring` token can be typed in one go. */
        const input = await dialog.$('input[role="combobox"]');
        await input.waitForDisplayed();
        await input.setValue('tag:recurring');

        /* `recurring` is a seeded tag (bc-seed tag taxonomy); the token narrows the
         * live suggestions to it as the sole match. */
        const listbox = await $('#palette-listbox');
        await browser.waitUntil(
            async () => (await listbox.$$('div[role="option"]').length) === 1,
            { timeoutMsg: 'expected the tag search to narrow to `recurring`' },
        );

        const only = await listbox.$('div[role="option"]');
        /* getText() is unreliable for these option rows under WebKitWebDriver, so
         * read the DOM text directly. */
        expect(await only.getAttribute('textContent')).toContain('recurring');
        await only.click();

        /* Committing clears the box (ready for the next token) and adds a named chip.
         * The clear is driven by a reactive update, so wait for it rather than
         * asserting immediately (which would race the render). */
        await browser.waitUntil(async () => (await input.getValue()) === '', {
            timeoutMsg: 'expected the input to clear after committing the token',
        });

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        const chipsText = await chips.getText();
        expect(chipsText).toContain('tag: recurring');

        /* Escape closes the palette. */
        await browser.keys('Escape');
        await expect(dialog).not.toBeDisplayed();
    });
});
