import { browser, $, expect } from '@wdio/globals';
import {
    accountSuggestions,
    commitHighlightedAccount,
    commitTagToken,
} from '../support/palette.js';

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
        expect(await chips.getText()).toContain('tag:recurring');
    });

    it('lists accounts by full path and chips them by their shortest unique name', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });

        const car = await accountSuggestions('car');
        expect(car).toContain('Assets :: Car');
        expect(car).toContain('Liabilities :: CarLoan');
        /* The highlighted run preserves the seed path's case, not the
         * lowercase query. */
        const mark = await $('#palette-listbox mark');
        expect(await mark.getText()).toBe('Car');
        await browser.keys('Escape');

        const drinking = await accountSuggestions('drinking');
        expect(drinking).toEqual(['Expenses :: Utilities :: Water :: Drinking']);
        await commitHighlightedAccount();

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('account:Drinking');
    });
});
