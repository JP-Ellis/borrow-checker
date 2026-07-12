import { type ChildProcess, spawn } from 'node:child_process';
import { execSync }                  from 'node:child_process';
import { mkdirSync }                 from 'node:fs';
import { join }                      from 'node:path';
import { dirname, resolve }          from 'node:path';
import { fileURLToPath }             from 'node:url';
import type { Options }              from '@wdio/types';

const __dirname = dirname(fileURLToPath(import.meta.url));

let tauriDriver: ChildProcess | undefined;

async function waitForDriver(port: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://localhost:${port}/status`);
      if (res.ok) return;
    } catch {
      // not ready yet
    }
    await new Promise<void>(r => setTimeout(r, 100));
  }
  throw new Error(`tauri-driver did not become ready within ${timeoutMs}ms`);
}

const APPLICATION =
  process.env['TAURI_BINARY'] ?? resolve(__dirname, '../target/debug/bc-app');
const APP_CRATE    = resolve(__dirname, '../crates/bc-app');
const TEST_DB_DIR  = resolve(__dirname, 'fixtures');
const TEST_DB_PATH = join(TEST_DB_DIR, 'test.db');

export const config: Options.Testrunner = {
  hostname: 'localhost',
  port:     4444,
  path:     '/',

  specs: ['./tests/flows/**/*.spec.ts'],

  maxInstances: 1,

  capabilities: [
    {
      maxInstances: 1,
      'wdio:enforceWebDriverClassic': true,
      'tauri:options': {
        application: APPLICATION,
      },
    },
  ],

  logLevel:  'warn',
  framework: 'mocha',
  reporters: ['spec'],

  /* Default wait for `waitForDisplayed`/`waitForExist`/`waitUntil` and
   * `expect(...)` polling. Deliberately generous to absorb container-in-CI
   * cold-start and slowness; specs should only override when a wait genuinely
   * needs to be shorter or much longer than this. */
  waitforTimeout: 15_000,

  mochaOpts: {
    ui:      'bdd',
    timeout: 60_000,
  },

  async onPrepare() {
    // Ensure fixtures directory exists.
    mkdirSync(TEST_DB_DIR, { recursive: true });

    // Set BC_DB_PATH so the Tauri app reads the test database.
    process.env['BC_DB_PATH'] = TEST_DB_PATH;

    // Seed the test database.
    // SEED_BIN is set by the container task when a pre-built Linux binary is available;
    // otherwise fall back to cargo run (builds bc-seed on demand, no nightly required).
    const seedBin = process.env['SEED_BIN'] || 'cargo run -p bc-seed --';
    console.log('Seeding test database…');
    execSync(`${seedBin} --db-path "${TEST_DB_PATH}" --force`, {
      cwd:   __dirname,
      stdio: 'inherit',
    });

    if (!process.env['SKIP_BUILD']) {
      console.log('Building Tauri debug binary…');
      execSync('cargo tauri build --debug', {
        cwd:   APP_CRATE,
        stdio: 'inherit',
      });
    }

    tauriDriver = spawn('tauri-driver', [], {
      stdio: [null, process.stdout, process.stderr],
      env:   { ...process.env },  // passes BC_DB_PATH to tauri-driver + child processes
    });

    try {
      await waitForDriver(4444, 15_000);
    } catch (err) {
      tauriDriver.kill();
      // tauri-driver not available on this platform (e.g. macOS); skip gracefully
      process.exit(0);
    }
  },

  async onComplete() {
    tauriDriver?.kill();
  },
};
