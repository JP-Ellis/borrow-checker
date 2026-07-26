import Database          from 'better-sqlite3';
import { browser, $, $$, expect as wdioExpect } from '@wdio/globals';
import { DB_PATH as TEST_DB_PATH } from '../support/db.js';

// ── Types ──────────────────────────────────────────────────────────────────

interface BudgetRevisionRow {
    id:              string;
    budget_id:       string;
    name:            string | null;
    target_amount:   string | null;
    target_currency: string | null;
}

interface BudgetRow {
    id:          string;
    archived_at: string | null;
}

// ── DB helpers ─────────────────────────────────────────────────────────────

/**
 * Return the earliest (initial) revision for the budget whose name matches `name`.
 * "Earliest" is by `effective_from ASC`, which is also the first row shown in the UI.
 */
function dbGetFirstRevision(name: string): BudgetRevisionRow | undefined {
    const db = new Database(TEST_DB_PATH, { readonly: true });
    try {
        return db
            .prepare(
                'SELECT br.id, br.budget_id, br.name, br.target_amount, br.target_currency ' +
                'FROM budget_revisions br ' +
                'WHERE br.budget_id = (' +
                '  SELECT budget_id FROM budget_revisions WHERE name = ? ORDER BY effective_from ASC LIMIT 1' +
                ') ' +
                'ORDER BY br.effective_from ASC LIMIT 1',
            )
            .get(name) as BudgetRevisionRow | undefined;
    } finally {
        db.close();
    }
}

/** Return the `budgets` anchor row by looking up via the initial revision name. */
function dbGetBudget(name: string): BudgetRow | undefined {
    const db = new Database(TEST_DB_PATH, { readonly: true });
    try {
        return db
            .prepare(
                'SELECT b.id, b.archived_at ' +
                'FROM budgets b ' +
                'JOIN budget_revisions br ON br.budget_id = b.id ' +
                'WHERE br.name = ? ' +
                'ORDER BY br.effective_from ASC LIMIT 1',
            )
            .get(name) as BudgetRow | undefined;
    } finally {
        db.close();
    }
}

// ── UI helpers ─────────────────────────────────────────────────────────────

/** Extract the current "Month YYYY" period label from main content. */
async function getMonthLabel(): Promise<string | null> {
    const text = await (await $('main')).getText();
    const m = text.match(
        /(?:January|February|March|April|May|June|July|August|September|October|November|December) \d{4}/,
    );
    return m?.[0] ?? null;
}

/** Navigate to /budget and wait for IPC data to populate the tree. */
async function navigateToBudget(): Promise<void> {
    const nav = await $('nav[aria-label="main navigation"]');
    await (await nav.$('a=budget')).click();

    await browser.waitUntil(
        () => browser.execute(() => window.location.pathname === '/budget'),
        { timeoutMsg: 'URL did not reach /budget within 5 s' },
    );

    const tree = await $('[aria-label="budget tree"]');

    /* Wait for the tree. On timeout, capture page text to diagnose whether
     * the budget page is stuck in loading / error / empty state. */
    try {
        await tree.waitForDisplayed();
    } catch {
        const body = await (await $('body')).getText().catch(() => '<body unavailable>');
        throw new Error(
            `[aria-label="budget tree"] not displayed after 15 s.\n` +
            `Page shows: ${body.slice(0, 800)}`,
        );
    }

    await browser.waitUntil(
        async () => (await tree.getText()).includes('Groceries'),
        { timeoutMsg: 'Budget tree did not populate within 15 s' },
    );
}

/**
 * Click a budget row's name span in the tree to open its detail panel.
 * Waits for [aria-label="budget detail"] to become visible.
 */
async function openDetail(name: string): Promise<void> {
    const tree = await $('[aria-label="budget tree"]');
    await (await tree.$(`span=${name}`)).click();
    await (await $('[aria-label="budget detail"]')).waitForDisplayed();
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

    it('shows a valid monthly period label in "Month YYYY" format', async () => {
        expect(await getMonthLabel()).not.toBeNull();
    });

    it('◀ steps back one month', async () => {
        const before = await getMonthLabel();
        await (await $('button=◀')).click();
        await browser.waitUntil(
            async () => {
                const after = await getMonthLabel();
                return after !== null && after !== before;
            },
            { timeoutMsg: 'Period did not change after ◀' },
        );
    });

    it('▶ steps forward one month', async () => {
        const before = await getMonthLabel();
        await (await $('button=▶')).click();
        await browser.waitUntil(
            async () => {
                const after = await getMonthLabel();
                return after !== null && after !== before;
            },
            { timeoutMsg: 'Period did not change after ▶' },
        );
    });

    it('granularity select → Quarterly shows Q format then restores Monthly', async () => {
        await setSelectValue('quarterly');
        await browser.waitUntil(
            async () => Boolean((await (await $('main')).getText()).match(/Q[1-4] \d{4}/)),
            { timeoutMsg: 'Period label did not update to quarterly format' },
        );
        await setSelectValue('monthly');
        await browser.waitUntil(
            async () => Boolean(await getMonthLabel()),
            { timeoutMsg: 'Period label did not restore to monthly format' },
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
});

describe('Budget — detail panel', () => {
    before(async () => {
        await navigateToBudget();
        await openDetail('Groceries');
    });

    it('shows Revisions section and Actions section in the detail panel', async () => {
        const text = (await (await $('[aria-label="budget detail"]')).getText()).toLowerCase();
        expect(text).toContain('revisions');
        expect(text).toContain('actions');
    });

    it('shows revision rows in the revisions timeline', async () => {
        /* Groceries has 2 seed revisions (Jan 2026 and Jul 2026). */
        await browser.waitUntil(
            async () => {
                const count = await browser.execute(
                    () => document.querySelectorAll('[data-testid="revision-row"]').length,
                );
                return count >= 1;
            },
            { timeoutMsg: 'No revision rows appeared in the detail panel' },
        );
    });

    it('shows Actions section with Archive button', async () => {
        const detail = await $('[aria-label="budget detail"]');
        expect((await detail.getText()).toLowerCase()).toContain('actions');
        await wdioExpect(await detail.$('button*=Archive budget')).toBeDisplayed();
    });

    it('clicking Groceries row again closes the detail panel', async () => {
        await (await (await $('[aria-label="budget tree"]')).$('span=Groceries')).click();
        await (await $('[aria-label="budget detail"]')).waitForDisplayed({
            reverse: true,
        });
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
        await (await $('button=Yes, archive')).waitForDisplayed();
        await clickButton('Yes, archive');

        /* — UI: Subscriptions disappears from the tree. */
        const tree = await $('[aria-label="budget tree"]');
        await browser.waitUntil(
            async () => !(await tree.getText()).includes('Subscriptions'),
            { timeoutMsg: 'Subscriptions did not disappear from tree after archive' },
        );

        /* — DB: archived_at is now set (non-null) on the budgets anchor row. */
        const row = dbGetBudget('Subscriptions');
        expect(row).toBeDefined();
        expect(row!.archived_at).not.toBeNull();
    });
});

describe('Budget — revision timeline', () => {
    it('adds a future-dated revision and shows a new row in the timeline', async () => {
        await navigateToBudget();
        await openDetail('Groceries');

        /* Count rows before adding. */
        const countRevRows = () =>
            browser.execute(
                () => document.querySelectorAll('[data-testid="revision-row"]').length,
            );
        await browser.waitUntil(
            async () => (await countRevRows()) >= 1,
            { timeoutMsg: 'Revision rows did not appear before add test' },
        );
        const beforeCount = await countRevRows();

        /* Open the add-revision form. */
        await clickButton('＋ add revision');
        await (await $('[aria-label="revision form"]')).waitForDisplayed();

        /* Set a clearly-future effective date and a target amount. */
        await setInputValue('[aria-label="revision form"] input[type="date"]', '2027-01-01');
        await setInputValue('[aria-label="revision form"] input[type="number"]', '250.00');

        /* Save and wait for the row count to increase. */
        await clickButton('Save');
        await browser.waitUntil(
            async () => (await countRevRows()) === beforeCount + 1,
            { timeoutMsg: 'New revision row did not appear after Save' },
        );
    });

    it('amends an existing revision by clicking a revision row', async () => {
        /* Navigate away first to reset detail-panel state, then back to budget. */
        await (await $('nav[aria-label="main navigation"]')).$('a=accounts').click();
        await browser.waitUntil(
            () => browser.execute(() => window.location.pathname.startsWith('/accounts')),
            { timeoutMsg: 'URL did not reach /accounts within 5 s' },
        );
        await navigateToBudget();
        await openDetail('Groceries');

        /* Wait for at least one revision row. */
        await browser.waitUntil(
            async () => {
                const count = await browser.execute(
                    () => document.querySelectorAll('[data-testid="revision-row"]').length,
                );
                return count >= 1;
            },
            { timeoutMsg: 'Revision rows did not appear before amend test' },
        );

        /* Click the first revision row to open the amend form. */
        const revRows = await $$('[data-testid="revision-row"]');
        const firstRow = revRows[0];
        await firstRow.click();
        await (await $('[aria-label="revision form"]')).waitForDisplayed();

        /* Change the target amount to a distinctive value. */
        await setInputValue('[aria-label="revision form"] input[type="number"]', '999.00');

        /* Save and wait for the revision form to close. */
        await clickButton('Save');
        await (await $('[aria-label="revision form"]')).waitForDisplayed({
            reverse: true,
        });

        /* — UI: the detail panel text should now contain "999". */
        await browser.waitUntil(
            async () => {
                const text = await (await $('[aria-label="budget detail"]')).getText();
                return text.includes('999');
            },
            { timeoutMsg: 'Detail panel did not show "999" after amend' },
        );

        /* — DB: the first (oldest) revision for Groceries now has target_amount ~999
         * (the amend test clicked the first row in the list, which is sorted
         * by effective_from ASC and corresponds to the 2026-01-01 seed revision). */
        const rev = dbGetFirstRevision('Groceries');
        expect(rev).toBeDefined();
        expect(rev!.target_amount).toMatch(/^999/);
    });
});
