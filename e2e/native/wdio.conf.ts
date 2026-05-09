import { type ChildProcess, spawn } from 'node:child_process';
import { execSync }                  from 'node:child_process';
import { resolve }                   from 'node:path';
import type { Options }              from '@wdio/types';

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
  process.env['TAURI_BINARY'] ?? resolve(__dirname, '../../target/debug/bc-app');
const APP_CRATE   = resolve(__dirname, '../../crates/bc-app');

export const config: Options.Testrunner = {
  specs: ['./tests/**/*.spec.ts'],

  maxInstances: 1,

  capabilities: [
    {
      maxInstances: 1,
      browserName: '',
      'wdio:enforceWebDriverClassic': true,
      'tauri:options': {
        application: APPLICATION,
      },
    },
  ],

  logLevel:  'warn',
  framework: 'mocha',
  reporters: ['spec'],

  mochaOpts: {
    ui:      'bdd',
    timeout: 60_000,
  },

  async onPrepare() {
    if (!process.env['SKIP_BUILD']) {
      console.log('Building Tauri debug binary…');
      execSync('cargo tauri build --debug', {
        cwd:   APP_CRATE,
        stdio: 'inherit',
      });
    }

    tauriDriver = spawn('tauri-driver', [], {
      stdio: [null, process.stdout, process.stderr],
    });

    await waitForDriver(4444, 15_000);
  },

  async onComplete() {
    tauriDriver?.kill();
  },
};
