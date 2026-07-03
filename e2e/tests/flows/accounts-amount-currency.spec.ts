/**
 * Flow tests for currency-marker validation on posting amount inputs.
 *
 * Amounts must carry an explicit currency marker (e.g. "AUD 110" or "A$110").
 * A bare number ("110") or an unknown marker ("XYZ 110") must block the Save
 * button; a correctly-marked amount that leaves the transaction balanced or
 * inferred must enable it.
 *
 * The seeded current-month Supermarket/Groceries transaction has two postings:
 *   - Groceries  +AUD 95.00
 *   - Checking   −AUD 95.00
 *
 * It is retargeted from the seed's historical "Coles" transactions because the
 * register is now scoped to the account's auto-jumped current period, and
 * "Coles" only appears in past months — "Supermarket" is the seed's
 * current-month Groceries transaction and is always visible on load.
 *
 * `from_posting` seeds the working buffer as "<code> <value>" (e.g. "AUD 95"),
 * so both postings already carry markers on open.
 */
import { browser, $$, $ }    from '@wdio/globals';

// ── Navigation helpers ──────────────────────────────────────────────────────

/**
 * Navigate to Accounts → Groceries and wait for transaction rows to appear.
 */
async function openGroceriesAccount(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();

    const groceriesSpan = await sidebarNav.$('span=Groceries');
    await groceriesSpan.waitForDisplayed();
    await groceriesSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    const register = await $('[aria-label="transaction register"]');
    await register.waitForDisplayed();
    await browser.waitUntil(
        () => browser.execute(
            () => (document.querySelector('[aria-label="transaction register"]')
                ?.querySelectorAll('[role="button"]').length ?? 0) > 0,
        ),
        { timeoutMsg: 'No transaction rows appeared in the Groceries register' },
    );
}

/**
 * Expand the current-month "Supermarket" transaction row and wait for the
 * status pill.
 */
async function expandSupermarketRow(): Promise<void> {
    const register = await $('[aria-label="transaction register"]');

    const supermarketSpan = await register.$('span=Supermarket');
    await supermarketSpan.waitForDisplayed({ timeoutMsg: '"Supermarket" row did not appear in the Groceries register',
    });

    await browser.execute((el: Element) => {
        const row = el.closest('[role="button"]');
        if (row instanceof HTMLElement) row.click();
    }, await supermarketSpan.getElement() as unknown as Element);

    const pill = await $('[data-testid="status-pill"]');
    await pill.waitForDisplayed({ timeoutMsg: 'Status pill did not appear after expanding the Supermarket row',
    });

    /* Give the currencies LocalResource time to resolve before we inspect
       balance state — IPC is local so this is effectively instantaneous, but
       we allow a short budget to be safe. */
    await browser.pause(400);
}

/**
 * Replace the value in a posting-amount input with `value`.
 *
 * Selects all existing text with Ctrl+A then types the replacement so that
 * whatever was seeded from the working buffer is completely overwritten.
 */
async function setPostingAmount(inputIndex: number, value: string): Promise<void> {
    const inputs = await $$('[data-testid="posting-amount"]');
    const input = inputs[inputIndex];
    if (!input) throw new Error(`posting-amount input [${inputIndex}] not found`);
    await input.waitForDisplayed();
    await input.click();
    await browser.keys(['Control', 'a']);
    if (value === '') {
        await browser.keys(['Backspace']);
    } else {
        await browser.keys(value.split(''));
    }
    /* Yield to Leptos reactive runtime so balance_state recomputes. */
    await browser.pause(150);
}

// ── Tests ───────────────────────────────────────────────────────────────────

describe('Accounts — currency-marker validation on posting amounts', () => {
    it('blocks Save when the amount has no currency marker', async () => {
        await openGroceriesAccount();
        await expandSupermarketRow();

        /* Type a bare number — missing the required currency prefix. */
        await setPostingAmount(0, '999');

        /* The save bar must appear (the transaction is now dirty). */
        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({ timeoutMsg: 'Save bar did not appear after changing a posting amount',
        });

        /* The Save button must be disabled because balance is Invalid. */
        const isDisabled: boolean = await browser.execute(
            (btn: Element) => (btn as HTMLButtonElement).disabled,
            await saveBtn.getElement() as unknown as Element,
        );
        expect(isDisabled).toBe(true);
    });

    it('blocks Save when the amount uses an unknown currency marker', async () => {
        await openGroceriesAccount();
        await expandSupermarketRow();

        /* "XYZ" is not a known commodity code, symbol, or alias. */
        await setPostingAmount(0, 'XYZ 999');

        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({ timeoutMsg: 'Save bar did not appear after typing an unknown currency',
        });

        const isDisabled: boolean = await browser.execute(
            (btn: Element) => (btn as HTMLButtonElement).disabled,
            await saveBtn.getElement() as unknown as Element,
        );
        expect(isDisabled).toBe(true);
    });

    it('enables Save when a valid currency marker produces an inferred balance', async () => {
        await openGroceriesAccount();
        await expandSupermarketRow();

        /*
         * Clear the second posting's amount so it becomes elided (inferred).
         * The first posting still carries "AUD 95.00" from the seed, so
         * derive_balance will return BalanceState::Inferred which enables Save.
         */
        await setPostingAmount(1, '');

        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({ timeoutMsg: 'Save bar did not appear after clearing the second posting amount',
        });

        const isDisabled: boolean = await browser.execute(
            (btn: Element) => (btn as HTMLButtonElement).disabled,
            await saveBtn.getElement() as unknown as Element,
        );
        expect(isDisabled).toBe(false);
    });
});
