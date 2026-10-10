/**
 * Import and discard paths the real csv plugin cannot reach: every row fate,
 * paging, a failed run that wrote, blocked and racing discards. Every import
 * command is answered by `page.route`; the database is never written. Bodies
 * follow the serde shapes pinned by bc-ipc's import snapshots.
 */
import { expect, test, type Page } from '@playwright/test';

const COUNTS = {
  removed_postings: 0, removed_transactions: 0, detached_adopted: 0, freed_tombstones: 0,
  other_batch_references_removed: 0, other_batch_references_tombstoned: 0, edited_postings: 0,
  reconciled_postings: 0, flagged_postings: 0, removed_tags: 0, kept_tags: 0,
  removed_accounts: 0, kept_accounts: 0, reverted_fields: 0,
};

const BATCH = {
  id: 'batch-0001', profile: 'mock-bank', importer: 'csv',
  started_at: '2026-10-10T01:00:00Z', finished_at: '2026-10-10T01:00:05Z',
  state: { kind: 'complete' },
  counts: { new_transactions: 4, attached_postings: 0, skipped_postings: 0 },
  discard: null,
};

const LEG = { account: 'Assets:Mock Bank:Checking', amount: { value: '-12.00', currency_code: 'AUD' }, fate: { kind: 'new' } };

function row(n: number, fate: object, extra: object = {}): object {
  return {
    location: `statement.csv data row ${n}`, date: '2026-01-05', description: `Woolworths Metro ${n}`,
    fate, legs: [LEG], diagnostics: [], ...extra,
  };
}

function previewBody(rows: object[], extra: object = {}): object {
  return {
    profile: 'mock-bank', fingerprint: '00000000000000aa',
    new_transactions: 0, attached_postings: 0, already_imported: 0, skipped_postings: 0,
    unresolved_accounts: [], unresolved_commodities: [], skips_by_cause: [], warnings: [],
    would_create_accounts: [], would_create_tags: [], account_totals: [], rows,
    other_diagnostics: [], ...extra,
  };
}

async function serveProfiles(page: Page, documentsRootSet = true): Promise<void> {
  await page.route('**/rpc/list_import_profiles', (route) => route.fulfill({ json: {
    documents_root_set: documentsRootSet,
    profiles: [
      { name: 'mock-bank', importer: 'csv', installed: true, config_text: '{\n  "account": "123456789"\n}' },
      { name: 'mock-card', importer: 'ofx', installed: false, config_text: '' },
    ],
  } }));
}

/** Serves `list_import_batches` from `batches()`, read on every call. */
async function serveBatches(page: Page, batches: () => object[]): Promise<void> {
  await page.route('**/rpc/list_import_batches', (route) => route.fulfill({ json: batches() }));
}

test('a missing importer and an unset documents root disable Preview', async ({ page }) => {
  await serveProfiles(page, false);
  await serveBatches(page, () => []);
  await page.goto('/import');
  await expect(page.getByTestId('import-root-banner')).toContainText('import.documents-root');
  for (const name of ['mock-bank', 'mock-card']) {
    await expect(page.getByTestId(`import-profile-row-${name}`).getByRole('button', { name: 'Preview' })).toBeDisabled();
  }

  await page.unroute('**/rpc/list_import_profiles');
  await serveProfiles(page, true);
  await page.reload();
  await expect(page.getByTestId('import-root-banner')).toHaveCount(0);
  const card = page.getByTestId('import-profile-row-mock-card');
  await expect(card.getByRole('button', { name: 'Preview' })).toBeDisabled();
  await expect(card).toContainText('importer `ofx` not installed');
});

test('every fate renders, the default filter hides already imported, rows page by 200', async ({ page }) => {
  const rows = [
    row(1, { kind: 'attach', owner: 'tx-0001' }),
    row(2, { kind: 'already_imported', owner: 'tx-0002' }),
    row(3, { kind: 'conflict', owners: ['tx-0003', 'tx-0004'] }),
    row(4, { kind: 'skipped', cause: 'unresolved account' }, {
      legs: [{ ...LEG, account: 'Assets:Mock Bank:Savings', fate: { kind: 'skipped', cause: 'unresolved account' } }],
      diagnostics: [{ location: 'statement.csv data row 4', cause: 'unresolved account', detail: 'Assets:Mock Bank:Savings names no account' }],
    }),
    ...Array.from({ length: 246 }, (_, i) => row(i + 5, { kind: 'create' })),
  ];
  await serveProfiles(page);
  await serveBatches(page, () => []);
  await page.route('**/rpc/preview_import', (route) => route.fulfill({ json: { kind: 'ready', ...previewBody(rows, {
    new_transactions: 246, attached_postings: 1, already_imported: 1, skipped_postings: 2,
    unresolved_accounts: [{ name: 'Assets:Mock Bank:Savings', postings: 1 }],
    unresolved_commodities: [{ name: 'XYZ', postings: 1 }],
    skips_by_cause: [{ cause: 'unresolved account', postings: 1 }, { cause: 'unregistered commodity', postings: 1 }],
    account_totals: [{ account: 'Assets:Mock Bank:Checking', amounts: [{ value: '-42.50', currency_code: 'AUD' }] }],
    other_diagnostics: [{ location: 'statement.csv header', cause: 'ignored column', detail: 'Balance is not mapped' }],
  }) } }));

  await page.goto('/import?profile=mock-bank');
  const preview = page.getByTestId('import-preview');
  const shown = preview.getByRole('table', { name: 'parsed rows' }).locator('tbody tr[data-fate]');
  await expect(preview).toContainText('246 new · 1 attach leg · 1 already imported · 2 legs skipped');
  await expect(page.getByTestId('import-commit')).toHaveText('Commit: 246 new, 1 attached');
  await expect(preview).toContainText('2 legs will be skipped: 1 unresolved account, 1 unregistered commodity.');
  await expect(preview.getByRole('region', { name: 'would post' })).toContainText('Assets:Mock Bank:Checking');
  await expect(preview.getByRole('region', { name: 'other diagnostics' })).toContainText(
    'statement.csv header: ignored column: Balance is not mapped',
  );

  await expect(page.getByTestId('import-fate-create')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('import-fate-already_imported')).toHaveAttribute('aria-pressed', 'false');
  await expect(shown).toHaveCount(200);
  await expect(preview.locator('tr[data-fate="attach"]')).toContainText('attach');
  await expect(preview.locator('tr[data-fate="conflict"]')).toContainText('conflict: 2 owners');
  await expect(preview.locator('tr[data-fate="skipped"]')).toContainText('skipped: unresolved account');
  await expect(preview).toContainText('unresolved account: Assets:Mock Bank:Savings names no account');

  await page.getByTestId('import-rows-more').click();
  await expect(shown).toHaveCount(249);
  await expect(page.getByTestId('import-rows-more')).toHaveCount(0);

  // A chip toggle returns to the first page.
  await page.getByTestId('import-fate-already_imported').click();
  await expect(shown).toHaveCount(200);
  await expect(page.getByTestId('import-rows-more')).toHaveText('Show more (50 hidden)');
});

test('an unregistered commodity links to Settings → Currencies', async ({ page }) => {
  await serveProfiles(page);
  await serveBatches(page, () => []);
  await page.route('**/rpc/preview_import', (route) => route.fulfill({ json: { kind: 'ready', ...previewBody(
    [row(1, { kind: 'skipped', cause: 'unregistered commodity' })],
    { skipped_postings: 1, unresolved_commodities: [{ name: 'XYZ', postings: 1 }],
      skips_by_cause: [{ cause: 'unregistered commodity', postings: 1 }] },
  ) } }));
  await page.goto('/import?profile=mock-bank');
  await page.getByTestId('import-blockers').getByRole('link', { name: /Currencies/ }).click();
  await expect(page).toHaveURL(/\/settings\?section=currencies$/);
  await expect(page.getByTestId('settings-nav-currencies')).toHaveAttribute('aria-current', 'page');
});

test('a run that failed after writing links to its batch with discard armed', async ({ page }) => {
  const incomplete = { ...BATCH, finished_at: null, state: { kind: 'incomplete' }, counts: null };
  await serveProfiles(page);
  await serveBatches(page, () => [incomplete]);
  await page.route('**/rpc/preview_import', (route) => route.fulfill({ json: { kind: 'ready', ...previewBody(
    [row(1, { kind: 'create' })], { new_transactions: 1 },
  ) } }));
  await page.route('**/rpc/commit_import', (route) => route.fulfill({ json: {
    kind: 'failed', stage: { kind: 'engine' }, message: 'disk full', batch_id: 'batch-0001',
  } }));
  await page.route('**/rpc/preview_discard', (route) => route.fulfill({ json: {
    kind: 'ready', counts: { ...COUNTS, removed_postings: 1, removed_transactions: 1 }, snapshot_planned: true,
  } }));

  await page.goto('/import?profile=mock-bank');
  await page.getByTestId('import-commit').click();
  const failure = page.getByTestId('import-failure');
  await expect(failure).toContainText('Import failed at the engine stage: disk full');
  await expect(failure).toContainText('The run stopped after writing');
  await page.getByTestId('import-failure-history').click();
  await expect(page).toHaveURL(/\/import\?batch=batch-0001&discard=1$/);
  const batch = page.getByTestId('import-batch-row-batch-0001');
  await expect(batch).toHaveAttribute('data-highlighted', 'true');
  await expect(batch).toContainText('incomplete');
  await expect(batch).toContainText('Run stopped before finishing; rows already written stay until discarded');
  await expect(batch.getByTestId('discard-consequences')).toContainText('Removes 1 posting across 1 transaction.');
});

test('a blocked discard lists the later batches; a confirm-time block falls back to it', async ({ page }) => {
  const later = { ...BATCH, id: 'batch-0002', started_at: '2026-10-11T09:30:00Z' };
  await serveProfiles(page);
  await serveBatches(page, () => [later, BATCH]);
  let previews = 0;
  await page.route('**/rpc/preview_discard', (route) => {
    previews += 1;
    return route.fulfill({ json: previews === 1
      ? { kind: 'ready', counts: { ...COUNTS, removed_postings: 4, removed_transactions: 4 }, snapshot_planned: true }
      : { kind: 'blocked', dependants: [{ batch_id: 'batch-0002', importer: 'csv', started_at: '2026-10-11T09:30:00Z', postings: 3, transactions: 2 }] } });
  });
  await page.route('**/rpc/discard_batch', (route) =>
    route.fulfill({ status: 409, json: { Conflict: 'later batches own legs on this batch' } }));

  await page.goto('/import');
  const batch = page.getByTestId('import-batch-row-batch-0001');
  await batch.getByTestId('discard-arm').click();
  await batch.getByTestId('discard-confirm').click();
  const blocked = batch.getByTestId('discard-blocked');
  await expect(blocked).toContainText("Later batches own legs on this batch's transactions. Discard them first.");
  await expect(blocked).toContainText('3 postings on 2 transactions');
  await expect(batch.getByTestId('discard-confirm')).toHaveCount(0);
  await blocked.getByRole('link').click();
  await expect(page).toHaveURL(/\/import\?batch=batch-0002$/);
  await expect(page.getByTestId('import-batch-row-batch-0002')).toHaveAttribute('data-highlighted', 'true');
});

test('edits warn, a disabled snapshot warns, and moved counts are flagged after confirm', async ({ page }) => {
  let discarded = false;
  const info = {
    counts: { ...COUNTS, removed_postings: 5, removed_transactions: 4, edited_postings: 2 },
    discarded_at: '2026-10-10T02:00:00Z',
    snapshot: '/backups/ledger_x/20261010-020000000.pre-discard.sqlite',
  };
  await serveProfiles(page);
  await serveBatches(page, () => [discarded ? { ...BATCH, state: { kind: 'discarded' }, discard: info } : BATCH]);
  await page.route('**/rpc/preview_discard', (route) => route.fulfill({ json: {
    kind: 'ready', counts: { ...COUNTS, removed_postings: 4, removed_transactions: 4, edited_postings: 2 }, snapshot_planned: false,
  } }));
  await page.route('**/rpc/discard_batch', (route) => {
    discarded = true;
    return route.fulfill({ json: info });
  });

  await page.goto('/import');
  const batch = page.getByTestId('import-batch-row-batch-0001');
  await batch.getByTestId('discard-arm').click();
  const consequences = batch.getByTestId('discard-consequences');
  await expect(consequences.locator('li').first()).toHaveText('2 postings you edited will be lost.');
  await expect(consequences).toContainText('No snapshot will be taken (`backup.auto-pre-discard` is off). This cannot be reversed.');
  await batch.getByTestId('discard-confirm').click();
  await expect(batch.getByTestId('discard-counts-changed')).toHaveText('Counts changed since the preview.');
  await expect(batch).toContainText('Removed 5 postings across 4 transactions.');
  await expect(batch.getByTestId('restore-snapshot-link')).toHaveAttribute(
    'href', '/settings?section=backup&backup=%2Fbackups%2Fledger_x%2F20261010-020000000.pre-discard.sqlite',
  );
});

test('a batch already discarded elsewhere refetches History with a notice', async ({ page }) => {
  let discarded = false;
  await serveProfiles(page);
  await serveBatches(page, () => [discarded
    ? { ...BATCH, state: { kind: 'discarded' }, discard: { counts: COUNTS, discarded_at: '2026-10-10T02:00:00Z', snapshot: null } }
    : BATCH]);
  await page.route('**/rpc/preview_discard', (route) => route.fulfill({ json: {
    kind: 'ready', counts: { ...COUNTS, removed_postings: 4, removed_transactions: 4 }, snapshot_planned: true,
  } }));
  await page.route('**/rpc/discard_batch', (route) => {
    discarded = true;
    return route.fulfill({ status: 422, json: { Validation: 'invalid input: import batch batch-0001 has already been discarded' } });
  });

  await page.goto('/import');
  const batch = page.getByTestId('import-batch-row-batch-0001');
  await batch.getByTestId('discard-arm').click();
  await batch.getByTestId('discard-confirm').click();
  await expect(page.getByText('That batch was already discarded. History is up to date.')).toBeVisible();
  await expect(batch).toContainText('discarded');
  await expect(batch.getByTestId('discard-arm')).toHaveCount(0);
});

test('a commit after a ?batch= visit keeps its outcome in view', async ({ page }) => {
  // Forty batches put the highlighted batch-0001 well below the profiles.
  const older = Array.from({ length: 40 }, (_, i) => ({ ...BATCH, id: `batch-${String(40 - i).padStart(4, '0')}` }));
  const fresh = { ...BATCH, id: 'batch-0041' };
  let committed = false;
  await serveProfiles(page);
  await serveBatches(page, () => (committed ? [fresh, ...older] : older));
  await page.route('**/rpc/preview_import', (route) => route.fulfill({ json: { kind: 'ready', ...previewBody(
    [row(1, { kind: 'create' })], { new_transactions: 1 },
  ) } }));
  await page.route('**/rpc/commit_import', (route) => {
    committed = true;
    return route.fulfill({ json: {
      kind: 'imported', batch_id: 'batch-0041', new_transactions: 1, attached_postings: 0,
      skipped_postings: 0, skips_by_cause: [], unresolved_accounts: [], unresolved_commodities: [],
      created_tags: [], created_accounts: [], warnings: [], snapshot: null,
    } });
  });

  await page.goto('/import?batch=batch-0001');
  await expect(page.getByTestId('import-batch-row-batch-0001')).toBeInViewport();
  const profile = page.getByTestId('import-profile-row-mock-bank');
  await profile.scrollIntoViewIfNeeded();
  await profile.getByRole('button', { name: 'Preview' }).click();
  await page.getByTestId('import-commit').click();
  // The refetched History has remounted every row once the new batch shows.
  await expect(page.getByTestId('import-batch-row-batch-0041')).toBeAttached();
  await expect(page.getByTestId('import-outcome')).toBeInViewport();
  await expect(page.getByTestId('import-batch-row-batch-0001')).not.toBeInViewport();
});

test('a link to a pruned snapshot says retention removed it', async ({ page }) => {
  await page.route('**/rpc/list_backups', (route) => route.fulfill({ json: [{
    file_name: '20261010-020000000.pre-discard.sqlite',
    path: '/backups/ledger_x/20261010-020000000.pre-discard.sqlite',
    kind: 'pre-discard', created_at: '2026-10-10T02:00:00', size_bytes: 1024,
  }] }));
  await page.goto('/settings?section=backup&backup=%2Fbackups%2Fledger_x%2F20261001-020000000.pre-discard.sqlite');
  await expect(page.getByTestId('backup-missing')).toHaveText(
    'That snapshot is no longer in the backup pool; retention removed it.',
  );
  await expect(page.getByTestId('backup-list').locator('li[data-highlighted="true"]')).toHaveCount(0);
});
