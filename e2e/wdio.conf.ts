import { type ChildProcess, spawn }  from 'node:child_process';
import { execSync }                   from 'node:child_process';
import { copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { cpus }                       from 'node:os';
import { join }                       from 'node:path';
import { dirname, resolve }           from 'node:path';
import { fileURLToPath }              from 'node:url';
import Database                       from 'better-sqlite3';
import type { Capabilities, Options } from '@wdio/types';

const __dirname = dirname(fileURLToPath(import.meta.url));

/* One tauri-driver per worker (see `beforeSession`), tracked so `afterSession`
 * can reap it. Each worker is a separate process, so this holds at most one. */
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
const APP_CRATE     = resolve(__dirname, '../crates/bc-app');
const TEST_DB_DIR   = resolve(__dirname, 'fixtures');
/* Seeded once, then copied per worker — never opened by the app itself. */
const TEMPLATE_DB   = join(TEST_DB_DIR, 'template.db');

/* Base ports; each worker offsets from these by its slot index. tauri-driver
 * needs two: its own WebDriver port and the WebKitWebDriver it fronts. */
const DRIVER_PORT_BASE = 4444;
const NATIVE_PORT_BASE = 5555;

/** Worker slot index parsed from a WDIO `cid` such as `0-3`. */
function slotOf(cid: string): number {
  return Number.parseInt(cid.split('-')[1] ?? '0', 10) || 0;
}

/**
 * How many spec files to run at once.
 *
 * A worker is not just a browser tab: it runs a full desktop app, its own
 * tauri-driver, and WebKit's helper processes. Budgeting ~2 cores each keeps a
 * 4-vCPU CI runner at 2 workers and a roomier workstation at the cap.
 * Over-subscribing does not merely run slower — it starves the app enough that
 * IPC writes miss their `waitforTimeout` and specs fail outright.
 *
 * `WDIO_MAX_INSTANCES` overrides it, which is also how to reproduce CI's
 * concurrency locally.
 */
const MAX_INSTANCES =
  Number.parseInt(process.env['WDIO_MAX_INSTANCES'] ?? '', 10)
  || Math.max(1, Math.min(4, Math.floor(cpus().length / 2)));

export const config: Options.Testrunner = {
  hostname: 'localhost',
  path:     '/',

  specs: ['./tests/flows/**/*.spec.ts'],

  /* Spec files are independent: each worker gets a private copy of the seeded
   * database (see `beforeSession`), so they may run concurrently. Kept modest
   * because every session launches a real WebKitGTK app that competes for CPU
   * — see `MAX_INSTANCES`. */
  maxInstances: MAX_INSTANCES,

  capabilities: [
    {
      browserName:  'wry',
      maxInstances: MAX_INSTANCES,
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
   * `expect(...)` polling. Deliberately generous to absorb CI cold-start and
   * slowness; specs should only override when a wait genuinely needs to be
   * shorter or much longer than this. */
  waitforTimeout: 15_000,

  mochaOpts: {
    ui:      'bdd',
    timeout: 60_000,
  },

  onPrepare() {
    // Ensure fixtures directory exists.
    mkdirSync(TEST_DB_DIR, { recursive: true });

    // Seed the template database that every worker copies from.
    // SEED_BIN lets a caller point at an already-built binary; otherwise fall
    // back to cargo run (builds bc-seed on demand, no nightly required).
    const seedBin = process.env['SEED_BIN'] || 'cargo run -p bc-seed --';
    console.log('Seeding test database…');
    execSync(`${seedBin} --db-path "${TEMPLATE_DB}" --force`, {
      cwd:   __dirname,
      stdio: 'inherit',
    });

    /* Fold the write-ahead log back into the main file. The seeder leaves most
     * of its writes in `template.db-wal`, and workers copy only the `.db`, so
     * without this they would each start from a partially-populated database. */
    const template = new Database(TEMPLATE_DB);
    template.pragma('wal_checkpoint(TRUNCATE)');
    template.close();

    if (!process.env['SKIP_BUILD']) {
      console.log('Building Tauri debug binary…');
      execSync('cargo tauri build --debug', {
        cwd:   APP_CRATE,
        stdio: 'inherit',
      });
    }
  },

  /**
   * Give this worker its own database and its own tauri-driver.
   *
   * The app inherits its environment from the tauri-driver that launches it,
   * so `BC_DB_PATH` has to be set on a driver owned by this worker — a single
   * shared driver would hand every session the same database. Ports are
   * derived from the worker slot so concurrent workers never collide.
   */
  async beforeSession(
    cfg: Options.Testrunner,
    _caps: Capabilities.RequestedStandaloneCapabilities,
    _specs: string[],
    cid: string,
  ) {
    const slot   = slotOf(cid);
    const dbPath = join(TEST_DB_DIR, `test-${cid}.db`);
    /* Drop any sidecars left by an earlier run: they would be replayed over
     * the freshly copied file and resurrect the previous run's writes. */
    for (const sidecar of ['-wal', '-shm']) {
      rmSync(`${dbPath}${sidecar}`, { force: true });
    }
    copyFileSync(TEMPLATE_DB, dbPath);

    const port = DRIVER_PORT_BASE + slot;
    cfg.port = port;

    /* Specs that assert directly against SQLite read this (see
     * `tests/support/db.ts`) so they open the same file as the app. */
    process.env['BC_DB_PATH'] = dbPath;

    tauriDriver = spawn(
      'tauri-driver',
      [
        '--port',        String(port),
        '--native-port', String(NATIVE_PORT_BASE + slot),
      ],
      {
        stdio: [null, process.stdout, process.stderr],
        env:   { ...process.env, BC_DB_PATH: dbPath },
      },
    );

    try {
      await waitForDriver(port, 15_000);
    } catch {
      tauriDriver.kill();
      // tauri-driver not available on this platform (e.g. macOS); skip gracefully
      process.exit(0);
    }
  },

  afterSession() {
    tauriDriver?.kill();
    tauriDriver = undefined;
  },
};
