/**
 * The accounts page and top bar across viewport widths: what stays in flow,
 * which tabs and register columns survive, and what holds still on scroll.
 * Every test only reads the seed.
 */
import { expect, test, type Page } from '@playwright/test';

// A Rust panic in the WASM app surfaces as a page error; any one fails the test.
let pageErrors: Error[] = [];
test.beforeEach(({ page }) => {
  pageErrors = [];
  page.on('pageerror', (e) => pageErrors.push(e));
});
test.afterEach(() => {
  expect(pageErrors).toEqual([]);
});

function sidebar(page: Page) {
  return page.getByRole('navigation', { name: 'account navigation' }).first();
}

function register(page: Page) {
  return page.getByLabel('transaction register');
}

/** Opens the accounts page on `name`, expanding `parents` in the sidebar first. */
async function openAccount(page: Page, name: string, parents: string[] = []): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  for (const parent of parents) {
    const toggle = sidebar(page).getByRole('button', { name: `toggle ${parent}` });
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
      await toggle.click();
    }
  }
  await sidebar(page).getByText(name, { exact: true }).click();
  await expect(page.getByLabel('account dashboard').getByText(name, { exact: true })).toBeVisible();
}

// MARK: Top bar

test('the top bar shows no status pill', async ({ page }) => {
  await page.goto('/');
  const topBar = page.getByRole('banner');
  await expect(topBar).toBeVisible();
  await expect(topBar).not.toContainText('pending');
});

test('at 1280 px every tab is inline and the overflow button is hidden', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('/');
  await expect(page.getByTestId('nav-more')).toBeHidden();
  await page.getByTestId('nav-settings').click();
  await expect(page).toHaveURL(/\/settings$/);
});

test('at 400 px settings is reachable through the overflow menu', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  await expect(page.getByTestId('nav-accounts')).toBeVisible();
  await page.getByTestId('nav-more').click();
  await page.getByTestId('nav-more-settings').click();
  await expect(page).toHaveURL(/\/settings$/);
  // Choosing an item closes the menu; the button marks the active overflow route.
  await expect(page.locator('#bc-nav-more')).toBeHidden();
  const more = page.getByTestId('nav-more');
  await expect(more).toHaveClass(/top-bar__tab--active/);
  // The active style paints an accent underline and a filled background.
  await expect(more).not.toHaveCSS('border-bottom-color', 'rgba(0, 0, 0, 0)');
  await expect(more).toHaveCSS('border-bottom-width', '2px');
  await expect(more).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
});

test('at 400 px the top bar fits without horizontal overflow', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  const fits = await page.getByRole('banner').evaluate((el) => el.scrollWidth <= el.clientWidth);
  expect(fits).toBe(true);
  await expect(page.getByRole('button', { name: /open command palette/ })).toBeVisible();
});

test('at 400 px active filters fold into one chip that opens them', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  await expect(page.getByTestId('filter-more')).toHaveCount(0);
  await page.getByRole('button', { name: /open command palette/ }).click();
  await page.keyboard.type('status:unreconciled');
  await page.locator('#palette-listbox div[role="option"]').first().waitFor();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Escape');
  // The chip shows only the count so it fits at 360 px; its accessible name spells it out.
  await expect(page.getByTestId('filter-more')).toHaveText('1 ▾');
  await expect(page.getByRole('button', { name: '1 filter' })).toBeVisible();
  await page.getByTestId('filter-more').click();
  await expect(page.locator('#bc-filter-more')).toContainText('status: unreconciled');
});

for (const width of [360, 400, 768, 1024]) {
  for (const filtered of [false, true]) {
    test(`at ${width} px the top bar fits and nothing paints over a tab${filtered ? ' with a filter active' : ''}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 800 });
      await page.goto('/');
      if (filtered) {
        await page.getByRole('button', { name: /open command palette/ }).click();
        await page.keyboard.type('status:unreconciled');
        await page.locator('#palette-listbox div[role="option"]').first().waitFor();
        await page.keyboard.press('Enter');
        await page.keyboard.press('Escape');
        await expect(page.getByRole('banner').getByText('status: unreconciled').first()).toBeAttached();
      }
      const result = await page.getByRole('banner').evaluate((bar) => {
        const visible = (el: Element) => {
          const r = el.getBoundingClientRect();
          return r.width > 0 && r.height > 0;
        };
        const targets = [
          ...bar.querySelectorAll('[data-testid^="nav-"]'),
          ...bar.querySelectorAll('button[aria-label^="open command palette"]'),
        ].filter(visible);
        const covered = targets
          .filter((el) => {
            const r = el.getBoundingClientRect();
            const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
            return !hit || !(hit === el || el.contains(hit));
          })
          .map((el) => el.getAttribute('data-testid') ?? el.getAttribute('aria-label'));
        return {
          scrollWidth: bar.scrollWidth,
          clientWidth: bar.clientWidth,
          covered,
          checked: targets.length,
        };
      });
      expect(result.checked).toBeGreaterThan(3);
      expect(result.scrollWidth).toBeLessThanOrEqual(result.clientWidth);
      expect(result.clientWidth).toBeLessThanOrEqual(width);
      expect(result.covered).toEqual([]);
    });
  }
}

// MARK: Account bar

test('the account bar is shown before any scroll', async ({ page }) => {
  await openAccount(page, 'Checking');
  await expect(page.getByTestId('account-bar')).toBeVisible();
  await expect(page.getByTestId('account-path')).toContainText('Checking');
  await expect(page.getByTestId('add-tx')).toBeVisible();
  // The accessible name carries the visible label, so voice control finds it.
  await expect(page.getByTestId('add-tx')).toHaveAccessibleName(/^\+ tx/);
});

test('the register holds still after a 200 px scroll', async ({ page }) => {
  await openAccount(page, 'Checking');
  const main = page.getByTestId('accounts-main-scroll');
  const row = register(page).locator('[data-tx-id]').first();
  await expect(row).toBeVisible();
  // Chromium's scroll anchoring would absorb an inserted bar into scrollTop.
  await main.evaluate((el) => {
    el.style.overflowAnchor = 'none';
    el.scrollTop = 200;
    el.dispatchEvent(new Event('scroll'));
  });
  const read = () =>
    Promise.all([
      main.evaluate((el) => el.scrollTop),
      row.evaluate((el) => el.getBoundingClientRect().top),
    ]);
  const [scrollBefore, topBefore] = await read();
  expect(scrollBefore).toBeGreaterThan(180);
  await page.waitForTimeout(300);
  const [scrollAfter, topAfter] = await read();
  expect(scrollAfter).toBe(scrollBefore);
  expect(topAfter).toBe(topBefore);
});

test('with no account selected the bar is absent', async ({ page }) => {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  await expect(page.getByText('// select an account from the sidebar')).toBeVisible();
  await expect(page.getByTestId('account-bar')).toBeHidden();
});

test('at 400 px a nested account keeps its leaf name in view', async ({ page }) => {
  await page.setViewportSize({ width: 400, height: 800 });
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  // At this width the tree lives in the drawer behind the rail button.
  await page.getByRole('button', { name: 'Open account navigation' }).click();
  const drawer = page.locator('#bc-sidebar-drawer');
  const toggle = drawer.getByRole('button', { name: 'toggle Expenses' });
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') {
    await toggle.click();
  }
  await drawer.getByText('Subscriptions', { exact: true }).click();
  const path = page.getByTestId('account-path');
  await expect(path).toHaveAttribute('title', /Subscriptions$/);
  const box = await path.boundingBox();
  expect(box?.height).toBeLessThanOrEqual(40);
  // The leaf's last character renders inside the bar's visible box.
  const leafVisible = await path.evaluate((el) => {
    const range = document.createRange();
    const text = el.firstElementChild!.firstChild!;
    range.setStart(text, text.textContent!.length - 1);
    range.setEnd(text, text.textContent!.length);
    const r = range.getBoundingClientRect();
    const b = el.getBoundingClientRect();
    return r.right <= b.right + 1 && r.left >= b.left - 1;
  });
  expect(leafVisible).toBe(true);
});

// MARK: Register

/** Opens the accounts page on `name` through the drawer, for widths below the sidebar's inline breakpoint. */
async function openAccountInDrawer(page: Page, name: string): Promise<void> {
  await page.goto('/');
  await page.getByTestId('nav-accounts').click();
  await page.getByRole('button', { name: 'Open account navigation' }).click();
  await page.locator('#bc-sidebar-drawer').getByText(name, { exact: true }).click();
  await expect(page.getByLabel('account dashboard').getByText(name, { exact: true })).toBeVisible();
}

/** Asserts every visible register row renders a non-empty date, payee and amount. */
async function expectCoreCells(page: Page): Promise<void> {
  const rows = register(page).locator('[data-tx-id]');
  await expect(rows.first()).toBeVisible();
  const widths = await rows.evaluateAll((els) =>
    els.slice(0, 10).map((el) => {
      const w = (sel: string) => el.querySelector(sel)?.getBoundingClientRect().width ?? 0;
      return { date: w('[class*="date"]'), payee: w('[class*="payee_cell"]'), amount: w('[class*="amount"]') };
    }),
  );
  for (const w of widths) {
    expect(w.date).toBeGreaterThan(0);
    expect(w.payee).toBeGreaterThan(0);
    expect(w.amount).toBeGreaterThan(0);
  }
}

for (const width of [400, 820, 1280]) {
  test(`at ${width} px every row shows date, payee and amount`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    if (width < 480) {
      await openAccountInDrawer(page, 'Checking');
    } else {
      await openAccount(page, 'Checking');
    }
    await expectCoreCells(page);
  });
}

test('at 820 px a tagged row keeps at least 12ch of payee', async ({ page }) => {
  await page.setViewportSize({ width: 820, height: 900 });
  await openAccount(page, 'Subscriptions', ['Expenses']);
  const row = register(page).locator('[data-tx-id]').filter({ hasText: 'Netflix' }).first();
  await expect(row).toBeVisible();
  const ok = await row.evaluate((el) => {
    const payee = el.querySelector('[class*="payee_cell"] [class*="payee"]') as HTMLElement;
    // Computed min-width resolves `12ch` to pixels in the payee's own font.
    const floor = parseFloat(getComputedStyle(payee).minWidth);
    return floor > 0 && payee.getBoundingClientRect().width >= floor - 0.5;
  });
  expect(ok).toBe(true);
});

test('the +N chip titles every tag of the row', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAccount(page, 'Subscriptions', ['Expenses']);
  // Only the seeded Netflix row with two tags renders a `+N` chip.
  const row = register(page)
    .locator('[data-tx-id]')
    .filter({ hasText: 'Netflix' })
    .filter({ has: page.locator('[class*="tag_more"]') });
  await expect(row).toHaveCount(1);
  await expect(row.locator('[class*="tag_more"]')).toHaveAttribute('title', 'recurring, subscription');
});

test('header and row columns line up with the balance column on and off', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAccount(page, 'Checking');
  const tracks = () =>
    register(page).evaluate((el) => {
      const cols = (n: Element | null) => (n ? getComputedStyle(n).gridTemplateColumns : '');
      return [cols(el.querySelector('[class*="col_headers"]')), cols(el.querySelector('[data-tx-id]'))];
    });
  const [headOn, rowOn] = await tracks();
  expect(headOn).toBe(rowOn);
  // The mode button cycles; click until the register hides the balance track.
  for (let i = 0; i < 4 && (await register(page).getAttribute('data-balance')) !== 'hidden'; i++) {
    await page.getByTestId('balance-mode').click();
  }
  await expect(register(page)).toHaveAttribute('data-balance', 'hidden');
  const [headOff, rowOff] = await tracks();
  expect(headOff).toBe(rowOff);
  expect(headOff).not.toBe(headOn);
});

test('the budget detail keeps its full row at desktop width', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('/');
  await page.getByTestId('nav-budget').click();
  const tree = page.getByLabel('budget tree');
  await expect(tree.getByText('Groceries', { exact: true }).first()).toBeVisible();
  const row = page.locator('[data-tx-id]').first();
  if ((await row.count()) === 0) {
    // Open a budget row so its transactions render.
    await tree.getByText('Groceries', { exact: true }).first().click();
  }
  await expect(row).toBeVisible();
  const area = await row.evaluate((el) => getComputedStyle(el).gridTemplateAreas);
  expect(area).toBe('none');
});
