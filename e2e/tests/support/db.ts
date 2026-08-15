import Database             from 'better-sqlite3';
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

/** One row of `transaction_metadata`, in display order. */
export interface MetaRow {
    key:        string;
    value_text: string;
    /** 1 when the value could not be read as its key's registered type. */
    mismatched: number;
}

/**
 * Finds the newest transaction whose `payee` metadata holds `payee`.
 *
 * A payee is an ordinary metadata entry under an ordinary key, so it lives in
 * `transaction_metadata` and not in a column of its own. Repeated keys are
 * legal, which is why this matches any entry rather than assuming one.
 */
export function dbTransactionIdByPayee(payee: string): string | undefined {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare(
                `SELECT t.id AS id
                   FROM transactions t
                   JOIN transaction_metadata m ON m.transaction_id = t.id
                  WHERE m.key = 'payee' AND m.value_text = ?
                  ORDER BY t.date DESC
                  LIMIT 1`,
            )
            .get(payee) as { id: string } | undefined;
        return row?.id;
    } finally {
        db.close();
    }
}

/** Reads every metadata entry on one transaction, in display order. */
export function dbTransactionMetadata(txId: string): MetaRow[] {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        return db
            .prepare(
                `SELECT key, value_text, mismatched
                   FROM transaction_metadata
                  WHERE transaction_id = ?
                  ORDER BY position`,
            )
            .all(txId) as MetaRow[];
    } finally {
        db.close();
    }
}

/** Reads the registered type of one metadata key, if the registry holds it. */
export function dbMetadataKeyType(key: string): string | undefined {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare('SELECT value_type FROM metadata_keys WHERE key = ?')
            .get(key) as { value_type: string } | undefined;
        return row?.value_type;
    } finally {
        db.close();
    }
}
