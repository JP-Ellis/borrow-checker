import { browser, $, expect } from '@wdio/globals';
import { commitTagToken }     from '../support/palette.js';

describe('Command palette filter builder', () => {
    it('searches a seeded tag inline and commits it as a named chip', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });

        /* `recurring` is a seeded tag (bc-seed tag taxonomy); typing the token
         * narrows the live suggestions to it as the sole match. */
        await commitTagToken('recurring');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('tag: recurring');
    });
});
