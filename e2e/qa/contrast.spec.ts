/**
 * APCA contrast floors for the colour tokens in `style/tokens/_colors.scss`.
 *
 * Each pair is rendered as a probe element on the QA index, read back as
 * computed colour, converted to sRGB through a 1×1 canvas (Chromium reports
 * `oklch(...)`), and scored in Node. Floors follow each token's role.
 */
import { expect, test, type Page } from '@playwright/test';
import { calcAPCA } from 'apca-w3';
import { THEMES, requireRoutes, setTheme } from './routes.js';

requireRoutes();

// MARK: Floors

const SURFACES = ['--bc-bg', '--bc-surface', '--bc-surface-alt', '--bc-surface-accent'];
const TONES    = ['accent', 'good', 'bad', 'warn'];
const SYNTAX   = ['--bc-keyword', '--bc-string', '--bc-number', '--bc-type', '--bc-fn'];

type Pair = { fg: string; bg: string; min: number };

const PAIRS: Pair[] = [
  ...SURFACES.map((bg) => ({ fg: '--bc-ink', bg, min: 75 })),
  ...SURFACES.map((bg) => ({ fg: '--bc-ink-soft', bg, min: 60 })),
  ...SURFACES.map((bg) => ({ fg: '--bc-ink-mute', bg, min: 45 })),
  ...SURFACES.map((bg) => ({ fg: '--bc-ink-dim', bg, min: 30 })),
  ...TONES.flatMap((t) => [
    { fg: `--bc-${t}`, bg: '--bc-bg', min: 45 },
    { fg: `--bc-${t}`, bg: `--bc-${t}-soft`, min: 45 },
  ]),
  ...SYNTAX.map((fg) => ({ fg, bg: '--bc-surface-alt', min: 60 })),
  { fg: '--bc-comment', bg: '--bc-surface-alt', min: 30 },
];

/**
 * Pairs below their floor today, keyed `theme:fg:bg`. Each runs as
 * `test.fail`, so raising the token above the floor turns the test red until
 * its entry is removed. See #294.
 */
const KNOWN_LOW = new Set<string>([
  'dark:--bc-accent:--bc-bg',
  'dark:--bc-accent:--bc-accent-soft',
  'dark:--bc-bad:--bc-bg',
  'dark:--bc-bad:--bc-bad-soft',
  'dark:--bc-ink-mute:--bc-bg',
  'dark:--bc-ink-mute:--bc-surface',
  'dark:--bc-ink-mute:--bc-surface-alt',
  'dark:--bc-ink-mute:--bc-surface-accent',
  'dark:--bc-ink-dim:--bc-bg',
  'dark:--bc-ink-dim:--bc-surface',
  'dark:--bc-ink-dim:--bc-surface-alt',
  'dark:--bc-ink-dim:--bc-surface-accent',
  'dark:--bc-keyword:--bc-surface-alt',
  'dark:--bc-string:--bc-surface-alt',
  'dark:--bc-type:--bc-surface-alt',
  'dark:--bc-fn:--bc-surface-alt',
  'dark:--bc-comment:--bc-surface-alt',
]);

// MARK: Measurement

/** Sentinels a probe falls back to when its token is undefined. */
const FG_SENTINEL = 'rgb(1, 2, 3)';
const BG_SENTINEL = 'rgb(4, 5, 6)';

type Measured = { fg: string; bg: string } | { undefinedToken: string };

async function measure(page: Page, pairs: Pair[]): Promise<Measured[]> {
  return page.evaluate(
    ({ pairs, fgSentinel, bgSentinel }) => {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
      const toHex = (css: string): string => {
        ctx.clearRect(0, 0, 1, 1);
        ctx.fillStyle = css;
        ctx.fillRect(0, 0, 1, 1);
        const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
        return `#${[r, g, b].map((n) => n.toString(16).padStart(2, '0')).join('')}`;
      };

      return pairs.map(({ fg, bg }) => {
        const probe = document.createElement('div');
        probe.style.color = `var(${fg}, ${fgSentinel})`;
        probe.style.backgroundColor = `var(${bg}, ${bgSentinel})`;
        document.body.appendChild(probe);
        const cs = getComputedStyle(probe);
        const color = cs.color;
        const background = cs.backgroundColor;
        probe.remove();
        if (color === fgSentinel) return { undefinedToken: fg };
        if (background === bgSentinel) return { undefinedToken: bg };
        return { fg: toHex(color), bg: toHex(background) };
      });
    },
    { pairs, fgSentinel: FG_SENTINEL, bgSentinel: BG_SENTINEL },
  );
}

// MARK: Tests

for (const theme of THEMES) {
  test.describe(`${theme} theme`, () => {
    let results: Measured[] = [];

    test.beforeAll(async ({ browser }) => {
      const page = await browser.newPage();
      await page.goto('/__test');
      await setTheme(page, theme);
      results = await measure(page, PAIRS);
      await page.close();
    });

    PAIRS.forEach((pair, i) => {
      const key = `${theme}:${pair.fg}:${pair.bg}`;
      test(`${theme}: ${pair.fg} on ${pair.bg} ≥ Lc ${pair.min}`, () => {
        const r = results[i];
        // Checked before `test.fail`, so an undefined known-low token still fails.
        if ('undefinedToken' in r) {
          throw new Error(`token undefined: ${r.undefinedToken}`);
        }
        test.fail(KNOWN_LOW.has(key), 'below floor today; see KNOWN_LOW');
        const lc = Math.abs(calcAPCA(r.fg, r.bg));
        expect(lc, `${key} measured Lc ${lc.toFixed(1)} (${r.fg} on ${r.bg})`).toBeGreaterThanOrEqual(pair.min);
      });
    });
  });
}
