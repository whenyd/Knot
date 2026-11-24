# Knot

[中文](README.md) | English

Knot (结) — one knot records one thing. A structured list for development tasks: a single-user, local, offline desktop to-do app.
For requirements see [docs/specs/goal-and-requirements.en.md](docs/specs/goal-and-requirements.en.md); for layering and the data model see [docs/specs/architecture.en.md](docs/specs/architecture.en.md);
**for what is left to do see [docs/ROADMAP.en.md](docs/ROADMAP.en.md)**. Chinese: [README.md](README.md).

## Current status

The full feature set is in place:
- Three-level Project / Module / Submodule structure (create, in-place rename, drag reordering, two-step deletion)
- Tasks with one level of subtasks
- Parent-child status aggregation
- Status/priority filters and sorting
- Notes, automatic recording of created/start/end times
- Cross-project moves and parent-child conversion
- Pre-migration automatic backups and a startup error page

23 release commands + 1 development-only command (`reset_database`) are implemented; for the full list see
[docs/ROADMAP.en.md](docs/ROADMAP.en.md#current-status), which this document does not duplicate.

The interface supports a three-state **中文 / English / System** switch (language button in the status bar; the choice is stored locally); backend errors, confirmation reasons and
aggregation notices also go through the same structured-message translation. See [docs/i18n/glossary.md](docs/i18n/glossary.md) for the terminology glossary.

## Structure

```
index.html            Window shell: sidebar + main area; static text only keeps data-i18n placeholders
src/
  api.ts              The only frontend ↔ backend interface layer (invoke appears only here)
  errors.ts           Turns a structured backend message / string / Error into one localized message
  grouping.ts         Pure-function data pipeline: in-group sorting, module-based tree grouping, scope filtering, group counts
  grouping.test.ts    Pure-function unit tests (vitest)
  menu.ts             Generic popup menu component (open/position/close)
  tooltip.ts          Custom hover tooltip
  main.ts             State + events + rendering (native DOM, no UI framework)
  i18n/               Language state, Chinese/English dictionaries, static-text binding, date formatting, backend message translation
src-tauri/src/
  main.rs             Entry point, only calls knot_lib::run()
  lib.rs              Wiring: open the connection, manage state, register commands, startup error window
  commands.rs         Command layer: unwrap parameters + forward, no rules
  domain.rs           Domain layer: the only write entry, all rules live here (with unit tests)
  message.rs          Structured message { key, params } and catalog
  db.rs / migrations.rs  Storage layer: database file, connection, backup, schema migration
src-tauri/backend-keys.json   Backend message key contract shared by frontend and backend
```

## Development

```bash
npm install
npm run tauri dev      # development window (frontend hot reload)
npm test               # frontend pure-function unit tests (vitest)
npm run build          # frontend type check + bundle
cargo test --manifest-path src-tauri/Cargo.toml   # backend rule unit tests
npm run tauri build    # produce installers (requires WiX/NSIS on the system)
npx tauri build --no-bundle    # release exe only; no installer bundling, no WiX/NSIS download
```

Database file: Windows `%APPDATA%/knot/tasks.sqlite` (the platform's app data directory on macOS/Linux).
Deleting that file resets the app.

## Layering rules

- Rules are expressed only in `domain.rs`; SQL triggers do not duplicate them.
- The frontend does not recompute aggregation rules; after any write it always re-fetches the current container in full.
- `invoke` appears only in `src/api.ts`.
- User-visible copy lives only in `src/i18n/**` and in the backend `message.rs` keys; a new backend key must be added to
  `src-tauri/backend-keys.json` and both the Chinese and English translation tables.
