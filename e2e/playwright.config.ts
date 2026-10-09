/**
 * Browser suite against `borrow-checker-server`. The server runs on a copy of
 * the seeded template database; `web:prepare` seeds and builds first.
 *
 * `web` drives flows that share and mutate `web.db`, so it runs one test at a
 * time. `qa` checks the `/__test` QA pages, which render fixtures and never
 * touch the database, so it runs fully parallel. `qa-discover` crawls those
 * pages into `qa/.routes.json`; it runs as a separate first invocation because
 * Playwright collects every spec before the server starts.
 */
import { defineConfig } from '@playwright/test';
import { join } from 'node:path';

const ROOT = join(import.meta.dirname, '..');
const DB   = join(import.meta.dirname, 'fixtures', 'web.db');
const PORT = 7272;

export default defineConfig({
  workers: '100%',
  use: { baseURL: `http://127.0.0.1:${PORT}` },
  projects: [
    { name: 'web', testDir: './web', fullyParallel: false, workers: 1 },
    { name: 'qa-discover', testDir: './qa', testMatch: 'discover.spec.ts' },
    {
      name: 'qa',
      testDir: './qa',
      testIgnore: 'discover.spec.ts',
      fullyParallel: true,
    },
  ],
  webServer: {
    command: `${join(ROOT, 'target', 'debug', 'borrow-checker-server')} --bind 127.0.0.1:${PORT}`,
    url: `http://127.0.0.1:${PORT}/`,
    env: {
      BC_DB__PATH: DB,
      BC_BACKUP__DIR: join(import.meta.dirname, 'fixtures', 'web-backups'),
      XDG_CONFIG_HOME: join(import.meta.dirname, 'fixtures', 'web-config'),
    },
    reuseExistingServer: false,
  },
});
