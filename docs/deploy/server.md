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

Only one BorrowChecker process may hold a database open at a time. The
desktop app and the web server must never run against the same database at
once: each takes a `<db file name>.lock` file beside the database, and
whichever one starts second waits briefly, then fails. `borrow-checker restore` refuses the same way while either runs; stop the server first.

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

A restore from the web UI exits the server with status 75; `Restart=always`
brings it back after any exit, including that one, and the new process swaps
the backup in on startup.

## Known limitation

Text typed into an open transaction editor can be lost if the register
refreshes underneath it — right after another save, or after using
"discard and reload" on a stale-edit conflict. Copy unsaved edits out before
triggering either.
