# fpbx-tools

A collection of interactive terminal tools for managing FusionPBX servers. Built in Rust with a full Ratatui TUI — no config files required, just SSH key access.

```
fpbx-backup       →   pick server   →   pick domain      →   .fpbx bundle
fpbx-restore      →   pick bundle   →   pick server      →   restored domain
fpbx-info         →   list bundles or inspect a single .fpbx file
fpbx-routes-xfer  →   pick routes   →   map gateways     →   copy to dest server
```

---

## Features

- **Interactive TUI** — keyboard-driven screens with filterable lists, progress bars, and a live scrollable log panel
- **SSH key auth** — connects via your existing `~/.ssh/` keys or agent; no passwords stored anywhere
- **Custom `.fpbx` bundle format** — a single portable archive containing the database dump, voicemail/recording files, XML config, a JSON manifest, and a SHA-256 checksum
- **Non-destructive by default** — the source server is never modified; backups are pure read operations
- **Domain-scoped exports** — only the selected domain's records are exported (extensions, dialplans, ring groups, IVRs, voicemails, recordings, gateways, contacts, and more)
- **Integrity verification** — the bundle checksum is validated before any restore begins
- **Bundle inspection** — `fpbx-info` lists all local bundles or shows full manifest details for a specific `.fpbx` file
- **Route transfer** — `fpbx-routes-xfer` copies global outbound routes between servers with interactive gateway UUID remapping
- **Structured logging** — all operations are logged to `~/.fpbx/backup.log` / `restore.log` / `routes-xfer.log` without cluttering the TUI

---

## Requirements

- Rust 1.78+ (install via [rustup](https://rustup.rs))
- SSH key access to your FusionPBX servers (the running user needs `sudo -u postgres` rights for `pg_dump`/`psql`)
- `libssh2` development headers on your build machine:
  - Debian/Ubuntu: `sudo apt install libssh2-1-dev`
  - RHEL/Rocky: `sudo dnf install libssh2-devel`
  - macOS: `brew install libssh2`

---

## Installation

```bash
git clone https://github.com/codemonkey76/fpbx-tools.git
cd fpbx-tools
cargo build --release
```

The compiled binaries will be at:

```
target/release/fpbx-backup
target/release/fpbx-restore
target/release/fpbx-info
```

Copy them to somewhere on your `$PATH`, e.g.:

```bash
sudo cp target/release/fpbx-backup target/release/fpbx-restore target/release/fpbx-info target/release/fpbx-routes-xfer /usr/local/bin/
```

---

## Usage

### Backup

```bash
fpbx-backup
```

The TUI walks you through five steps:

| Step        | What happens                                                                                               |
| ----------- | ---------------------------------------------------------------------------------------------------------- |
| **Server**  | Enter the source host and SSH user, press Enter to verify connectivity and confirm FusionPBX is present    |
| **Domain**  | Browse or filter (`/`) the list of domains on that server, select one with Enter                           |
| **Output**  | Confirm or edit the destination path (default: `~/.fpbx/backups/`)                                         |
| **Running** | Watch the live progress bar and log as the DB is exported, files are archived, and the bundle is assembled |
| **Done**    | Bundle path is shown; press `q` or Enter to exit                                                           |

The resulting file is named `<domain>-<YYYYMMDD-HHMMSS>.fpbx` and saved in the configured output directory.

### Restore

```bash
fpbx-restore
```

---

### Inspect bundles

```bash
# List all bundles in ~/.fpbx/backups/
fpbx-info

# Show full details for a specific bundle
fpbx-info path/to/bundle.fpbx
```

The output includes domain name, UUID, source host, creation date, table row counts, backed-up file paths, and DB/file sizes. The checksum is verified automatically — if `open_bundle` succeeds the bundle is valid.

---

### Transfer outbound routes

```bash
fpbx-routes-xfer
```

| Step | What happens |
| --- | --- |
| **Source** | Enter the source host and SSH user, verify connectivity |
| **Routes** | Browse the server's global outbound dialplans; toggle selection with Space, select all/none with `a`/`n` |
| **Dest** | Enter the destination host and SSH user, verify connectivity |
| **Gateways** | Map each gateway referenced in the selected routes to a gateway on the destination server (auto-matched by name where possible; press `s` to skip) |
| **Confirm** | Review the transfer and press `y` to proceed |
| **Running** | Routes and their dialplan details are inserted on the destination; bridge actions are rewritten with the remapped gateway UUIDs; FusionPBX XML is reloaded |

Existing routes with the same name in the global context are replaced before inserting.

| Step        | What happens                                                                     |
| ----------- | -------------------------------------------------------------------------------- |
| **Bundle**  | Browse `~/.fpbx/backups/` and select a `.fpbx` file                              |
| **Preview** | Inspect the manifest — domain name, source host, creation date, table row counts |
| **Server**  | Enter the destination host and SSH user, verify access                           |
| **Confirm** | Review the domain mapping and confirm before any writes occur                    |
| **Running** | SQL is imported, files are restored, row counts are verified                     |

---

## Bundle format

A `.fpbx` file is a gzipped tar archive with the following structure:

```
<domain>-<timestamp>.fpbx
├── manifest.json       # domain metadata, table counts, file paths, timestamps
├── db.sql.gz           # gzipped SQL export scoped to the domain UUID
├── files.tar.gz        # voicemail, recordings, dialplan XML, directory XML
└── checksum.sha256     # SHA-256 of the above three files
```

The manifest is plain JSON and can be inspected without extracting the full bundle:

```bash
tar xzf acme_example_com-20250101-120000.fpbx manifest.json -O | jq .
```

---

## Database tables exported

All tables are filtered by `domain_uuid`. The export includes:

`v_domains`, `v_domain_settings`, `v_users`, `v_groups`, `v_user_groups`, `v_extensions`, `v_extension_users`, `v_gateways`, `v_dialplans`, `v_dialplan_details`, `v_ring_groups`, `v_ring_group_destinations`, `v_ivr_menus`, `v_ivr_menu_options`, `v_time_conditions`, `v_time_condition_periods`, `v_voicemails`, `v_voicemail_messages`, `v_call_center_queues`, `v_call_center_agents`, `v_call_center_tiers`, `v_recordings`, `v_contacts`, `v_contact_phones`, `v_contact_emails`, `v_contact_urls`, `v_contact_addresses`, `v_fax`, `v_fax_files`

Tables that don't exist on a given FusionPBX version are skipped automatically.

---

## File paths backed up

For a domain named `acme.example.com`, the following paths are included (if they exist):

```
/var/lib/freeswitch/storage/voicemail/default/acme.example.com/
/var/lib/freeswitch/recordings/acme.example.com/
/etc/freeswitch/dialplan/acme.example.com/
/etc/freeswitch/directory/acme.example.com/
```

---

## Workspace layout

```
fpbx-tools/
├── Cargo.toml                  # workspace root
├── fpbx-core/                  # shared library
│   └── src/
│       ├── ssh.rs              # SSH session, exec, SFTP up/download
│       ├── domain.rs           # domain discovery, table list, file paths
│       ├── db.rs               # pg_dump export and psql import over SSH
│       ├── bundle.rs           # .fpbx archive create/open/verify
│       └── verify.rs           # post-restore row count diffing
├── fpbx-backup/                # backup binary (TUI)
│   └── src/
│       ├── main.rs
│       └── tui/
│           ├── app.rs          # state machine + background worker
│           ├── ui.rs           # Ratatui draw functions
│           └── widgets.rs      # shared widget helpers
├── fpbx-restore/               # restore binary (TUI)
│   └── src/
│       ├── main.rs
│       └── tui/
│           ├── app.rs
│           ├── ui.rs
│           └── widgets.rs
├── fpbx-info/                  # bundle inspection CLI
│   └── src/
│       └── main.rs             # list bundles or show manifest details
└── fpbx-routes-xfer/           # outbound route transfer binary (TUI)
    └── src/
        ├── main.rs
        └── tui/
            ├── app.rs          # state machine, route/gateway fetch, transfer worker
            ├── ui.rs           # Ratatui draw functions
            └── mod.rs
```

---

## Key bindings

| Key                    | Action                         |
| ---------------------- | ------------------------------ |
| `↑` / `↓` or `j` / `k` | Navigate lists                 |
| `/`                    | Open filter input              |
| `Enter`                | Confirm / select / advance     |
| `Esc`                  | Go back / cancel filter        |
| `Tab`                  | Switch between input fields    |
| `q`                    | Quit (when no task is running) |

---

## SSH requirements on the remote servers

The SSH user needs passwordless `sudo` access to run `pg_dump` and `psql` as the `postgres` user. Add a sudoers entry on each FusionPBX host:

```
# /etc/sudoers.d/fpbx-tools
your-admin-user ALL=(postgres) NOPASSWD: /usr/bin/pg_dump, /usr/bin/psql
```

If your FusionPBX runs FreeSWITCH files under a different base path than `/var/lib/freeswitch`, adjust `DomainFilePaths::for_domain()` in `fpbx-core/src/domain.rs`.

---

## Logging

Both tools write structured logs to:

```
~/.fpbx/backup.log
~/.fpbx/restore.log
```

Set the `RUST_LOG` environment variable to increase verbosity:

```bash
RUST_LOG=debug fpbx-backup
```

---

## Roadmap

- `fpbx-restore` TUI screens (bundle picker, preview, confirm, progress)
- Domain rename on restore (map source domain name to a different destination name)
- Scheduled/cron backup mode (`fpbx-backup --headless --host ... --domain ...`)
- Listing and pruning old bundles (`fpbx-backup --list`, `--prune-older-than 30d`)
- Restore dry-run mode (validate bundle against destination without writing)

---

## License

MIT
