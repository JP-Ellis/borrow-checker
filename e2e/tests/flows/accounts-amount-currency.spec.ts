/**
 * Flow tests for currency-marker validation on posting amount inputs.
 *
 * Amounts must carry an explicit currency marker (e.g. "AUD 110" or "A$110").
 * A bare number ("110") or an unknown marker ("XYZ 110") must block the Save
 * button; a correctly-marked amount that leaves the transaction balanced or
 * inferred must enable it.
 *
 * The seeded Coles/Groceries transaction has two postings:
 *   - Groceries  +AUD 110.00
 *   - CreditCard −AUD 110.00
 *
 * `from_posting` seeds the working buffer as "<code> <value>" (e.g. "AUD 110"),
 * so both postings already carry markers on open.
 */
import { browser, $$, $ }    from '@wdio/globals';

// ── Navigation helpers ──────────────────────────────────────────────────────

/**
 * Navigate to Accounts → Groceries and wait for transaction rows to appear.
 */
async function openGroceriesAccount(): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed({ timeout: 5_000 });
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeout: 5_000, timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed({ timeout: 10_000 });

    const groceriesSpan = await sidebarNav.$('span=Groceries');
    await groceriesSpan.waitForDisplayed({ timeout: 10_000 });
    await groceriesSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeout: 5_000, timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    const register = await $('[aria-label="transaction register"]');
    await register.waitForDisplayed({ timeout: 10_000 });
    await browser.waitUntil(
        () => browser.execute(
            () => (document.querySelector('[aria-label="transaction register"]')
                ?.querySelectorAll('[role="button"]').length ?? 0) > 0,
        ),
        { timeout: 15_000, timeoutMsg: 'No transaction rows appeared in the Groceries register' },
    );
}

/**
 * Expand the first "Coles" transaction row and wait for the status pill.
 */
async function expandColesRow(): Promise<void> {
    const register = await $('[aria-label="transaction register"]');

    const colesSpan = await register.$('span=Coles');
    await colesSpan.waitForDisplayed({
        timeout: 10_000,
        timeoutMsg: '"Coles" row did not appear in the Groceries register',
    });

    await browser.execute((el: Element) => {
        const row = el.closest('[role="button"]');
        if (row instanceof HTMLElement) row.click();
    }, await colesSpan.getElement() as unknown as Element);

    const pill = await $('[data-testid="status-pill"]');
    await pill.waitForDisplayed({
        timeout: 5_000,
        timeoutMsg: 'Status pill did not appear after expanding the Coles row',
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
    await input.waitForDisplayed({ timeout: 3_000 });
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
        await expandColesRow();

        /* Type a bare number — missing the required currency prefix. */
        await setPostingAmount(0, '999');

        /* The save bar must appear (the transaction is now dirty). */
        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({
            timeout: 3_000,
            timeoutMsg: 'Save bar did not appear after changing a posting amount',
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
        await expandColesRow();

        /* "XYZ" is not a known commodity code, symbol, or alias. */
        await setPostingAmount(0, 'XYZ 999');

        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({
            timeout: 3_000,
            timeoutMsg: 'Save bar did not appear after typing an unknown currency',
        });

        const isDisabled: boolean = await browser.execute(
            (btn: Element) => (btn as HTMLButtonElement).disabled,
            await saveBtn.getElement() as unknown as Element,
        );
        expect(isDisabled).toBe(true);
    });

    it('enables Save when a valid currency marker produces an inferred balance', async () => {
        await openGroceriesAccount();
        await expandColesRow();

        /*
         * Clear the second posting's amount so it becomes elided (inferred).
         * The first posting still carries "AUD 110.00" from the seed, so
         * derive_balance will return BalanceState::Inferred which enables Save.
         */
        await setPostingAmount(1, '');

        const saveBtn = await $('[aria-label="save transaction"]');
        await saveBtn.waitForDisplayed({
            timeout: 3_000,
            timeoutMsg: 'Save bar did not appear after clearing the second posting amount',
        });

        const isDisabled: boolean = await browser.execute(
            (btn: Element) => (btn as HTMLButtonElement).disabled,
            await saveBtn.getElement() as unknown as Element,
        );
        expect(isDisabled).toBe(false);
    });
});
