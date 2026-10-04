import { browser, $, expect } from '@wdio/globals';
import { commitStatusToken }  from '../support/palette.js';

describe('Command palette status filter', () => {
    it('commits each status as its own chip, joined by and', async () => {
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

        /* `balanced` is a substring of `unbalanced`; Enter takes the exact label. */
        const remove = await $('[data-testid="filter-chips"] button[aria-label="remove status:unbalanced filter"]');
        await remove.click();
        await commitStatusToken('balanced');
        const text = await $('[data-testid="filter-chips"]').getText();
        expect(text).toContain('status:balanced');
        expect(text).not.toContain('status:unbalanced');
        expect(text).toContain('status:unreconciled');
    });
});
