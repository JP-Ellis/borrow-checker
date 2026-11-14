/**
 * Flow test for spread attribution on the budget page: a 1,200 AUD bill spread
 * over 1 Jan–31 Dec 2026 counts at its March share, and the transaction list
 * shows that share followed by the full amount and the spread range.
 *
 * The spec seeds its own expense account, budget and bill into the worker's
 * database, under names no other spec searches for, so it depends on neither
 * the seed's dates nor the clock. March 2026 has 31 of the year's 365 days:
 * 1,200 × 31 / 365 = 101.92.
 */
import Database from 'better-sqlite3';
import { browser, $, expect } from '@wdio/globals';
import { DB_PATH } from '../support/db.js';
import { freshView } from '../support/nav.js';

// ── Seed ───────────────────────────────────────────────────────────────────

const BUDGET_NAME = 'Spread Premium';
const PAYEE = 'Insurance Co';
const CREATED_AT = '2026-01-01T00:00:00Z';

/** A valid TypeID: `prefix`, then `tag` left-padded with zeros to 26 chars. */
function typeId(prefix: string, tag: string): string {
    return `${prefix}_${tag.padStart(26, '0')}`;
}

/**
 * Inserts an expense account under `Expenses`, a monthly 100 AUD budget on it
 * from 2026-01-01, and a 1,200 AUD bill dated 2026-01-01 paid from Checking
 * whose expense leg spreads over 2026.
 */
function seedSpreadBill(): void {
    const db = new Database(DB_PATH);
    try {
        const parentOf = (name: string): string => {
            const row = db
                .prepare('SELECT id FROM accounts WHERE name = ? AND archived_at IS NULL LIMIT 1')
                .get(name) as { id: string } | undefined;
            if (row === undefined) throw new Error(`seed has no ${name} account`);
            return row.id;
        };
        const expenses = parentOf('Expenses');
        const checking = parentOf('Checking');
        const account = typeId('account', '9sp1');
        const budget = typeId('budget', '9sp1');
        const tx = typeId('transaction', '9sp1');

        db.transaction(() => {
            db.prepare(
                `INSERT INTO accounts (id, name, account_type, kind, parent_id, created_at)
                 VALUES (?, ?, 'expense', 'deposit_account', ?, ?)`,
            ).run(account, BUDGET_NAME, expenses, CREATED_AT);
            db.prepare('INSERT INTO budgets (id, account_id, created_at) VALUES (?, ?, ?)')
                .run(budget, account, CREATED_AT);
            db.prepare(
                `INSERT INTO budget_revisions
                   (id, budget_id, effective_from, name, target_amount, target_currency,
                    period, rollover, intent, created_at)
                 VALUES (?, ?, '2026-01-01', ?, '100.00', 'AUD',
                         '{"type":"monthly"}', 'reset_to_zero', 'limit', ?)`,
            ).run(typeId('budget_rev', '9sp1'), budget, BUDGET_NAME, CREATED_AT);
            db.prepare(
                `INSERT INTO transactions (id, date, description, reconciliation, created_at)
                 VALUES (?, '2026-01-01', 'Annual premium', 'reconciled', ?)`,
            ).run(tx, CREATED_AT);
            db.prepare(
                `INSERT INTO transaction_metadata (transaction_id, key, position, value_text, mismatched)
                 VALUES (?, 'payee', 0, ?, 0)`,
            ).run(tx, PAYEE);
            const leg = db.prepare(
                `INSERT INTO postings
                   (id, transaction_id, account_id, amount, commodity, position, spread_from, spread_until)
                 VALUES (?, ?, ?, ?, 'AUD', ?, ?, ?)`,
            );
            leg.run(typeId('posting', '9sp1'), tx, account, '1200.00', 0, '2026-01-01', '2026-12-31');
            leg.run(typeId('posting', '9sp2'), tx, checking, '-1200.00', 1, null, null);
        })();
    } finally {
        db.close();
    }
}

// ── UI helpers ─────────────────────────────────────────────────────────────

/** The current "Month YYYY" period label in the main content. */
async function getMonthLabel(): Promise<string | null> {
    const text = await (await $('main')).getText();
    const m = text.match(
        /(?:January|February|March|April|May|June|July|August|September|October|November|December) \d{4}/,
    );
    return m?.[0] ?? null;
}

/** Opens /budget and waits for the tree to list the seeded budget. */
async function navigateToBudget(): Promise<void> {
    const nav = await $('nav[aria-label="main navigation"]');
    await (await nav.$('a=budget')).click();
    await browser.waitUntil(
        () => browser.execute(() => window.location.pathname === '/budget'),
        { timeoutMsg: 'URL did not reach /budget within 5 s' },
    );
    const tree = await $('[aria-label="budget tree"]');
    await tree.waitForDisplayed();
    await browser.waitUntil(
        async () => (await tree.getText()).includes(BUDGET_NAME),
        { timeoutMsg: `Budget tree did not list ${BUDGET_NAME}` },
    );
}

/** Months from `year`-`month` (1-based) to the current month; at least 0. */
function monthsSince(year: number, month: number): number {
    const now = new Date();
    return Math.max(0, (now.getFullYear() - year) * 12 + (now.getMonth() + 1 - month));
}

/**
 * Steps the period back with ◀ until the label reads `label`, a month in
 * `year`-`month`. The page opens on the current month, so the step count
 * follows from the clock, with a margin.
 */
async function stepBackTo(label: string, year: number, month: number): Promise<void> {
    const limit = monthsSince(year, month) + 3;
    for (let i = 0; i <= limit; i++) {
        const current = await getMonthLabel();
        if (current === label) return;
        await (await $('button=◀')).click();
        await browser.waitUntil(
            async () => (await getMonthLabel()) !== current,
            { timeoutMsg: 'Period did not change after ◀' },
        );
    }
    throw new Error(`Never reached ${label}`);
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Budget — spread posting share', () => {
    before(async () => {
        seedSpreadBill();
        await freshView();
        await navigateToBudget();
        await stepBackTo('March 2026', 2026, 3);
    });

    it('lists the March share, then the full amount and the spread range', async () => {
        const tree = await $('[aria-label="budget tree"]');
        await (await tree.$(`span=${BUDGET_NAME}`)).click();
        const detail = await $('[aria-label="budget detail"]');
        await detail.waitForDisplayed();

        let text = '';
        await browser
            .waitUntil(async () => {
                text = await (await $('[aria-label="budget detail"]')).getText();
                return text.includes('101.92');
            })
            .catch(() => {
                throw new Error(`Detail panel did not list the March share; it shows: ${text}`);
            });

        // Grouping follows the webview's locale, so `1,200.00` may render as
        // `1200.00`; drop the separator before matching.
        const plain = text.replace(/(\d)[,   ](?=\d{3}\b)/g, '$1');
        const share = plain.indexOf('101.92');
        const note = plain.indexOf('(1200.00 · 1 Jan–31 Dec)');
        expect({ text, follows: note > share }).toEqual({ text, follows: true });
    });
});
