# Run the web server

`borrow-checker server` serves the ledger to browsers on your network. It has
no authentication: bind it only to an interface you trust (loopback, or a
Tailscale address), and never to a public interface.

## Build

```sh
mise run build:server
install -m 755 target/release/borrow-checker-server ~/.local/bin/
```

`borrow-checker server` finds the binary beside `borrow-checker` or on `PATH`.

## Configure

In the BorrowChecker config file:

```toml
[server]
bind = "100.64.0.10:7171"   # this host's Tailscale address
```

`--bind` overrides it, and `BC_SERVER__BIND` overrides the config file. The
database, backup directory and plugins come from the same config as the CLI.

The server has no `--db-path` flag. To pick the database, set `BC_DB__PATH`,
or put `--db-path` before the subcommand:

```sh
borrow-checker --db-path ~/ledger.db server   # forwarded as BC_DB__PATH
BC_DB__PATH=~/ledger.db borrow-checker-server
```

`--db-path` after `server` fails with an "unexpected argument" error. Every
argument after `server` passes unchanged to `borrow-checker-server`.

Only one BorrowChecker process may hold a database open at a time. The
desktop app and the web server must never run against the same database at
once. Each takes a `<db file name>.lock` file beside the database. Whichever
one starts second waits briefly, then fails.

`borrow-checker restore` refuses the same way while either runs. Stop the
server first.

## Request checks

The server refuses a request that another website could make through your
browser:

- **Host allowlist.** Every request, pages included, must name the server by
  an IP address, by `localhost`, or by a hostname in `allowed-hosts`. Any
  other `Host` gets 403. A DNS-rebinding page points its own hostname at
  your server's address, so its requests carry that hostname and fail.
  Behind a reverse proxy, every `X-Forwarded-Host` entry must pass the
  same check.
- **Same origin.** An RPC request whose `Origin` matches neither its `Host`
  nor its `X-Forwarded-Host` gets 403. So does one marked
  `Sec-Fetch-Site: cross-site`.
- **JSON only.** An RPC request must send `Content-Type: application/json`.
  Anything else gets 415. A browser sends that content type to another site
  only after a CORS preflight. The server answers no preflight.

To reach the server by name, list the name without a scheme or port. For a
Tailscale MagicDNS name:

```toml
[server]
bind          = "100.64.0.10:7171"
allowed-hosts = ["ledger.example.ts.net"]
```

Tailscale Serve needs the same entry. It keeps the browser's `Host`, and
the server matches an `https://` `Origin` against it on port 443.

`BC_SERVER__ALLOWED_HOSTS` replaces the list with comma-separated names, for
example `BC_SERVER__ALLOWED_HOSTS=ledger.example.ts.net,ledger`.

## Run under systemd

`~/.config/systemd/user/borrow-checker.service`:

```ini
[Unit]
Description=BorrowChecker web server
After=network-online.target

[Service]
ExecStart=%h/.local/bin/borrow-checker-server
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
```

```sh
systemctl --user daemon-reload
systemctl --user enable --now borrow-checker
loginctl enable-linger "$USER"   # keep it running when you log out
```

A restore from the web UI exits the server with status 75.
`Restart=always` restarts the server after any exit, status 75 included. The
new process swaps the backup in on startup.

The page waits for the server to answer, then reloads. Without a supervisor
such as systemd, a restore leaves the server stopped. Start it again to
apply the restore. After a minute the page says the server has not come
back; reload it once the server is running.

The web UI restores only from the open ledger's own pool,
`{backup-dir}/{ledger-id}/`, as of the server's startup. Another ledger's
backups and files at the root of the backup directory are refused. After you
change the backup directory in the web UI, restart the server before
restoring from the new directory.
