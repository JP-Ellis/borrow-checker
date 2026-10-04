import { browser, $, expect } from '@wdio/globals';
import { commitStatusToken }  from '../support/palette.js';

describe('Command palette status filter', () => {
    it('commits balance status as its own chip, replacing an earlier pick', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });

        await commitStatusToken('unreconciled');
        await commitStatusToken('unbalanced');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('status:unreconciled');
        expect(await chips.getText()).toContain('status:unbalanced');

        /* `balanced` is a substring of `unbalanced`; the exact label wins and replaces it. */
        await commitStatusToken('balanced');
        const text = await $('[data-testid="filter-chips"]').getText();
        expect(text).toContain('status:balanced');
        expect(text).not.toContain('status:unbalanced');
        expect(text).toContain('status:unreconciled');
    });
});
