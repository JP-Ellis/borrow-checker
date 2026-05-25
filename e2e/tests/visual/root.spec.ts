/**
 * Design-token existence and APCA contrast tests.
 *
 * Migrated from the Playwright web suite.  Tests run against the real Tauri
 * binary on WebKitGTK — a stricter environment than the previous Chromium target.
 *
 * Colour scheme is forced via data-theme="light|dark" so results are
 * independent of the host OS / container GTK theme.
 */
import { browser, $ } from '@wdio/globals';
import { calcAPCA } from 'apca-w3';

/** Resolve a CSS colour string to an sRGB hex value via a 1×1 canvas. */
async function toHex(cssColour: string): Promise<string> {
    return browser.execute((colour) => {
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 1;
        const ctx = canvas.getContext('2d')!;
        ctx.fillStyle = colour;
        ctx.fillRect(0, 0, 1, 1);
        const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
        return `#${[r, g, b].map(n => n.toString(16).padStart(2, '0')).join('')}`;
    }, cssColour);
}

const EXPECTED_TOKENS = [
    '--bc-ink',
    '--bc-bg',
    '--bc-surface',
    '--bc-good',
    '--bc-bad',
    '--bc-warn',
    '--bc-font-mono',
    '--bc-font-sans',
] as const;

describe('Design tokens', () => {
    before(async () => {
        /* Navigate to the dashboard so global styles are loaded. */
        const nav  = await $('nav[aria-label="main navigation"]');
        const link = await nav.$('a=dashboard');
        await link.click();
        /* Wait for main to appear (confirms the route rendered). */
        await (await $('main')).waitForDisplayed({ timeout: 5_000 });
    });

    for (const token of EXPECTED_TOKENS) {
        it(`${token} is defined`, async () => {
            const value = await browser.execute(
                (t: string) =>
                    getComputedStyle(document.documentElement).getPropertyValue(t).trim(),
                token,
            );
            expect(value).not.toBe('');
        });
    }
});

describe('APCA contrast (WCAG 3 draft)', () => {
    for (const scheme of ['light', 'dark'] as const) {
        it(`ink-on-bg meets Lc ≥ 60 in ${scheme} mode`, async () => {
            /* Force the colour scheme independently of the host GTK theme. */
            await browser.execute((s: string) => {
                document.documentElement.setAttribute('data-theme', s);
            }, scheme);

            const { ink, bg } = await browser.execute(() => {
                const s = getComputedStyle(document.documentElement);
                return {
                    ink: s.getPropertyValue('--bc-ink').trim(),
                    bg:  s.getPropertyValue('--bc-bg').trim(),
                };
            });

            const hexInk = await toHex(ink);
            const hexBg  = await toHex(bg);
            const lc     = Math.abs(calcAPCA(hexInk, hexBg));
            expect(lc).toBeGreaterThanOrEqual(60);
        });
    }

    after(async () => {
        await browser.execute(() => {
            document.documentElement.removeAttribute('data-theme');
        });
    });
});
