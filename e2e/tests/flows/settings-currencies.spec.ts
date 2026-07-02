/**
 * Flow tests for Settings → Currencies (editable currency-registry panel).
 *
 * The app seeds 9 default currencies via `seed_defaults` at startup (USD,
 * AUD, EUR, …). AUD is the commodity used by every seeded posting (see
 * `bc-seed`'s `amt()` helper), so it is guaranteed to be "in use" and is
 * used below to exercise the referenced-delete block.
 *
 * The panel's delete model is *staged*: clicking the trash icon on a saved
 * row only flags it (struck-through, with an undo button) — the actual
 * backend delete, and any "in use" rejection, happens when Save is clicked.
 *
 * Test sequence:
 *   1. Navigate to Settings → Currencies; assert the 9 seeded currencies render.
 *   2. Add a currency (NZD / NZ$), save, and confirm it persists after
 *      re-navigating away and back.
 *   3. Edit an alias to "$" (colliding with USD's symbol) and assert Save is
 *      disabled with the conflict message shown.
 *   4. Stage-delete AUD (referenced by seeded postings), save, and assert the
 *      in-use block banner appears and the row is not actually removed.
 */
import { dirname }       from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolve }       from 'node:path';
import Database           from 'better-sqlite3';
import { browser, $, $$ } from '@wdio/globals';

const __dirname  = dirname(fileURLToPath(import.meta.url));
const DB_PATH    = resolve(__dirname, '../../fixtures/test.db');

// ── DB helpers ──────────────────────────────────────────────────────────────

function dbHasCommodityCode(code: string): boolean {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare('SELECT 1 FROM commodities WHERE code = ?')
            .get(code) as unknown;
        return row !== undefined;
    } finally {
        db.close();
    }
}

// ── Navigation helpers ──────────────────────────────────────────────────────

/**
 * Navigate to Settings, click the "Currencies" sidebar item, and wait for at
 * least one currency row to render.
 */
async function openSettingsCurrencies(): Promise<void> {
    const nav = await $('nav[aria-label="main navigation"]');
    await nav.waitForExist();
    const settingsLink = await nav.$('a=settings');
    await settingsLink.waitForDisplayed();
    await settingsLink.click();

    await browser.waitUntil(
        () => browser.execute(() => window.location.pathname === '/settings'),
        { timeoutMsg: 'Pathname did not reach /settings within 5 s' },
    );

    const currenciesNav = await $('[data-testid="settings-nav-currencies"]');
    await currenciesNav.waitForDisplayed();
    await currenciesNav.click();

    await browser.waitUntil(
        async () => {
            const rows = await $$('[data-testid="currency-row"]');
            return (await rows.length) > 0;
        },
        { timeoutMsg: 'No currency rows appeared in the Currencies panel' },
    );
}

/** Return the `(row element, code)` pairs currently rendered in the table. */
async function currencyRows(): Promise<{ row: WebdriverIO.Element; code: string }[]> {
    const rows = await $$('[data-testid="currency-row"]');
    const out: { row: WebdriverIO.Element; code: string }[] = [];
    for (const row of rows) {
        const codeInput = await row.$('[data-testid="currency-code"]');
        const code = await codeInput.getValue();
        out.push({ row, code });
    }
    return out;
}

/** Height (px) of the save bar — 0 when retracted, non-zero when dirty. */
async function saveBarHeight(): Promise<number> {
    const bar = await $('[data-testid="currency-savebar"]');
    return browser.execute(
        (el: Element) => el.getBoundingClientRect().height,
        await bar.getElement() as unknown as Element,
    );
}

async function isSaveDisabled(): Promise<boolean> {
    const saveBtn = await $('[data-testid="currency-save"]');
    return browser.execute(
        (btn: Element) => (btn as HTMLButtonElement).disabled,
        await saveBtn.getElement() as unknown as Element,
    );
}

// ── Tests ───────────────────────────────────────────────────────────────────

describe('Settings — Currencies', () => {
    it('renders the seeded currencies', async () => {
        await openSettingsCurrencies();

        const rows = await currencyRows();
        expect(rows.length).toBe(9);

        const codes = rows.map(r => r.code);
        expect(codes).toContain('USD');
        expect(codes).toContain('AUD');
        expect(codes).toContain('EUR');
    });

    it('adds a currency and persists it after re-navigating away and back', async () => {
        await openSettingsCurrencies();

        const addBtn = await $('[data-testid="currency-add"]');
        await addBtn.click();

        const codeInputs = await $$('[data-testid="currency-code"]');
        const symbolInputs = await $$('[data-testid="currency-symbol"]');
        const codeCount = await codeInputs.length;
        const symbolCount = await symbolInputs.length;
        const newCode = codeInputs[codeCount - 1];
        const newSymbol = symbolInputs[symbolCount - 1];
        if (!newCode || !newSymbol) throw new Error('new currency row inputs not found');

        await newCode.setValue('NZD');
        await newSymbol.setValue('NZ$');

        // Save bar must be showing (dirty) before saving.
        await browser.waitUntil(
            async () => (await saveBarHeight()) > 0,
            { timeoutMsg: 'Save bar did not appear after adding a currency' },
        );

        const saveBtn = await $('[data-testid="currency-save"]');
        await saveBtn.click();

        // Save bar retracts once the IPC writes complete.
        await browser.waitUntil(
            async () => (await saveBarHeight()) === 0,
            { timeoutMsg: 'Save bar did not retract after saving NZD' },
        );

        // Re-navigate away and back, then confirm NZD is still present.
        const nav = await $('nav[aria-label="main navigation"]');
        const accountsLink = await nav.$('a=accounts');
        await accountsLink.click();
        await browser.waitUntil(
            () => browser.execute(() => window.location.pathname === '/accounts'),
            { timeoutMsg: 'Pathname did not reach /accounts within 5 s' },
        );

        await openSettingsCurrencies();
        const rows = await currencyRows();
        expect(rows.map(r => r.code)).toContain('NZD');
    });

    it('disables Save and shows the conflict message on an alias collision', async () => {
        await openSettingsCurrencies();

        const rows = await currencyRows();
        const audRow = rows.find(r => r.code === 'AUD');
        if (!audRow) throw new Error('AUD row not found');

        const aliasInput = await audRow.row.$('[data-testid="currency-alias-input"]');
        await aliasInput.setValue('$');
        await browser.keys(['Enter']);

        // The conflict message must show and Save must be disabled.
        const conflict = await $('[data-testid="currency-conflict"]');
        await browser.waitUntil(
            async () => (await conflict.getText()) !== '',
            { timeoutMsg: 'Conflict message did not appear' },
        );
        const conflictText = await conflict.getText();
        expect(conflictText).toContain('$');

        expect(await isSaveDisabled()).toBe(true);

        // Discard so later tests are not blocked by this unsaved conflict.
        const discardBtn = await $('[data-testid="currency-discard"]');
        await discardBtn.click();
        await browser.waitUntil(
            async () => (await saveBarHeight()) === 0,
            { timeoutMsg: 'Save bar did not retract after discarding' },
        );
    });

    it('blocks deleting a referenced currency (AUD) and shows the in-use banner', async () => {
        await openSettingsCurrencies();

        const rowsBefore = await currencyRows();
        const audRowBefore = rowsBefore.find(r => r.code === 'AUD');
        if (!audRowBefore) throw new Error('AUD row not found');
        expect(dbHasCommodityCode('AUD')).toBe(true);

        // Stage the delete — flags the row but does not remove it yet.
        const deleteBtn = await audRowBefore.row.$('[data-testid="currency-delete"]');
        await deleteBtn.click();

        await browser.waitUntil(
            async () => (await audRowBefore.row.getAttribute('data-deleted')) === 'true',
            { timeoutMsg: 'AUD row was not flagged as staged for deletion' },
        );

        // Row remains present (staged, not removed) with an undo button.
        const rowsStaged = await currencyRows();
        expect(rowsStaged.map(r => r.code)).toContain('AUD');
        const undoBtn = await audRowBefore.row.$('[data-testid="currency-undo"]');
        await undoBtn.waitForDisplayed();

        // Save must be enabled (staged delete is a valid, non-conflicting change).
        expect(await isSaveDisabled()).toBe(false);

        const saveBtn = await $('[data-testid="currency-save"]');
        await saveBtn.click();

        // The in-use block banner must appear, referencing AUD.
        const banner = await $('[data-testid="currency-banner"]');
        await banner.waitForDisplayed({ timeoutMsg: 'In-use block banner did not appear after saving a referenced delete',
        });
        const bannerText = await banner.getText();
        expect(bannerText).toContain('AUD');

        // The row must NOT have actually been removed from the database.
        expect(dbHasCommodityCode('AUD')).toBe(true);

        const rowsAfter = await currencyRows();
        expect(rowsAfter.map(r => r.code)).toContain('AUD');
    });
});
