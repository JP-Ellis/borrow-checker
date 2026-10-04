import { browser, $, $$, expect } from '@wdio/globals';
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

        /* `balanced` is a substring of `unbalanced`. With the `unbalanced`
           chip still present, Enter must commit the exact label as a second,
           distinct chip rather than leaving `unbalanced` as the only match. */
        await commitStatusToken('balanced');
        const labels = await $$('[data-testid="filter-chips"] button[aria-label^="edit "]')
            .map((chip) => chip.getAttribute('aria-label'));
        expect(labels).toEqual([
            'edit status:unreconciled filter',
            'edit status:unbalanced filter',
            'edit status:balanced filter',
        ]);

        /* Removing `unbalanced` leaves the other two untouched. */
        const remove = await $('[data-testid="filter-chips"] button[aria-label="remove status:unbalanced filter"]');
        await remove.click();
        const text = await $('[data-testid="filter-chips"]').getText();
        expect(text).toContain('status:balanced');
        expect(text).not.toContain('status:unbalanced');
        expect(text).toContain('status:unreconciled');
    });
});
