/**
 * Flow test proving the per-account dashboard sparkline recomputes against
 * the app-wide global filter: applying a `tag:` token narrows the rendered
 * cash-flow geometry to exactly the tagged postings, and clearing the chip
 * restores the original unfiltered shape.
 *
 * Every assertion runs against a *settled* reading (two consecutive identical
 * non-empty frames) so it cannot be satisfied by the resource's empty in-flight
 * or error frame — a filtered fetch that silently goes unfiltered, or fails
 * outright, must fail this spec.
 *
 * Reuses the ⌘K palette harness (imports, dialog/listbox selectors, token
 * commit flow) and chip helpers from `register-global-filter.spec.ts` /
 * `dashboard-global-filter.spec.ts` verbatim.
 */
import { browser, $, expect } from '@wdio/globals';

// ── Navigation helpers ───────────────────────────────────────────────────────

/** Navigate to Accounts → `name` via the top-bar nav and sidebar. */
async function openAccount(name: string): Promise<void> {
    const navAccounts = await $('[data-testid="nav-accounts"]');
    await navAccounts.waitForDisplayed();
    await navAccounts.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts'),
        { timeoutMsg: 'URL did not reach /accounts within 5 s' },
    );

    const sidebarNav = await $('nav[aria-label="account navigation"]');
    await sidebarNav.$('a').waitForDisplayed();

    const accountSpan = await sidebarNav.$(`span=${name}`);
    await accountSpan.waitForDisplayed();
    await accountSpan.click();

    await browser.waitUntil(
        async () => (await browser.getUrl()).includes('/accounts/'),
        { timeoutMsg: 'URL did not update to account route within 5 s' },
    );

    const balance = await $('[data-testid="dashboard-balance"]');
    await balance.waitForDisplayed();
}

// ── Sparkline helpers ─────────────────────────────────────────────────────

/** One sampled frame of the rendered sparkline. */
interface SparklineReading {
    /** Every shape's `points` attribute joined — the change signal. */
    signature: string;
    /** Income polyline y-coordinates, oldest bucket first (SVG units). */
    incomeY: number[];
    /** Expense polyline y-coordinates, oldest bucket first (SVG units). */
    expenseY: number[];
}

/** Reads the sparkline's rendered geometry: change signature plus both series. */
async function readSparkline(): Promise<SparklineReading> {
    return browser.execute(() => {
        const root = document.querySelector('[data-testid="dashboard-sparkline"]');
        if (!root) return { signature: '', incomeY: [], expenseY: [] };
        const signature = Array.from(root.querySelectorAll('polyline, polygon'))
            .map(el => el.getAttribute('points') ?? '')
            .join('|');
        const ys = (el: Element | undefined): number[] =>
            (el?.getAttribute('points') ?? '')
                .trim()
                .split(/\s+/)
                .filter(Boolean)
                .map(pair => Number(pair.split(',')[1]));
        // The income line is emitted before the expense line (see `Sparkline`).
        const lines = Array.from(root.querySelectorAll('polyline'));
        return { signature, incomeY: ys(lines[0]), expenseY: ys(lines[1]) };
    });
}

/**
 * Whether a reading carries real plotted geometry.
 *
 * The dashboard renders `points=[]` for BOTH the in-flight (`None`) and the
 * error frame of the sparkline `LocalResource`, which still emits the polylines
 * with an empty `points` attribute. Requiring every shape to carry points — and
 * both series to carry the same, plural bucket count — rejects those frames, so
 * a wait built on this predicate can never be satisfied by a loading blip or by
 * a filtered backend path that failed outright.
 */
function hasGeometry(r: SparklineReading): boolean {
    return (
        r.signature !== ''
        && r.signature.split('|').every(seg => seg.trim() !== '')
        && r.incomeY.length >= 2
        && r.incomeY.length === r.expenseY.length
        && [...r.incomeY, ...r.expenseY].every(Number.isFinite)
    );
}

/**
 * Waits for a *settled* sparkline reading satisfying `accept`.
 *
 * "Settled" means two consecutive polls produced identical non-empty geometry,
 * so the returned frame is the resource's resolved output rather than a
 * transient. `accept` is folded into the wait (not asserted afterwards) because
 * the pre-fetch DOM still shows the previous shape, which is itself stable.
 */
async function waitForSettledSparkline(
    accept: (reading: SparklineReading) => boolean,
    timeoutMsg: string,
): Promise<SparklineReading> {
    let previous = '';
    let settled: SparklineReading | undefined;
    await browser.waitUntil(
        async () => {
            const reading = await readSparkline();
            if (!hasGeometry(reading) || !accept(reading)) {
                previous = '';
                return false;
            }
            if (previous === reading.signature) {
                settled = reading;
                return true;
            }
            previous = reading.signature;
            return false;
        },
        { timeout: 15000, interval: 250, timeoutMsg },
    );
    if (!settled) throw new Error(timeoutMsg);
    return settled;
}

/**
 * Converts both series to fractions of the chart's plotted maximum.
 *
 * The component scales income and expenses on one shared y-axis spanning
 * `[global_min, global_max]`, so normalising against the observed y extremes
 * recovers each bucket's value as a fraction of the largest plotted flow —
 * without hard-coding the SVG's padding constants. Both series are
 * non-negative (inflow / |outflow|), so a flat-zero bucket maps to `0` and the
 * largest flow to `1`.
 */
function toFractions(r: SparklineReading): { income: number[]; expenses: number[] } {
    const all = [...r.incomeY, ...r.expenseY];
    const top = Math.min(...all);
    const bottom = Math.max(...all);
    const range = bottom - top;
    const frac = (y: number): number => (range === 0 ? 0 : (bottom - y) / range);
    return { income: r.incomeY.map(frac), expenses: r.expenseY.map(frac) };
}

/** Buckets carrying a materially non-zero flow, as `[index, fraction]` pairs. */
function nonZeroBuckets(values: number[]): [number, number][] {
    return values
        .map((v, i): [number, number] => [i, v])
        .filter(([, v]) => v > 0.02);
}

/** Waits until the dashboard sparkline wrapper is rendered. */
async function waitForSparkline(): Promise<void> {
    const el = await $('[data-testid="dashboard-sparkline"]');
    await el.waitForDisplayed();
}

// ── Palette helpers (mirrors palette-filter-builder.spec.ts) ────────────────

/** Opens the ⌘K palette and returns the dialog element. */
async function openPalette() {
    const openButton = await $('button[aria-label="open command palette (⌘K)"]');
    await openButton.click();

    const dialog = await $('div[role="dialog"][aria-label="Command palette"]');
    await expect(dialog).toBeDisplayed();
    return dialog;
}

/**
 * Types `tag:<tagName>` into the palette's single inline search box, narrows
 * to the sole matching option, and commits it as a chip. Closes the palette
 * afterwards — chips sit behind the z-900 palette overlay, so the overlay
 * must be dismissed before any other top-bar interaction (chip removal).
 */
async function commitTagToken(tagName: string): Promise<void> {
    const dialog = await openPalette();

    const input = await dialog.$('input[role="combobox"]');
    await input.waitForDisplayed();
    await input.setValue(`tag:${tagName}`);

    const listbox = await $('#palette-listbox');
    await browser.waitUntil(
        async () => (await listbox.$$('div[role="option"]').length) === 1,
        { timeoutMsg: `expected the tag search to narrow to \`${tagName}\`` },
    );
    const only = await listbox.$('div[role="option"]');
    expect(await only.getAttribute('textContent')).toContain(tagName);
    await only.click();

    await browser.waitUntil(async () => (await input.getValue()) === '', {
        timeoutMsg: 'expected the input to clear after committing the token',
    });

    await browser.keys('Escape');
    await expect(dialog).not.toBeDisplayed();
}

// ── Filter chip helpers ──────────────────────────────────────────────────────

/** Clicks a chip's ✕ by its exact label (e.g. `"tag: recurring"`). */
async function removeChip(label: string): Promise<void> {
    const btn = await $(`[data-testid="filter-chips"] button[aria-label="remove ${label} filter"]`);
    await btn.waitForDisplayed();
    await btn.click();
}

/** Number of remove buttons currently rendered inside the chip strip. */
async function chipButtonCount(): Promise<number> {
    return browser.execute(
        () => document.querySelectorAll('[data-testid="filter-chips"] button').length,
    );
}

/**
 * Clears any filter chips left over from a spec that ran earlier in the same
 * suite (specs share one long-lived app session — see `wdio.conf.ts`; the
 * global filter store is provided once at the shell root and outlives route
 * changes). Keeps this spec self-contained regardless of run order.
 */
async function clearAllChips(): Promise<void> {
    for (let i = 0; i < 10; i += 1) {
        const count = await chipButtonCount();
        if (count === 0) return;
        const btn = await $('[data-testid="filter-chips"] button');
        await btn.click();
        await browser.waitUntil(
            async () => (await chipButtonCount()) < count,
            { timeoutMsg: 'chip removal did not reduce the chip count' },
        );
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

describe('Account dashboard — sparkline global filter', () => {
    it('re-renders the sparkline under a tag filter and restores on clear', async () => {
        await browser.execute(() => {
            window.history.pushState({}, '', '/');
            window.dispatchEvent(new PopStateEvent('popstate', { state: null }));
        });
        await clearAllChips();

        // 1-2. Select CreditCard and capture the settled unfiltered geometry.
        // The seed gives CreditCard current-month `recurring`-tagged activity
        // (an 80.00 membership charge and a 15.00 refund, both inside the
        // sparkline's default trailing window) alongside many other unrelated
        // current-month postings, so narrowing to `tag:recurring` collapses the
        // chart to just those two flows.
        await openAccount('CreditCard');
        await waitForSparkline();
        const before = await waitForSettledSparkline(
            () => true,
            'Sparkline never settled on an unfiltered baseline shape',
        );

        // The unfiltered CreditCard chart spreads spending over several
        // buckets — a precondition for the filtered shape below being a
        // genuine narrowing rather than a coincidence.
        expect(nonZeroBuckets(toFractions(before).expenses).length).toBeGreaterThan(1);

        // 3. Commit `tag:recurring` — the sparkline resource re-fetches against
        // the filter's membership set. Wait for a *settled, non-empty* geometry
        // that differs from the baseline: the resource's in-flight and error
        // frames both render empty polylines, which `hasGeometry` rejects, so
        // this cannot pass on a transient or on a failed filtered fetch.
        await commitTagToken('recurring');

        const chips = await $('[data-testid="filter-chips"]');
        await expect(chips).toBeDisplayed();
        expect(await chips.getText()).toContain('tag: recurring');

        const filtered = await waitForSettledSparkline(
            r => r.signature !== before.signature,
            'Sparkline geometry did not settle on a new shape after applying tag:recurring',
        );
        expect(filtered.signature).not.toBe(before.signature);

        // The filtered chart must be exactly the two seeded `recurring` legs:
        // one bucket of inflow (the 15.00 refund), one bucket of outflow (the
        // 80.00 charge), every other bucket flat at zero. Both counts and the
        // 15/80 ratio are clock-independent — the seed dates are relative to
        // the current month, and the ratio is recovered from the shared y-axis
        // rather than from any absolute date or bucket index.
        const fracs = toFractions(filtered);
        expect(filtered.incomeY.length).toBe(before.incomeY.length);
        const incomeBuckets = nonZeroBuckets(fracs.income);
        const expenseBuckets = nonZeroBuckets(fracs.expenses);
        expect(incomeBuckets.length).toBe(1);
        expect(expenseBuckets.length).toBe(1);
        // The 80.00 outflow is the largest plotted flow, so it normalises to 1;
        // the 15.00 inflow must sit at 15/80 of it.
        expect(expenseBuckets[0][1]).toBeCloseTo(1, 2);
        expect(incomeBuckets[0][1]).toBeCloseTo(15 / 80, 2);

        // 4. Remove the chip — the sparkline resource re-fetches unfiltered and
        // must settle back on the original baseline geometry (again ignoring
        // the empty in-flight frame).
        await removeChip('tag: recurring');
        await expect($('[data-testid="filter-chips"]')).not.toBeDisplayed();

        const restored = await waitForSettledSparkline(
            r => r.signature === before.signature,
            'Sparkline geometry did not settle back on the unfiltered baseline',
        );
        expect(restored.signature).toBe(before.signature);
    });
});
