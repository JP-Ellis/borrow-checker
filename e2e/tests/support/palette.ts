import { browser, $, $$, expect } from '@wdio/globals';

/**
 * Helpers for driving the ⌘K command palette.
 *
 * Committing a token re-renders the palette, which replaces its DOM nodes.
 * Element handles taken before a commit therefore go stale: WebdriverIO
 * recovers by re-finding them from the original selector, but each recovery
 * logs `Request encountered a stale element`. Everything here re-queries at the
 * point of use so the handle never outlives a render.
 */

const DIALOG = 'div[role="dialog"][aria-label="Command palette"]';
const INPUT  = `${DIALOG} input[role="combobox"]`;

/** Opens the palette and waits for the dialog to be displayed. */
export async function openPalette(): Promise<void> {
    const openButton = await $('button[aria-label="open command palette (⌘K)"]');
    await openButton.click();
    await expect(await $(DIALOG)).toBeDisplayed();
}

/** Types `raw` into the palette's inline search box. */
async function typeToken(raw: string): Promise<void> {
    await openPalette();
    const input = await $(INPUT);
    await input.waitForDisplayed();
    await input.setValue(raw);
}

/** Waits for the search box to clear, the signal that a token was committed. */
async function waitForCommit(): Promise<void> {
    await browser.waitUntil(
        async () => browser.execute(
            sel => (document.querySelector(sel) as HTMLInputElement | null)?.value === '',
            INPUT,
        ),
        { timeoutMsg: 'expected the input to clear after committing the token' },
    );
}

/**
 * Dismisses the palette.
 *
 * Chips sit behind the z-900 palette overlay, so the overlay must be gone
 * before any other top-bar interaction (chip removal) can land.
 */
async function closePalette(): Promise<void> {
    await browser.keys('Escape');
    await expect(await $(DIALOG)).not.toBeDisplayed();
}

/**
 * Types `tag:<tagName>`, narrows to the sole matching suggestion, and commits
 * it as a chip.
 */
export async function commitTagToken(tagName: string): Promise<void> {
    await typeToken(`tag:${tagName}`);

    await browser.waitUntil(
        async () => (await $$(`#palette-listbox div[role="option"]`).length) === 1,
        { timeoutMsg: `expected the tag search to narrow to \`${tagName}\`` },
    );

    const only = await $(`#palette-listbox div[role="option"]`);
    expect(await only.getAttribute('textContent')).toContain(tagName);
    await only.click();

    await waitForCommit();
    await closePalette();
}

/**
 * Types free payee/narration `text` and commits it with Enter.
 *
 * Free text has no listbox suggestions — the listbox shows only a
 * "↵ search payee/narration" hint.
 */
export async function commitTextToken(text: string): Promise<void> {
    await typeToken(text);
    await browser.keys('Enter');
    await waitForCommit();
    await closePalette();
}

/** Types an `after:<date>` token and commits it with Enter. */
export async function commitAfterToken(date: string): Promise<void> {
    await commitTextToken(`after:${date}`);
}
