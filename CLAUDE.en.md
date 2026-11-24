# CLAUDE.md

[中文](CLAUDE.md) | English

> This English mirror is for reference; Claude Code loads CLAUDE.md.

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

A single-user, local, offline desktop to-do app (Tauri 2 + native DOM/TypeScript + SQLite). The spec is split into three documents; read them before starting:

- `docs/specs/goal-and-requirements.en.md` — what is wanted (capabilities, trade-offs, Non-Goals)
- `docs/specs/architecture.en.md` — how it is layered, the data model, the command list, interaction conventions, internationalization
- `docs/ROADMAP.en.md` — what is left, dependency order, and **the open questions that must be confirmed before starting (D1–D6)**

The Chinese documentation is the authoritative version; the English mirrors live in the same directories as the Chinese (`*.en.md`, `README.en.md`, `CLAUDE.en.md`), and the glossary is in `docs/i18n/glossary.md`. Update Chinese and English together when changing behavior.

Spec-driven: before changing behavior, first check whether the corresponding task in the ROADMAP has an open question; after the change, write the spec back into goal/architecture, then tick off the ROADMAP. The "Current status" section of the README stays in sync with the ROADMAP; if the two conflict, the ROADMAP is authoritative.

## Common commands

```bash
npm install
npm run tauri dev                          # development window; Vite is pinned to 1420 (strictPort)
npm run build                              # tsc + vite build (frontend type check + bundle)
npm test                                   # frontend pure-function unit tests (vitest)
npx tsc --noEmit                           # type check only
cargo test --manifest-path src-tauri/Cargo.toml             # all backend unit tests
cargo test --manifest-path src-tauri/Cargo.toml <test name>  # a single test, e.g. v2_upgrades_existing_data_and_cascades
npm run tauri build                        # release installers (NSIS / MSI; requires WiX/NSIS on the system)
```

Frontend tests use vitest: `src/grouping.test.ts` covers the sorting, grouping, scope, counting and filtering pure functions in `grouping.ts`; `src/i18n.test.ts` covers Chinese/English dictionary key/placeholder completeness, dates, error translation and catalog consistency with `src-tauri/backend-keys.json`. Backend tests are still embedded in the `#[cfg(test)]` modules of `domain.rs` / `migrations.rs` / `db.rs` / `message.rs` / `lib.rs`, using an in-memory database or a temporary directory.

Database: Windows `%APPDATA%/knot/tasks.sqlite`. Deleting that file resets the app; the dev build also has a "Reset database (dev)" entry at the bottom of the sidebar (it clears only the live database and sidecar files and keeps `.bak-*` backups), and release builds do not register this command.

## Layering and hard rules

```
frontend (src/main.ts) → src/api.ts → commands.rs → domain.rs → db.rs / migrations.rs → SQLite
```

- **Rules live only in `domain.rs`**: status aggregation, cascades, sort-order assignment and ownership validation all live here and are pinned by unit tests. `commands.rs` is a thin shell (unwrap parameters + forward); it writes no business `if`s and assembles no SQL. SQL triggers do not express the rules again.
- **`invoke` appears only in `src/api.ts`**. The frontend never touches Tauri commands directly and never recomputes aggregation rules — after any write it always `reload()`s to re-fetch the current container in full (the data volume is small, so no incremental patch).
- **The IPC field contract belongs to Rust**: structs use `#[serde(rename_all = "camelCase")]`, enum values are lowercase, and the types in `api.ts` correspond verbatim. Field names are defined in one place; `domain.rs` has tests specifically watching this contract.
- **The backend only decides "which message", never writes sentences**: all `Err`s, `plan_set_status.reason` and `notices` are `crate::message::Message { key, params }` (`message.rs`). A key must appear in both the Chinese and English tables in `src-tauri/backend-keys.json` and `src/i18n/backend.ts`; parameters hold only stable tokens, numbers and raw technical details. `describe()` catches string errors thrown by the Tauri framework.
- **Tell first, then write**: all destructive/aggregating operations go through `plan_*` (read-only, returns `needs_confirm` / counts) → frontend dialog → `apply_*` / `delete_*`. The numbers in the confirmation dialog come from the backend, and the frontend does not recompute the cascade scope.
- **User-visible copy lives only in `src/i18n/**`**: no new natural language may be added to `main.ts` or `index.html`; static HTML keeps only `data-i18n*` placeholders. Status/priority tokens, `data-*`, `aria-sort` and `localStorage` values are never translated. Date display must go through `formatSchedule` in `src/i18n/date.ts`; storage and sorting still use `YYYY-MM-DDTHH:MM`.
- **Language state**: the mode is stored in `localStorage` under `knot.language.v1` (`zh` / `en` / `system`), defaulting to `system`; after switching, update `<html lang>`, the static copy and run a full `render()`. New UI copy must be added to both `messages.zh.ts` and `messages.en.ts`, and plurals use `.one` / `.other`.
- **Comments are in English**: source comments (ts/rs/html/css/sql) are always English; Chinese data in test fixtures may be kept. Released migrations may only have their comments changed, with not a single character of SQL structure or logic touched.

## Data model and migrations

- "My Tasks" is not a project record, but `task.project_id IS NULL`.
- The classification is `project → module → submodule`; subtasks have only one level (`parent_task_id`), and a subtask stores no `module_id`/`submodule_id` — the truth of ownership is in the parent task, and the backend rejects "a subtask with a subtask".
- Tasks carry `created_at` / `start_at` / `end_at` (local-time text `YYYY-MM-DDTHH:MM`, `NULL` when not recorded), and **none of them are edited manually**: `create_task` writes the created time once; `write_status` in `domain.rs` maintains the start / end times on status changes — entering Doing records the start, entering Done records the end, reverting to Todo clears both. Times are only displayed and do not participate in sorting, filtering or counting.
- Deletion is always "deleting the parent takes the children": the foreign keys of project/module/submodule/parent task are all `CASCADE`. Deleting a module really deletes the tasks in its group; they are not returned to Default (D1 decided).
- Migrations use `PRAGMA user_version`, and `MIGRATIONS: &[&str]` runs in order. **Not a single character of the SQL structure and logic of released entries may be changed; the Chinese comments inside them may be translated** (the history of v1 writing classification foreign keys as `SET NULL` cannot be changed, and v2 changes them to `CASCADE` via "create a new table → copy → drop the old table → rename"). Forward-only, no down migrations.
- The whole migration is wrapped in a single transaction; `foreign_keys` is turned off before the transaction (a no-op inside a transaction), and `PRAGMA foreign_key_check` runs **before committing**; on failure it rolls back `user_version` as well. Every connection explicitly runs `PRAGMA foreign_keys = ON` when opened.
- Automatic pre-migration backup: only when upgrading an old database (`0 < user_version < latest version`) is `tasks.sqlite.bak-v<old version>-<epoch milliseconds>` generated, keeping the latest 5 (only `bak-v*` is cleaned up); if the backup fails, migration stops. On migration failure `lib.rs` opens a `?startupError=` error window showing the reason and backup path instead of panicking. `reset_database` is registered only in debug, and the button is shown only when `import.meta.env.DEV` is true; release builds have no such path.

## Frontend structure

- `main.ts` holds the single mutable `state` (`container` / `scope` / `collapsedNodes`, etc.), and all rendering is "full state → rebuild the DOM", with no framework and no two-way binding.
- `grouping.ts` is a pure-function data pipeline: `sortTasks` → `buildGroups` → `selectScope` → `countGroup`; the same tree serves both the sidebar scope-tree counts and the table grouping, with no second copy. **Group counts include parent tasks only** (subtasks render under their parent and do not form groups of their own). The Phase 5 filters only affect rendering, and counts are still based on unfiltered data; when a parent task does not match but a subtask does, the parent row is greyed out as context.
- `menu.ts` is the only menu implementation (open/position/close/keyboard navigation/subpanels), shared by three places: the sidebar right-click menu, the task row right-click menu and the priority popover. Task row moves and deletes are only in the right-click menu; do not write another implementation for a new menu.
- Column widths are maintained in `main.ts`: the table draws no cell borders (no permanent vertical lines between columns and no horizontal lines between rows), only a single divider below the header; dragging a header divider **adjusts the columns on both sides at the same time**, hovering highlights both headers, dragging shows both column widths, and the vertical line appears only at the handle; the "Task" column absorbs the remaining width; non-task column widths are stored in `localStorage`.
- Column sorting lives in `state.sort` in `main.ts`: the buttons to the right of the priority / created / start / end headers cycle ascending → descending → default, and `sortTasks(tasks, sort)` in `grouping.ts` does the sorting; once a column is selected the whole group is sorted by it (no longer moving done tasks last), parent tasks are sorted within the group, and subtasks use the same rule under their respective parents. Sorting only affects rendering and is not written to the database.
- The sidebar width is maintained by `.sidebar-resizer` (zero width, 10px drag area): 180–420px, leaving at least 360px for the main area, stored in `localStorage` under `knot.sidebarWidth.v1`; the left and right arrow keys can also adjust it.
- Dialogs (`askConfirm` / `askText` / `askNote`) share one modal, and **only one is allowed at a time** — a new dialog dismisses the previous one as cancel; otherwise two `await`s compete for the same set of listeners.
- The new-project form is the only non-dialog input form and has a single exit, `resetProjectForm()`; a `document` listener is attached when it opens and removed when it closes.
- Fallback after deletion: when deleting a module/submodule, if `scope` is in the deleted subtree → back to the project's "All"; when deleting a project, if the view is currently on that project → back to "My Tasks".

## UI verification conventions

The verification conventions at the end of the ROADMAP are a hard requirement; do not just report "it compiles":

1. `cargo test` + `npx tsc --noEmit` + `npm run build`.
2. Changes involving the UI: use `mockIPC` from `@tauri-apps/api/mocks` to mount `main.ts` onto `index.html`, start `npx vite --port 1420`, and take screenshots with headless Edge for manual review; when verifying a call chain, make the mock backend stateful and simulate clicks.
3. Go through the interactions in a real window with `npm run tauri dev` (screenshots prove rendering, but not whether the data is correct).
4. When wrapping up, find the PID by port and then `taskkill //F //PID`: `background_command stop` cannot kill the child process spawned by `npx`, which will hold port 1420.
