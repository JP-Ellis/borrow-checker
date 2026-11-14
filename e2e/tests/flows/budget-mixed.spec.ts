/**
 * Flow test for a budget window across a target-commodity change: a monthly
 * budget of 200 AUD from 1 Jan 2026, then 150 USD from 1 Feb 2026, viewed over
 * Q1 2026. The row reads "mixed" with no verdict colour and an empty bar, and
 * its mixed-period badge expands into one sub-row per month, each in its own
 * commodity.
 *
 * The spec seeds its own expense account, budget and spend into the worker's
 * database, under names no other spec searches for, so it depends on neither
 * the seed's dates nor the clock.
 */
import Database from 'better-sqlite3';
import { browser, $, expect } from '@wdio/globals';
import { DB_PATH } from '../support/db.js';
import { freshView } from '../support/nav.js';

// ── Seed ───────────────────────────────────────────────────────────────────

const BUDGET_NAME = 'Mixed Provisions';
const CREATED_AT = '2026-01-01T00:00:00Z';

/** A valid TypeID: `prefix`, then `tag` left-padded with zeros to 26 chars. */
function typeId(prefix: string, tag: string): string {
    return `${prefix}_${tag.padStart(26, '0')}`;
}

/**
 * Inserts an expense account under `Expenses` with a monthly budget of
 * 200 AUD from 2026-01-01 and 150 USD from 2026-02-01, plus 120 AUD spent on
 * 2026-01-15 and 90 USD on 2026-02-10, both paid from Checking.
 */
function seedMixedBudget(): void {
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
        const account = typeId('account', '9mx1');
        const budget = typeId('budget', '9mx1');

        db.transaction(() => {
            db.prepare(
                `INSERT INTO accounts (id, name, account_type, kind, parent_id, created_at)
                 VALUES (?, ?, 'expense', 'deposit_account', ?, ?)`,
            ).run(account, BUDGET_NAME, expenses, CREATED_AT);
            db.prepare('INSERT INTO budgets (id, account_id, created_at) VALUES (?, ?, ?)')
                .run(budget, account, CREATED_AT);
            const revision = db.prepare(
                `INSERT INTO budget_revisions
                   (id, budget_id, effective_from, name, target_amount, target_currency,
                    period, rollover, intent, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, '{"type":"monthly"}', 'reset_to_zero', 'limit', ?)`,
            );
            revision.run(typeId('budget_rev', '9mx1'), budget, '2026-01-01', BUDGET_NAME,
                '200.00', 'AUD', CREATED_AT);
            revision.run(typeId('budget_rev', '9mx2'), budget, '2026-02-01', BUDGET_NAME,
                '150.00', 'USD', CREATED_AT);

            const tx = db.prepare(
                `INSERT INTO transactions (id, date, description, reconciliation, created_at)
                 VALUES (?, ?, ?, 'reconciled', ?)`,
            );
            const leg = db.prepare(
                `INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
                 VALUES (?, ?, ?, ?, ?, ?)`,
            );
            const spend = [
                { tag: '9mx1', date: '2026-01-15', amount: '120.00', commodity: 'AUD' },
                { tag: '9mx2', date: '2026-02-10', amount: '90.00', commodity: 'USD' },
            ];
            for (const s of spend) {
                const id = typeId('transaction', s.tag);
                tx.run(id, s.date, 'Pantry restock', CREATED_AT);
                leg.run(typeId('posting', `${s.tag}a`), id, account, s.amount, s.commodity, 0);
                leg.run(typeId('posting', `${s.tag}b`), id, checking, `-${s.amount}`, s.commodity, 1);
            }
        })();
    } finally {
        db.close();
    }
}

// ── UI helpers ─────────────────────────────────────────────────────────────

/** The current "Qn YYYY" period label in the main content. */
async function getQuarterLabel(): Promise<string | null> {
    const text = await (await $('main')).getText();
    return text.match(/Q[1-4] \d{4}/)?.[0] ?? null;
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

/** Switches the period granularity select to quarterly. */
async function selectQuarterly(): Promise<void> {
    await browser.execute(() => {
        const el = document.querySelector('select') as HTMLSelectElement | null;
        if (!el) throw new Error('select element not found');
        el.value = 'quarterly';
        el.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await browser.waitUntil(
        async () => (await getQuarterLabel()) !== null,
        { timeoutMsg: 'Period label did not switch to quarterly format' },
    );
}

/** Steps the period back with ◀ until the label reads `label`. */
async function stepBackTo(label: string): Promise<void> {
    for (let i = 0; i < 40; i++) {
        const current = await getQuarterLabel();
        if (current === label) return;
        await (await $('button=◀')).click();
        await browser.waitUntil(
            async () => (await getQuarterLabel()) !== current,
            { timeoutMsg: 'Period did not change after ◀' },
        );
    }
    throw new Error(`Never reached ${label}`);
}

/** The seeded budget's row: the nearest `div` above its label. */
function budgetRow() {
    return $('[aria-label="budget tree"]').$(
        `.//span[normalize-space()="${BUDGET_NAME}"]/ancestor::div[1]`,
    );
}

/** A native sub-row: the `div` holding the period label `start`. */
function subRow(start: string) {
    return $('[aria-label="budget tree"]').$(
        `.//span[normalize-space()="${start}"]/ancestor::div[1]`,
    );
}

/** The non-numeric characters of `text`: the commodity symbol or code. */
function symbolOf(text: string): string {
    return text.replace(/[\d.,\s−-]/g, '');
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Budget — window across a commodity change', () => {
    before(async () => {
        seedMixedBudget();
        await freshView();
        await navigateToBudget();
        await selectQuarterly();
        await stepBackTo('Q1 2026');
    });

    it('reads mixed, with no verdict colour and an empty bar', async () => {
        const row = await budgetRow();
        await browser
            .waitUntil(async () => (await row.getText()).includes('mixed'))
            .catch(async () => {
                throw new Error(`Row did not read mixed; it shows: ${await row.getText()}`);
            });

        const cells = await browser.execute((el: HTMLElement) => {
            const probe = (value: string): string => {
                const p = document.createElement('span');
                p.style.color = value;
                el.appendChild(p);
                const colour = getComputedStyle(p).color;
                p.remove();
                return colour;
            };
            const mixed = Array.from(el.querySelectorAll(':scope > span'))
                .filter((s) => s.textContent?.trim() === 'mixed');
            return {
                mixedCells: mixed.length,
                actualColour: mixed[0] ? getComputedStyle(mixed[0]).color : null,
                mute: probe('var(--bc-ink-mute)'),
                segments: Array.from(el.querySelectorAll('[style*="--seg"]'))
                    .map((s) => s.getAttribute('style') ?? ''),
            };
        }, row as unknown as HTMLElement);

        expect(cells.mixedCells).toBe(2);
        expect(cells.actualColour).toBe(cells.mute);
        expect(cells.segments.length).toBeGreaterThan(0);
        for (const style of cells.segments) {
            expect(style).toContain('var(--bc-ink-mute)');
            expect(style).toMatch(/width:0%/);
        }
    });

    it('expands into an AUD sub-row and USD sub-rows', async () => {
        const row = await budgetRow();
        await (await row.$('.//span[starts-with(normalize-space(), "monthly")]')).click();

        const jan = await subRow('2026-01-01');
        await jan.waitForDisplayed();
        const feb = await subRow('2026-02-01');
        await feb.waitForDisplayed();
        await browser
            .waitUntil(async () =>
                (await jan.getText()).includes('120.00') && (await feb.getText()).includes('90.00'))
            .catch(async () => {
                throw new Error(
                    `Sub-rows did not load; Jan: ${await jan.getText()} Feb: ${await feb.getText()}`,
                );
            });

        const amounts = async (start: string): Promise<string[]> =>
            browser.execute(
                (e: HTMLElement) => Array.from(e.querySelectorAll(':scope > span'))
                    .slice(1)
                    .map((s) => s.textContent?.trim() ?? ''),
                (await subRow(start)) as unknown as HTMLElement,
            );
        const [janSpent, janTarget] = await amounts('2026-01-01');
        const [febSpent, febTarget] = await amounts('2026-02-01');

        expect([janSpent, janTarget]).toEqual([
            expect.stringContaining('120.00'),
            expect.stringContaining('200.00'),
        ]);
        expect([febSpent, febTarget]).toEqual([
            expect.stringContaining('90.00'),
            expect.stringContaining('150.00'),
        ]);
        expect(symbolOf(janTarget)).toBe(symbolOf(janSpent));
        expect(symbolOf(febTarget)).toBe(symbolOf(febSpent));
        expect(symbolOf(janTarget)).not.toBe(symbolOf(febTarget));
    });
});
