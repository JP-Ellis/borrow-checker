/**
 * Paths and environment shared by `borrow-checker-server` and the CLI calls
 * specs make against the same database.
 */
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';

const E2E = join(import.meta.dirname, '..', '..');
const ROOT = join(E2E, '..');
const FIXTURES = join(E2E, 'fixtures');

/** The documents root the server preopens for importers. */
export const DOCS = join(FIXTURES, 'web-docs');

/** The server binary `test:web` builds. */
export const SERVER_BIN = join(ROOT, 'target', 'debug', 'borrow-checker-server');

/** Environment for the server and every CLI call against its database. */
export const SERVER_ENV: Record<string, string> = {
  BC_DB__PATH: join(FIXTURES, 'web.db'),
  BC_BACKUP__DIR: join(FIXTURES, 'web-backups'),
  XDG_CONFIG_HOME: join(FIXTURES, 'web-config'),
  BC_IMPORT__DOCUMENTS_ROOT: DOCS,
  BORROW_CHECKER_PLUGIN_DIR: join(ROOT, 'target', 'plugins'),
};

/** Runs `borrow-checker <args>` against the suite's database; returns stdout. */
export function cli(...args: string[]): string {
  return execFileSync(join(ROOT, 'target', 'debug', 'borrow-checker'), args, {
    env: { ...process.env, ...SERVER_ENV },
    encoding: 'utf8',
  });
}
