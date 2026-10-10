/**
 * Browser suite against `borrow-checker-server`. The server runs on a copy of
 * the seeded template database, with the built plugins and `fixtures/web-docs`
 * as its documents root; `test:web` seeds and builds first.
 */
import { defineConfig } from '@playwright/test';

import { SERVER_BIN, SERVER_ENV } from './web/support/env.js';

const PORT = 7272;

export default defineConfig({
  testDir: './web',
  fullyParallel: false,
  workers: 1,
  use: { baseURL: `http://127.0.0.1:${PORT}` },
  webServer: {
    command: `${SERVER_BIN} --bind 127.0.0.1:${PORT}`,
    url: `http://127.0.0.1:${PORT}/`,
    env: SERVER_ENV,
    reuseExistingServer: false,
  },
});
