# AGENTS.md — fpbx-tools

Agent guide for working in this Rust workspace.

---

## What this project is

A set of interactive terminal tools for backing up and restoring FusionPBX domains across servers. Four Cargo workspace members:

| Crate | Type | Purpose |
|---|---|---|
| `fpbx-core` | library | Shared SSH, DB, bundle, domain, and verify logic |
| `fpbx-backup` | binary | TUI: connect → pick domain → export → `.fpbx` bundle |
| `fpbx-restore` | binary | TUI: pick bundle → connect → import SQL + files |
| `fpbx-info` | binary | CLI: list bundles or inspect a single `.fpbx` file |
| `fpbx-routes-xfer` | binary | TUI: copy global outbound routes between servers with gateway remapping |

---

## Essential commands

```bash
# Build all binaries (debug)
cargo build

# Build release binaries
cargo build --release
# Output: target/release/fpbx-backup, fpbx-restore, fpbx-info

# Check compile errors without producing artifacts (fastest)
cargo check

# Run tests (no unit tests exist yet — this will still compile-check)
cargo test

# Run a specific binary
cargo run -p fpbx-backup
cargo run -p fpbx-restore
cargo run -p fpbx-info

# Inspect a bundle from the command line
cargo run -p fpbx-info -- path/to/bundle.fpbx
# or with no args, lists bundles in ~/.fpbx/backups/

# Increase log verbosity (logs go to ~/.fpbx/backup.log or restore.log)
RUST_LOG=debug cargo run -p fpbx-backup
```

---

## Workspace layout

```
fpbx-tools/
├── Cargo.toml              # workspace root; all shared deps declared here
├── fpbx-core/src/
│   ├── lib.rs              # re-exports: bundle::*, domain::*, ssh::SshSession
│   ├── ssh.rs              # SshSession (connect/exec/download/upload/verify_fusionpbx)
│   ├── domain.rs           # FpbxDomain, list_domains(), count_domain_rows(), DomainFilePaths
│   ├── db.rs               # export_domain_sql_v2(), import_domain_sql()
│   ├── bundle.rs           # BundleManifest, create_bundle(), open_bundle(), list_bundles()
│   └── verify.rs           # VerifyReport, verify_restore()
├── fpbx-backup/src/
│   ├── main.rs             # terminal setup/teardown, event loop (100ms tick)
│   └── tui/
│       ├── app.rs          # App state machine + background worker threads
│       ├── ui.rs           # Ratatui draw functions, one per AppScreen variant
│       └── widgets.rs      # shared widget helpers
├── fpbx-restore/src/
│   ├── main.rs             # same terminal boilerplate as fpbx-backup
│   └── tui/
│       ├── app.rs          # restore App state machine
│       ├── ui.rs
│       └── mod.rs
├── fpbx-info/src/
│   └── main.rs             # no TUI; plain stdout with colored crate
└── fpbx-routes-xfer/src/
    ├── main.rs             # terminal setup/teardown, event loop
    └── tui/
        ├── app.rs          # state machine, route/gateway fetch, transfer worker
        ├── ui.rs           # Ratatui draw functions
        └── mod.rs
```

---

## Code patterns and conventions

### Rust edition
All crates use `edition = "2024"`.

### Error handling
- `anyhow::Result<T>` is used everywhere.
- Propagate with `?`; annotate context with `.context("…")` or `.with_context(|| …)`.
- Never panic in library code — always return `Result`.
- `exec_ok()` on `SshSession` is the standard helper: runs a remote command and bails with stderr on non-zero exit.

### SSH execution pattern
```rust
// Preferred: fail on non-zero
session.exec_ok("command")?;

// When you want to inspect exit code or stderr
let (stdout, stderr, code) = session.exec("command")?;

// When failure is expected/acceptable (e.g., table may not exist)
session.exec_ok(&cmd).unwrap_or_else(|_| "default".into());
```

### Background workers in TUI
Both TUI binaries use the same pattern:
1. `App` holds `worker: Option<Arc<Mutex<WorkerState>>>`.
2. `start_*()` methods spawn a `thread::spawn` closure that writes into `WorkerState`.
3. `App::tick()` (called every 100ms) polls `worker.lock().unwrap()` and transitions `AppScreen` when `done == true`.
4. The UI thread never blocks — all SSH/IO work runs in the worker thread.

### TUI screen state machine
Each binary has an `AppScreen` enum with variants for each wizard step. `App::handle_key()` dispatches to a per-screen handler method. `ui.rs::draw()` matches on `app.screen.clone()` and calls the matching `draw_*()` function.

### Ratatui layout pattern
All `draw_*` functions follow:
```rust
let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([...])
    .margin(N)
    .split(area);
// render widgets into chunks[i]
```

### Color palette (ui.rs)
```rust
const ACCENT: Color = Color::Cyan;
const MUTED:  Color = Color::DarkGray;
const OK:     Color = Color::Green;
const ERR:    Color = Color::Red;
const TITLE:  Color = Color::White;
```
Reuse these in all new screens. `fpbx-restore/tui/ui.rs` should mirror this palette.

### Logging
Use `tracing::{debug, info}` macros. Log files go to `~/.fpbx/backup.log` / `restore.log`. Never write to stdout from library code (TUI owns the terminal).

### Dependency management
All versions live in `[workspace.dependencies]` in the root `Cargo.toml`. Crate-level `Cargo.toml` files reference them with `.workspace = true`. When adding a new dependency, add it to the root first.

---

## The `.fpbx` bundle format

A gzipped tar archive containing exactly:

```
manifest.json      — JSON-serialized BundleManifest (version, domain, table_counts, etc.)
db.sql.gz          — gzipped plain SQL (INSERT statements, wrapped in BEGIN/COMMIT)
files.tar.gz       — tar of voicemail/recordings/dialplan/directory dirs
checksum.sha256    — SHA-256 of the above three files concatenated
```

`open_bundle()` always verifies the checksum before returning. `create_bundle()` computes it at assembly time. Bundle file names: `<domain_name_dots_to_underscores>-<YYYYMMDD-HHMMSS>.fpbx`.

---

## Database export strategy (`db.rs`)

`export_domain_sql_v2` is the current (active) export function used by `fpbx-backup`. The older `export_domain_sql` is kept but not called. The v2 approach:

1. For each table in `DOMAIN_TABLES` (ordered for FK safety), check if the table exists on the remote.
2. Check if the table has a `domain_uuid` column; if so, filter by it.
3. Export as CSV via `COPY … TO STDOUT WITH CSV HEADER`, then convert to `INSERT … ON CONFLICT DO NOTHING` statements locally.
4. Wrap everything in `BEGIN; … COMMIT;` and gzip locally (no temp files on the remote).

All psql commands run as: `sudo -u postgres psql -d fusionpbx …`

The remote user needs passwordless sudo to run `pg_dump` and `psql` as `postgres`.

---

## SSH config auto-complete

`fpbx-backup` parses `~/.ssh/config` at startup into a `HashMap<alias, SshHostEntry>`. When the user types a known `Host` alias in the host field, the SSH user is auto-populated from the config's `User` directive and the actual `HostName` is resolved for connecting. Wildcard entries (`Host *`) are skipped.

---

## Known incomplete areas (as of last commit)

- `fpbx-restore`: The `Confirm` screen handler has a `// TODO: start restore worker` placeholder — the actual restore worker is not yet spawned. `import_domain_sql()` in `fpbx-core/src/db.rs` is implemented and ready to call.
- `fpbx-restore`: `widgets.rs` and full `ui.rs` screens (BundlePicker, Preview, Server, Confirm, Progress, Done) may be partially stubbed.
- The `fpbx-backup/src/tui/widgets.rs` file exists but shared widgets may be minimal.
- No unit tests exist anywhere in the workspace yet.
- `verify_restore()` sets `files_ok: true` unconditionally — file verification is not yet implemented.

---

## FusionPBX remote paths

Hardcoded in `fpbx-core/src/domain.rs` → `DomainFilePaths::for_domain()`:

```
/var/lib/freeswitch/storage/voicemail/default/<domain>/
/var/lib/freeswitch/recordings/<domain>/
/etc/freeswitch/dialplan/<domain>/
/etc/freeswitch/directory/<domain>/
```

Only paths that actually exist on the remote are included (checked with `test -d`).

---

## Build requirements

- Rust 1.78+
- `libssh2` dev headers on the build machine:
  - Debian/Ubuntu: `sudo apt install libssh2-1-dev`
  - RHEL/Rocky: `sudo dnf install libssh2-devel`
  - macOS: `brew install libssh2`
