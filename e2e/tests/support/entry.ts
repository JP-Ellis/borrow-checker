import Database from 'better-sqlite3';
import { browser, $ } from '@wdio/globals';
import { DB_PATH } from './db.js';

/** A posting as stored: `amount` is null for an elided leg. */
export interface PostingRow {
    account_name: string;
    amount:       string | null;
    commodity:    string | null;
}

/**
 * Marks the account named `leaf` as closed on `closedOn`, so a posting dated
 * after it draws a write warning. The seed has no closed account.
 */
export function dbCloseAccount(leaf: string, closedOn: string): void {
    const db = new Database(DB_PATH);
    try {
        const { changes } = db
            .prepare('UPDATE accounts SET closed_on = ? WHERE name = ?')
            .run(closedOn, leaf);
        if (changes !== 1) throw new Error(`expected one account named ${leaf}, updated ${changes}`);
    } finally {
        db.close();
    }
}

/** How many transactions carry `payee` as a metadata entry. */
export function dbCountByPayee(payee: string): number {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db
            .prepare(
                `SELECT COUNT(DISTINCT transaction_id) AS n
                   FROM transaction_metadata
                  WHERE key = 'payee' AND value_text = ?`,
            )
            .get(payee) as { n: number };
        return row.n;
    } finally {
        db.close();
    }
}

/** The transaction's postings in display order. */
export function dbPostings(transactionId: string): PostingRow[] {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        return db
            .prepare(
                `SELECT a.name AS account_name, p.amount, p.commodity
                   FROM postings p
                   JOIN accounts a ON a.id = p.account_id
                  WHERE p.transaction_id = ?
                  ORDER BY p.position`,
            )
            .all(transactionId) as PostingRow[];
    } finally {
        db.close();
    }
}

/** The id of the account named `leaf`, as used in `/accounts/<id>`. */
export function dbAccountId(leaf: string): string {
    const db = new Database(DB_PATH, { readonly: true });
    try {
        const row = db.prepare('SELECT id FROM accounts WHERE name = ?').get(leaf) as
            { id: string } | undefined;
        if (row === undefined) throw new Error(`no account named ${leaf}`);
        return row.id;
    } finally {
        db.close();
    }
}

/** How many register rows are expanded. */
export async function expandedCount(): Promise<number> {
    return browser.execute(
        () => document.querySelectorAll('[aria-label="transaction register"] [data-tx-id][aria-expanded="true"]').length,
    );
}

/** Selector for the inline register draft. */
export const DRAFT = '[data-testid="register-draft"]';
/** Selector for the global new-transaction dialog. */
export const DIALOG = '[role="dialog"][aria-label="New transaction"]';

/** Waits for the draft or dialog under `root` to show its first leg. */
export async function waitForEditor(root: string): Promise<void> {
    await $(root).waitForDisplayed({ timeoutMsg: `${root} did not appear` });
    await $(root).$('[data-testid="posting-row"]').waitForDisplayed({
        timeoutMsg: `${root} showed no posting row`,
    });
}

/** The `index`th leg row inside `root`. */
export async function leg(root: string, index: number): Promise<WebdriverIO.Element> {
    const rows = await $(root).$$('[data-testid="posting-row"]').getElements();
    const row = rows[index];
    if (row === undefined) throw new Error(`${root} has no leg ${index}`);
    return row;
}

/**
 * Replaces leg `index`'s account by typing `query` (a substring of the full
 * path) and accepting the first suggestion.
 */
export async function pickAccount(root: string, index: number, query: string): Promise<void> {
    const input = (await leg(root, index)).$('[data-testid="account-input"]');
    await input.setValue(query);
    await (await leg(root, index)).$('ul li').waitForDisplayed({
        timeoutMsg: `no suggestion for ${query}`,
    });
    await browser.keys(['Enter']);
    await browser.waitUntil(
        async () => (await (await leg(root, index)).$('ul').isExisting()) === false,
        { timeoutMsg: `the account menu stayed open after picking ${query}` },
    );
}

/** Types `value` into leg `index`'s amount, replacing what is there. */
export async function setAmount(root: string, index: number, value: string): Promise<void> {
    await (await leg(root, index)).$('[data-testid="posting-amount"]').setValue(value);
}

/** Sets the draft's payee through its first metadata row. */
export async function setPayee(root: string, payee: string): Promise<void> {
    await $(root).$('[data-testid="meta-value"]').setValue(payee);
}

/** The Save button inside `root`. */
export function saveButton(root: string): ChainablePromiseElement {
    return $(root).$('[aria-label="save transaction"]');
}

/** The text of the toast of `kind`, once one exists in the DOM. */
export async function toastText(kind: 'success' | 'warn'): Promise<string> {
    const selector = `[popover] [data-kind="${kind}"]`;
    await $(selector).waitForExist({ timeoutMsg: `no ${kind} toast` });
    return browser.execute(
        (sel: string) => document.querySelector(sel)?.textContent ?? '',
        selector,
    );
}

/** Whether a toast of `kind` is in the DOM. */
export async function hasToast(kind: 'success' | 'warn'): Promise<boolean> {
    return $(`[popover] [data-kind="${kind}"]`).isExisting();
}

/** Clicks the toast action labelled `label`. */
export async function clickToastAction(label: string): Promise<void> {
    const button = $('[popover] [data-kind]').$(`button=${label}`);
    await button.waitForExist({ timeoutMsg: `the toast offered no "${label}"` });
    await browser.execute((el: Element) => (el as HTMLElement).click(), await button.getElement() as unknown as Element);
}
