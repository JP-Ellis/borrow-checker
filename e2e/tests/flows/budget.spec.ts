import { dirname }       from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolve }       from 'node:path';
import Database          from 'better-sqlite3';
import { browser, $, $$, expect as wdioExpect } from '@wdio/globals';

const __dirname    = dirname(fileURLToPath(import.meta.url));
const TEST_DB_PATH = resolve(__dirname, '../../fixtures/test.db');

// ── Types ──────────────────────────────────────────────────────────────────

interface BudgetRow {
    id:              string;
    name:            string | null;
    target_amount:   string | null;
    target_currency: string | null;
    archived_at:     string | null;
}

// ── DB helpers ─────────────────────────────────────────────────────────────

function dbGetBudget(name: string): BudgetRow | undefined {
    const db = new Database(TEST_DB_PATH, { readonly: true });
    try {
        return db
            .prepare(
                'SELECT id, name, target_amount, target_currency, archived_at ' +
                'FROM budgets WHERE name = ?',
            )
            .get(name) as BudgetRow | undefined;
    } finally {
        db.close();
    }
}

// ── UI helpers ─────────────────────────────────────────────────────────────

/** Navigate to /budget and wait for IPC data to populate the tree. */
async function navigateToBudget(): Promise<void> {
    const nav = await $('nav[aria-label="main navigation"]');
    await (await nav.$('a=budget')).click();

    await browser.waitUntil(
        () => browser.execute(() => window.location.pathname === '/budget'),
        { timeout: 5_000, timeoutMsg: 'URL did not reach /budget within 5 s' },
    );

    const tree = await $('[aria-label="budget tree"]');

    /* Wait for the tree. On timeout, capture page text to diagnose whether
     * the budget page is stuck in loading / error / empty state. */
    try {
        await tree.waitForDisplayed({ timeout: 15_000 });
    } catch {
        const body = await (await $('body')).getText().catch(() => '<body unavailable>');
        throw new Error(
            `[aria-label="budget tree"] not displayed after 15 s.\n` +
            `Page shows: ${body.slice(0, 800)}`,
        );
    }

    await browser.waitUntil(
        async () => (await tree.getText()).includes('Groceries'),
        { timeout: 15_000, timeoutMsg: 'Budget tree did not populate within 15 s' },
    );
}

/**
 * Click a budget row's name span in the tree to open its detail panel.
 * Waits for [aria-label="budget detail"] to become visible.
 */
async function openDetail(name: string): Promise<void> {
    const tree = await $('[aria-label="budget tree"]');
    await (await tree.$(`span=${name}`)).click();
    await (await $('[aria-label="budget detail"]')).waitForDisplayed({ timeout: 5_000 });
}

/**
 * Poll all visible buttons until one has the exact text, then click it.
 * Handles conditionally-rendered buttons (Leptos <Show> blocks).
 */
async function clickButton(label: string, timeout = 5_000): Promise<void> {
    await browser.waitUntil(
        async () => {
            for (const btn of await $$('button')) {
                if ((await btn.getText()) === label) {
                    await btn.click();
                    return true;
                }
            }
            return false;
        },
        { timeout, timeoutMsg: `"${label}" button did not appear within ${timeout} ms` },
    );
}

/** Wait until no button on the page has the given exact text. */
async function waitForButtonGone(label: string, timeout = 10_000): Promise<void> {
    await browser.waitUntil(
        async () => {
            for (const btn of await $$('button')) {
                if ((await btn.getText()) === label) return false;
            }
            return true;
        },
        { timeout, timeoutMsg: `"${label}" button did not disappear within ${timeout} ms` },
    );
}

/**
 * Set an <input> value via direct DOM manipulation and fire a bubbling 'input'
 * event so Leptos's on:input handler fires and updates reactive state.
 */
async function setInputValue(selector: string, value: string): Promise<void> {
    await browser.execute((sel: string, val: string) => {
        const el = document.querySelector(sel) as HTMLInputElement | null;
        if (!el) throw new Error(`Input not found: ${sel}`);
        el.value = val;
        el.dispatchEvent(new Event('input', { bubbles: true }));
    }, selector, value);
}

/**
 * Set a <select> value via direct DOM manipulation and fire the change event
 * so Leptos's on:change handler fires and updates reactive state.
 * Uses browser.execute to avoid WebdriverIO type gaps with ChainablePromiseElement.
 */
async function setSelectValue(value: string): Promise<void> {
    await browser.execute((val: string) => {
        const el = document.querySelector('select') as HTMLSelectElement | null;
        if (!el) throw new Error('select element not found');
        el.value = val;
        el.dispatchEvent(new Event('change', { bubbles: true }));
    }, value);
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Budget — period navigation', () => {
    before(async () => {
        await navigateToBudget();
    });

    it('shows "January 2025" with frozen clock (2025-01-15)', async () => {
        expect(await (await $('main')).getText()).toContain('January 2025');
    });

    it('◀ steps back to December 2024', async () => {
        await (await $('button=◀')).click();
        await browser.waitUntil(
            async () => (await (await $('main')).getText()).includes('December 2024'),
            { timeout: 5_000, timeoutMsg: 'Period did not update to December 2024' },
        );
    });

    it('▶ returns to January 2025', async () => {
        await (await $('button=▶')).click();
        await browser.waitUntil(
            async () => (await (await $('main')).getText()).includes('January 2025'),
            { timeout: 5_000, timeoutMsg: 'Period did not return to January 2025' },
        );
    });

    it('granularity select → Quarterly shows Q1 2025 then restores Monthly', async () => {
        await setSelectValue('quarterly');
        await browser.waitUntil(
            async () => (await (await $('main')).getText()).includes('Q1 2025'),
            { timeout: 5_000, timeoutMsg: 'Period label did not update to Q1 2025' },
        );
        await setSelectValue('monthly');
        await browser.waitUntil(
            async () => (await (await $('main')).getText()).includes('January 2025'),
            { timeout: 5_000, timeoutMsg: 'Period label did not restore to January 2025' },
        );
    });

    it('% mode toggle cycles "$ value" → "% target" → "$ value"', async () => {
        await wdioExpect(await $('button=$ value')).toBeDisplayed();
        await (await $('button=$ value')).click();
        await wdioExpect(await $('button=% target')).toBeDisplayed();
        await (await $('button=% target')).click();
        await wdioExpect(await $('button=$ value')).toBeDisplayed();
    });
});

describe('Budget — tree display', () => {
    before(async () => {
        await navigateToBudget();
    });

    it('shows ACCOUNT / PROGRESS / SPENT / TARGET column headers', async () => {
        const text = await (await $('[aria-label="budget tree"]')).getText();
        expect(text).toContain('ACCOUNT');
        expect(text).toContain('PROGRESS');
        expect(text).toContain('SPENT / TARGET');
    });

    it('shows all 7 seed budget names', async () => {
        const text = await (await $('[aria-label="budget tree"]')).getText();
        for (const name of [
            'Groceries', 'Electricity', 'Transport', 'Dining',
            'Entertainment', 'Subscriptions', 'Healthcare',
        ]) {
            expect(text).toContain(name);
        }
    });

    it('rows show "· tracking" because seed budgets have no target_amount', async () => {
        /* All seed budgets are created without a target_amount, so
         * is_tracking_only=true and each row renders as "spent · tracking". */
        const text = await (await $('[aria-label="budget tree"]')).getText();
        expect(text).toContain('· tracking');
    });
});

describe('Budget — detail panel', () => {
    before(async () => {
        await navigateToBudget();
        await openDetail('Groceries');
    });

    it('shows Settings section with Name, Target, Period and Rollover fields', async () => {
        const text = await (await $('[aria-label="budget detail"]')).getText();
        expect(text).toContain('Settings');
        expect(text).toContain('Name');
        expect(text).toContain('Target');
        expect(text).toContain('Period');
        expect(text).toContain('Rollover');
    });

    it('shows Actions section with Archive button', async () => {
        const detail = await $('[aria-label="budget detail"]');
        expect(await detail.getText()).toContain('Actions');
        await wdioExpect(await detail.$('button*=Archive budget')).toBeDisplayed();
    });

    it('clicking Groceries row again closes the detail panel', async () => {
        await (await (await $('[aria-label="budget tree"]')).$('span=Groceries')).click();
        await (await $('[aria-label="budget detail"]')).waitForDisplayed({
            timeout: 5_000,
            reverse: true,
        });
    });
});

describe('Budget — edit budget', () => {
    before(async () => {
        await navigateToBudget();
        await openDetail('Groceries');
    });

    it('changing the name field shows Save and Reset buttons', async () => {
        await setInputValue(
            '[aria-label="budget detail"] input[type="text"]',
            'Groceries Renamed',
        );
        /* dirty=true → Leptos <Show> renders Save and Reset into the DOM. */
        await browser.waitUntil(
            async () => {
                for (const btn of await $$('button')) {
                    if ((await btn.getText()) === 'Save') return true;
                }
                return false;
            },
            { timeout: 5_000, timeoutMsg: 'Save button did not appear after name change' },
        );
        await wdioExpect(await $('button=Reset')).toBeDisplayed();
    });

    it('clicking Reset restores the original name and hides Save/Reset', async () => {
        await clickButton('Reset');
        /* dirty=false → <Show> removes Save and Reset from the DOM. */
        await waitForButtonGone('Save');
        await browser.waitUntil(
            async () => {
                const val = await browser.execute(() => {
                    const el = document.querySelector(
                        '[aria-label="budget detail"] input[type="text"]',
                    ) as HTMLInputElement | null;
                    return el?.value ?? null;
                });
                return val === 'Groceries';
            },
            { timeout: 5_000, timeoutMsg: 'Name input did not reset to "Groceries"' },
        );
    });

    it('saving a target converts tracking-only to budgeted — UI and DB', async () => {
        await setInputValue(
            '[aria-label="budget detail"] input[type="number"]',
            '500',
        );
        await clickButton('Save');
        /* dirty cleared → Save/Reset disappear once the IPC call returns. */
        await waitForButtonGone('Save');

        /* — UI: Groceries row should no longer show "· tracking".
         * After re-fetch, is_tracking_only=false because target_amount is now set. */
        await browser.waitUntil(
            async () => {
                const tree     = await $('[aria-label="budget tree"]');
                const nameSpan = await tree.$('span=Groceries');
                const rowDiv   = await nameSpan.$('..');
                return !(await rowDiv.getText()).includes('· tracking');
            },
            { timeout: 10_000, timeoutMsg: 'Groceries row did not leave tracking-only mode' },
        );

        /* — DB: target_amount='500.00', target_currency='AUD', not archived. */
        const row = dbGetBudget('Groceries');
        expect(row).toBeDefined();
        expect(row!.target_amount).toBe('500.00');
        expect(row!.target_currency).toBe('AUD');
        expect(row!.archived_at).toBeNull();
    });
});

describe('Budget — archive', () => {
    before(async () => {
        await navigateToBudget();
        await openDetail('Subscriptions');
    });

    it('"⊘ Archive budget" button shows confirmation', async () => {
        await (await (await $('[aria-label="budget detail"]')).$('button*=Archive budget')).click();
        await wdioExpect(await $('button=Yes, archive')).toBeDisplayed();
        await wdioExpect(await $('button=Cancel')).toBeDisplayed();
    });

    it('Cancel hides the confirmation and restores Archive button', async () => {
        await clickButton('Cancel');
        await waitForButtonGone('Yes, archive');
        await wdioExpect(
            await (await $('[aria-label="budget detail"]')).$('button*=Archive budget'),
        ).toBeDisplayed();
    });

    it('confirming archive removes the row from the tree and sets archived_at in DB', async () => {
        /* Re-open confirmation. */
        await (await (await $('[aria-label="budget detail"]')).$('button*=Archive budget')).click();
        await (await $('button=Yes, archive')).waitForDisplayed({ timeout: 3_000 });
        await clickButton('Yes, archive');

        /* — UI: Subscriptions disappears from the tree. */
        const tree = await $('[aria-label="budget tree"]');
        await browser.waitUntil(
            async () => !(await tree.getText()).includes('Subscriptions'),
            { timeout: 10_000, timeoutMsg: 'Subscriptions did not disappear from tree after archive' },
        );

        /* — DB: archived_at is now set (non-null). */
        const row = dbGetBudget('Subscriptions');
        expect(row).toBeDefined();
        expect(row!.archived_at).not.toBeNull();
    });
});
