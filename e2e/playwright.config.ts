/**
 * Browser suite against `borrow-checker-server`. The server runs on a copy of
 * the seeded template database; `test:web` seeds and builds first.
 */
import { defineConfig } from '@playwright/test';
import { join } from 'node:path';

const ROOT = join(import.meta.dirname, '..');
const DB   = join(import.meta.dirname, 'fixtures', 'web.db');
const PORT = 7272;

export default defineConfig({
  testDir: './web',
  fullyParallel: false,
  workers: 1,
  use: { baseURL: `http://127.0.0.1:${PORT}` },
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
