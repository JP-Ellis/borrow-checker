import { dirname, resolve } from 'node:path';
import { fileURLToPath }    from 'node:url';

/**
 * Absolute path of the SQLite database backing *this worker's* app instance.
 *
 * Specs run concurrently, and each worker gets a private copy of the seeded
 * template (see `beforeSession` in `wdio.conf.ts`), so there is no single
 * shared `fixtures/test.db` to point at. The worker's tauri-driver is launched
 * with `BC_DB_PATH` set to this file, and the same value is exported into the
 * worker process so specs can assert against exactly the database the app under
 * test is writing to.
 */
export const DB_PATH = process.env['BC_DB_PATH']
  ?? resolve(dirname(fileURLToPath(import.meta.url)), '../../fixtures/template.db');
