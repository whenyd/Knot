---
id: architecture-en
type: architecture-design
title: Architecture — Local SQLite + Tauri Command Boundary
status: draft
parent: goal-and-requirements-en
lang: en
---

# Architecture

[中文](architecture.md) | English

One constraint runs through the whole document: **there is only one copy of the data, in the user's local file**. Therefore every decision that "could quietly corrupt the data" (migration, cascade, aggregation) leans conservative: back up first, confirm first, forward-only writes.

## Layering

```
Frontend (Vite + TS, native DOM)
   ├─ src/i18n/                —— language state, Chinese/English dictionaries, static text, date display
   └─ src/api.ts               —— the only invoke layer; types correspond verbatim to the backend
   │  Only sends commands and renders; knows no aggregation rules
   ▼
Tauri command layer (commands.rs)      —— thin shell: validate parameters, begin transactions, call domain
   ▼
Domain layer (domain.rs)               —— the only write entry: status aggregation, cascades, sort-order assignment
   ▼
Structured messages (message.rs)       —— { key, params }; the backend produces no user-visible natural language
   ▼
Storage layer (db.rs + migrations.rs)  —— rusqlite, connections, user_version migrations
   ▼
%APPDATA%/knot/tasks.sqlite
```

Rules are only allowed to live in `domain.rs`. The frontend does not recompute them, and SQL triggers do not express them again — otherwise the same rules would have two implementations and would inevitably drift. The same goes for language rules: user-visible copy lives only in `src/i18n/**`, and the backend only decides "which message it is".

## Data model

```sql
CREATE TABLE project (
  id          INTEGER PRIMARY KEY,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE module (
  id          INTEGER PRIMARY KEY,
  project_id  INTEGER NOT NULL REFERENCES project(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE submodule (
  id          INTEGER PRIMARY KEY,
  module_id   INTEGER NOT NULL REFERENCES module(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE task (
  id             INTEGER PRIMARY KEY,
  project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,    -- NULL = "My Tasks"
  module_id      INTEGER REFERENCES module(id) ON DELETE CASCADE,
  submodule_id   INTEGER REFERENCES submodule(id) ON DELETE CASCADE,
  parent_task_id INTEGER REFERENCES task(id) ON DELETE CASCADE,       -- NULL = top-level task
  title          TEXT    NOT NULL,
  status         TEXT    NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','doing','done')),
  priority       TEXT    NOT NULL DEFAULT 'mid'  CHECK (priority IN ('high','mid','low')),
  note           TEXT    NOT NULL DEFAULT '',
  created_at     TEXT,                                              -- local time 'YYYY-MM-DDTHH:MM', written on creation
  start_at       TEXT,                                              -- local time 'YYYY-MM-DDTHH:MM', NULL = not set
  end_at         TEXT,                                              -- as above; when both are set, end_at >= start_at
  sort_order     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_task_project ON task(project_id);
CREATE INDEX idx_task_module  ON task(module_id);
CREATE INDEX idx_task_parent  ON task(parent_task_id);
```

A few deliberate choices:

- **"My Tasks" is not a project row**, but `project_id IS NULL`. This saves a "system record that always exists" and also keeps the "project list" query naturally clean.
- **Subtasks do not store `module_id`/`submodule_id`** (always NULL). A subtask moves with its parent across containers and cannot change classification on its own, but it can be retargeted to another top-level task inside the same container. The single source of truth for its classification is the parent task; a redundant copy would only drift.
- **A subtask's priority does not exceed its parent's**: a new subtask defaults to "Medium", and when the parent is lower, the parent's level is used; moving a subtask above its parent is rejected; when the parent is lowered, subtasks still above it are lowered too in the same transaction. Historical data is normalized by the v3 migration.
- **Status and priority are stored as text rather than integers**, constrained with `CHECK`. The database file is meant to be human-readable, `'doing'` reads better than `1`, and the value set is enforced by `CHECK`.
- **All time fields store local-time text** `YYYY-MM-DDTHH:MM`; this is a single-user local tool, so no time-zone conversion is needed, and with a fixed format string comparison is chronological order. **Created time** is written once by `create_task` and never changes; old tasks before v5 have `NULL`. **Start / end times** are written automatically on status changes; there is no manual-set command.
- **"Only one level of subtasks" cannot be expressed with `CHECK`** (SQLite's CHECK does not support subqueries), so `domain.rs` guarantees it at creation time.
- Foreign-key cascades are always "deleting the parent takes the children": deleting a project takes its modules and tasks, deleting a module takes its submodules and tasks, and deleting a task takes its subtasks. **Deleting a module really deletes the tasks inside it, not "return them to Default"** (both are data, but only one is recoverable, so deletion always goes through `plan_*` to state the count first).

> Note: when the v1 migration was released, these three columns were written as `ON DELETE SET NULL` ("return to Default"). Released migration entries cannot be changed, so v2 uses "create a new table → copy → drop the old table → rename" to change `module_id`/`submodule_id` to `CASCADE` (SQLite does not support ALTER for foreign keys).

## Migration strategy

- **Version number**: `PRAGMA user_version`; `MIGRATIONS: &[&str]` runs in order starting from entry 1; `user_version` is "how many entries have been applied".
- **Timing**: after opening the connection and before any reads or writes, the whole thing is wrapped in a **single transaction**. On failure it rolls back and refuses to start, showing the backup file path in the UI.
- **Forward-only**: no down migrations are written. A down migration is never executed for long, is a false sense of safety, and rollback relies on the backup below.
- **Automatic pre-migration backup**: only when `0 < user_version < MIGRATIONS.len()` (upgrading an old database that already has data) is the database file copied to `tasks.sqlite.bak-v<old version>-<epoch milliseconds>` in the same directory; a new database and a startup at the same version are not backed up. If the backup fails, migration stops; the schema is never changed without a backup. After each backup only the latest 5 are kept, and cleanup only recognizes the `tasks.sqlite.bak-v*` prefix (manually placed `.bak-import-*` files are left alone). On migration failure the error message puts the **backup path first**, followed by a sentence with the reason.
- **Foreign key switch**: every connection explicitly runs `PRAGMA foreign_keys = ON` (bundled SQLite also enables it by default, but the code does not rely on compile options). Inside the migration transaction it is turned off first (foreign keys get in the way when rebuilding tables), and `PRAGMA foreign_key_check` runs **before committing** — a failed check rolls back `user_version` as well, so the next startup fails again and refuses to start instead of mistakenly believing the migration completed.
- **Startup failure screen**: setup failure no longer panics; instead the main window is hidden, a new window titled with the neutral `Knot` is created, and it loads `index.html?startupError=<percent-encoded JSON>`. The JSON shape is `StartupFailure { error: Message, backupPath?: string }`; the frontend `boot()` reads that parameter before rendering any UI or calling any command, renders the error page in the current language, and shows the backup path as a separate paragraph (`textContent`, not HTML concatenation). The window title is created before the WebView loads and cannot get the user's language, so it stays neutral; the page body still follows `knot.language.v1` / the system language.
- **Development reset**: only the dev build's invoke registration includes `reset_database` (`#[cfg(debug_assertions)]`), and the frontend only shows the "Reset database (dev)" button at the bottom of the sidebar when `import.meta.env.DEV` is true. The whole reset holds the lock: swap the file connection out for an in-memory connection and release the handle → delete `tasks.sqlite` and its `-journal`/`-wal`/`-shm` sidecar files → reopen, run migrations and put it back. **`.bak-*` backups are kept**; if deleting the main file fails, the original database is reconnected and an error is returned, and the app remains usable. Release builds do not register this command.

## Status aggregation (parent-child)

**Aggregation is an event-triggered derivation, not an invariant that always holds.** This is intentional: users are allowed to manually push a parent task down, at the cost that "parent Todo, all subtasks done" may occur until the next subtask change corrects it.

The three events that trigger aggregation (all completed in the same transaction):

1. A subtask's status changes
2. A subtask is added
3. A subtask is deleted

Aggregation actions: all subtasks `done` → **do not write the parent directly**; return `ParentStatusProposal` entries (a retarget may produce one for the old and one for the new parent), and the frontend asks the user for each before calling `apply_set_status`. Otherwise, as soon as any subtask is `doing` or `done` → set the parent to `doing`; all subtasks `todo` → set the parent to `todo` (these two are written automatically with a notice). The rule is implemented literally: with zero subtasks `all()` is true, so deleting the last unfinished subtask also proposes completing the parent. Only when the parent status is **actually rewritten** is a notice returned; going from `done` back to `doing` shows "The parent task was restored to doing", pushing from `todo` to `doing` shows "The parent task was marked as doing automatically", and reverting to `todo` shows "The parent task was restored to todo" or "The parent task was marked as todo automatically" depending on the previous status; the frontend queues them FIFO and shows toasts that disappear after 3 seconds. Manually changing a parent task (including the cascade that writes subtasks) does not trigger aggregation and produces no notice; deleting a parent task only cascades downward and does not aggregate upward.

| Event | Parent task | Subtask | What the user sees |
|---|---|---|---|
| Manually set the parent to `done`, all subtasks done | → done | unchanged | none |
| Manually set the parent to `done`, unfinished subtasks exist | confirm first → done | set to done too | dialog states the number affected; confirm button is "OK" |
| A subtask becomes `done` and is the last one | propose → done (written only after confirmation) | unchanged | dialog asks whether to complete the parent |
| A subtask becomes `done` but unfinished siblings remain | → doing (unchanged if already doing) | unchanged | toast when it was todo |
| A subtask becomes `doing` | → doing (unchanged if already doing) | unchanged | toast when it was todo / done |
| A subtask reverts / a new unfinished subtask is added | all todo → todo; otherwise → doing (unchanged if already doing) | unchanged | toast when rewritten |
| Delete the last unfinished subtask | propose → done (zero subtasks also counts as all done) | unchanged | dialog asks whether to complete the parent |
| Delete a subtask but unfinished siblings remain | recompute from the remaining subtasks: all todo → todo; some progress → doing | unchanged | toast when rewritten |
| Manually move the parent from `done` back | → the status the user chose | **unchanged** | none (may be temporarily inconsistent) |
| Delete the parent task | deleted | deleted too | dialog states the count |

## Automatic time recording

**Created time** is written once in `create_task` together with the task itself and never changes afterwards; tasks before v5 do not have this value (`NULL`), and the migration does not fabricate it.

**Start / end times** are not independently editable fields, but a **by-product of status writes**: `write_status` in `domain.rs` is the only entry point for changing status, and it maintains both times in the same `UPDATE`.

| Status change | `start_at` | `end_at` |
|---|---|---|
| Todo → Doing | record the current minute if empty | clear |
| Doing / Todo → Done | unchanged | record the current minute |
| Any status → Todo | clear | clear |
| Same status written again | unchanged | unchanged |

Aggregation automatically changing a parent task (auto-done / auto-restore-to-doing) also goes through `write_status`, so it maintains times the same way; all subtasks completed by one cascade share the same timestamp. If a task that never started is marked done by a cascade or aggregation, it only gets an end time, and the start time stays `NULL`.

## Command list

All write operations return the affected rows/nodes, and the frontend re-fetches the current container in full based on that (the data volume is small, so no incremental patch). "Confirm first, then write" is consistently shaped as two steps, `plan_*` / `apply_*`, and the frontend does not recompute rules:

```
// reads
list_projects()                       -> [{ id, name, counts, is_todo }]   // the first item is the virtual "My Tasks"
list_modules(project_id)              -> [{ id, name, submodules: [...] }]
list_tasks(container)                 -> [task]   // container = { kind: 'todo' } | { kind: 'project', id }

// writes · tasks
create_task({ container, module_id?, submodule_id?, parent_task_id?, title, priority }) -> { task, notices }
rename_task(id, title)                              // in-place edit; reject if empty after trim
set_note(id, note)
set_priority(id, priority)                            // a subtask cannot exceed its parent; lowering the parent cascades to subtasks
move_task(id, { container, module_id?, submodule_id? })   // same or cross container
reparent_task(id, parent_task_id?) -> { task, notices, parent_proposals? }    // Some = move under that parent (demote or retarget); None = promote to top level

// writes · status (two-step)
plan_set_status(id, status)           -> { affects: [task], needs_confirm, reason: Message | null }
apply_set_status(id, status, cascade: bool) -> { changed: [task], notices: [Message], parent_proposals? }

// writes · structure (deletion is also two-step: get the count, then confirm)
create_project(name) / rename_project(id, name) / plan_delete_project(id) -> { task_count } / delete_project(id)
create_module(project_id, name) / rename_module(id, name)
create_submodule(module_id, name) / rename_submodule(id, name)
move_module(id, before_id?) -> [module]   // before_id = null means move to the end; returns the project's new module tree

// delete structure nodes (module and submodule ids are independent, so kind must be included)
plan_delete_node(kind, id) -> { task_count } / delete_node(kind, id)   // kind = 'module' | 'submodule'

// delete tasks
plan_delete_task(id) -> { task_count }   // includes subtasks
delete_task(id) -> { notices, parent_proposals? }
```

`parent_proposals` is an array, each entry shaped `{ parentTaskId, parentTitle, status, message: Message }`. One operation usually has at most one; when a subtask is retargeted, the old and the new parent can both become completable, so there may be two. It only proposes what aggregation would do to the parent; the parent has not been written. After the frontend confirms each dialog it calls `apply_set_status(parentTaskId, status, false)`; cancelling keeps the current state and the next subtask event evaluates it again.

The deletion count always includes subtasks that will be taken by the cascade; `plan_*` is read-only, and the frontend uses `task_count` to fill the confirmation dialog and only calls `delete_*` after the user confirms.

**Error conventions**: a command's `Err` is always a structured message `Message { key, params }` (`src-tauri/src/message.rs`); the backend produces no user-visible natural language, and the frontend uses `translateBackendMessage` to translate it into the current language in the bottom toast. `params` only holds stable tokens, numbers and raw technical details (such as a SQLite error), never sentences; cases that need classified handling (`needs_confirm`, counts) still use fields inside `Ok` and do not go through the error channel. String errors thrown by the Tauri framework itself (command not found, parameter deserialization failure) are caught by `describe()` and turned into a complete sentence. A new backend key must be added to both the Chinese and English tables in `src-tauri/backend-keys.json` and `src/i18n/backend.ts`, and the frontend and backend each have a test watching the catalog.

**Field-name conventions**: structures crossing IPC are always camelCase (on the Rust side `#[serde(rename_all = "camelCase")]`, enum values lowercase), and frontend types correspond verbatim. Field names are owned by the backend, and the frontend does not map them separately. Several unit tests in `domain.rs` specifically watch this contract.

**Ownership parameters are validated by the backend**: `create_task` accepts `module_id`/`submodule_id`, and `domain.rs` verifies "the module belongs to that project", "the submodule belongs to that module" and "tasks in My Tasks cannot have a module". The frontend is only responsible for translating the current scope into these two ids (`resolveTarget`) and does not decide legality in place of the backend. `priority` is supplied explicitly by the caller (the frontend defaults to `mid`); when creating a subtask, `domain.rs` still clamps it to the parent's level or below, sharing the same ceiling rule as `set_priority`.

**Module order rules**: `move_module(id, before_id?)` performs ownership validation and sort-order assignment in `domain.rs` — both modules must exist and belong to the same project (a cross-project move is an error), `before_id` being `null` means move to the end, and an id equal to itself is treated as no change. When something actually changes, all modules of that project have their `sort_order` rewritten to `0..n-1` (also fixing duplicate sort orders that may exist in historical data); when nothing changes, nothing is written. Submodules do not participate in this ordering; they move with their module. The frontend does not compute sort orders; it only reports "insert before whom".

**Module name rules**: `create_module` and `rename_module` validate the trimmed name within the same project; a duplicate of another module returns the structured message `validation.moduleNameDuplicate { name }`. The frontend shows this key in a warning dialog with only a confirm button; other errors still use the bottom toast. Renaming a module to its own current name is allowed. Project and submodule names may still repeat. No unique index is added at the database level — existing databases may already contain duplicates, and adding a constraint would require a migration and cleanup; the rule stays in `domain.rs`.

**Subtask ownership rules**: when `parent_task_id` is given, the backend rejects "the parent task already has a parent" (only one level allowed) and "the parent task is not in the current container"; when a subtask is stored, `module_id`/`submodule_id` are always NULL (ignored if the caller passes them), `project_id` follows the parent task, and the sort order is queued separately as `MAX+1` under that parent task. The parent task's classification is the subtask's classification; a redundant copy is not needed or allowed.

**Parent-child conversion rules**: `reparent_task` handles moving under a parent (`Some(target_id)`) and promoting to top level (`None`) with one command, validating and updating inside a transaction, and recomputing the affected parents' status on success. For `Some(target_id)`, the target must be in the current container, have no parent and not be the task itself; the moved task must have no subtasks, otherwise grandchild tasks would appear, so it errors directly. Putting a top-level task under the target is demotion; putting an existing subtask under a different target is a **retarget** inside the same container, and both the old and the new parent are re-aggregated, so two `parent_proposals` may be returned. After the move, `module_id`/`submodule_id` are cleared, `project_id` follows the target, and the sort order is the target's subtask `MAX+1`; if the original priority is higher than the target's, it is reduced to the target's level; targeting the current parent is a no-op. Promotion (`None`) only accepts tasks that have a parent: clear `parent_task_id`, inherit the original parent's `project_id`/`module_id`/`submodule_id`, and take the top-level group's `MAX+1` as the sort order. Neither case changes the task's own status or times; parent aggregation to `done` only returns a confirmation proposal.

**Default value ownership**: the default value of `status` is written explicitly by `domain.rs` on INSERT; `priority` comes from the caller (the quick-add dropdown defaults to `mid`), and the subtask ceiling is still clamped by `domain.rs`. The schema's `DEFAULT` is only a fallback, so the same rule is not expressed in two places.

## Frontend structure

- **View state**: `container` (My Tasks / a project), `scope` (All / a module / a submodule / Default), `statusFilter: Set<Status>`, `priorityFilter: Set<Priority>`, `sort: TaskSort | null` (header column sort), `collapsedNodes: Set<nodeId>` (modules collapsed in the sidebar scope tree). Filters and column sorting are kept across scopes and projects within the session; the sidebar width is stored separately in `localStorage` (`knot.sidebarWidth.v1`).
- **Quick-add module / priority**: `composer = { target, priority }` is module-level state that applies only to the next task; it is neither written to the database nor stored locally. `target` defaults to `resolveTarget(buildGroups(...), scope)` ("All" / "Default" → the default group, a module / submodule → that node), and `priority` defaults to `mid`. Two independent buttons to the right of the input each open a flat panel through `openMenu` (in "My Tasks" the module panel only offers "Default"); a selection redraws only the corresponding trigger and the placeholder. A successful add, `applyScope`, `selectProject`, database reset and view fallbacks caused by deletion call `resetComposer()`; `composerTargetValid()` catches deleted modules / submodules at render time. When the task lands somewhere the current view cannot show, `reload()` checks `tr[data-task-id]` and shows a `toast.taskAddedElsewhere` notice.
- **Language state**: `src/i18n/index.ts` holds the module-level current language; the mode (`zh` / `en` / `system`) is stored in `localStorage` (`knot.language.v1`), and `system` is resolved from `navigator.language` only once at startup. Switching the language writes the store, updates `<html lang>` and `data-language`, reruns `applyStaticTranslations()`, refreshes the text of the theme/language/maximize buttons, and then runs a full `render()`; at the same time it clears the column-width cache so that `COLUMN_MIN` under the new language re-clamps the stored widths. Switching the language discards an in-progress inline edit; this is an accepted trade-off.
- **One tree per render**: the result of `buildGroups` is used both to compute counts for the sidebar scope tree and to group the table, and `selectScope` then applies the scope — the same tree is never computed twice, let alone with two inconsistent copies.
- **Two collapses do not affect each other**: collapsing in the sidebar scope tree (`collapsedNodes`) only controls the tree's display, and collapsing table group rows (`collapsedGroups`) only controls the table's rows. One is navigation, the other is reading density.
- **Data pipeline** (single direction, no two-way binding): `list_tasks` → `buildGroups` → `selectScope` → at render time the two filter sets decide whether a task appears; grouping and counts always use the unfiltered tree. The default in-group order is "status (done last) → priority → created time"; when `state.sort` has a value, the whole group is sorted only by the selected column. Parent tasks are sorted within the group, and subtasks are sorted under their respective parents by the same rules. `grouping.ts` is language-agnostic: the derived "Default" group's `label` is `null`, and the display name is supplied by `scopeLabel()` in `main.ts` according to the current language; module/submodule `label`s are still the user's names.
- **Table columns are fixed** as Status / Task / Priority / Created / Start / End / Note. The table body is 14.5px, and the task title and note summary share the same height; the task title wraps automatically by column width (`white-space: normal` + `overflow-wrap: anywhere`), the row height grows with the content, and the parent task row's collapse arrow / count / "+" align with the first line; the three time columns are **read-only** summaries, formatted at the display layer with `Intl.DateTimeFormat` as `2026/11/04 09:00` (24-hour clock, identical in Chinese and English; not recorded shows `—`), with no click-to-edit entry; subtask rows use the same three columns. Storage and sorting still use the raw `YYYY-MM-DDTHH:MM` text. The priority / created / start / end headers have sort buttons to their right (ascending → descending → default); times do not affect filtering or counting.
- **Hover tooltips**: the border of the native `title` is drawn by the system and cannot be changed with CSS; `initTooltips()` in `src/tooltip.ts` moves the title text to `data-tooltip` and removes `title` when the pointer enters, then shows `.hover-tip` after 400ms (a 1px thin border and 4px small radius, positioned with the pointer; white background, body text color, 13px, consistent with toasts and the column-width hint); leaving, clicking, scrolling the wheel, zooming, scrolling or losing focus hides it and restores `title`, avoiding showing both at once with the system tooltip.
- **Resizable columns**: the table draws no cell borders, and dividers appear only on hover / drag; every header divider is a drag handle, and dragging resizes the columns on both sides of the line at the same time with the total width unchanged. After one side reaches its minimum width, continuing to drag pulls the farther columns on the same side in the order `status → title → priority → created → start → end → note`, and vice versa; on hover both headers highlight together, and during a drag the current widths of the columns on both sides of the divider appear (e.g. `Created 152px · Start 80px`). The "Task" column always absorbs the remaining width, and when the window is resized only it flexes; the remaining width is computed from `.grid-scroll`'s `clientWidth`, not from the table's own `clientWidth` (the table width is expanded by the column widths, which would create a dead loop of "the table does not shrink → the task column does not shrink"). Other column widths are stored in `localStorage` (`knot.columnWidths.v1`) and reused on the next launch; when storage is unavailable, default widths are used. The minimum column width only guarantees that content does not disappear; when a time column is dragged narrow it is truncated with an ellipsis, and the full value is still available on hover.
- **Pure-function unit tests**: `src/grouping.test.ts` uses vitest to cover `sortTasks` / `buildGroups` / `selectScope` / `resolveTarget` / `countGroup` and the filter functions; `npm test` runs them directly, without starting Tauri or touching the DOM.
- **Filter semantics**: status and priority are related by "and", and within a set by "or"; a group is allowed to have nothing selected. When a parent task does not match but a subtask does, the parent task is kept as context with `.is-context` greyed out, and the subtask is shown normally; the whole parent task tree is hidden only when neither matches. A filter empty state does not delete user-created groups and does not change any counts. When a task leaves the view because its status changed to `done` and "Done" is currently unchecked, the frontend adds a 3-second toast after that row disappears (without the task title), so that a click does not appear to have no effect.
- **Interaction ownership**: clicking the status cell cycles the three states and calls `plan_set_status`; "Set status…" in the task row's right-click menu directly specifies one of the three states, and both entry points share the same `changeStatus`; if `needs_confirm` is true, a dialog appears; after `apply_set_status` returns, it re-fetches; when the target status equals the current one, nothing is written. If the result carries a `parentProposal` (all subtasks done, so aggregation suggests completing the parent), the frontend asks with `askConfirm` and calls `apply_set_status` again for the parent only after confirmation; cancelling leaves the parent unchanged. The same handling is attached after deleting a subtask and after reparenting. In-place editing (double-click) goes through `rename_task`, using `textarea.title-input` to replace only the title element itself: the width matches the cell, the height grows automatically on `input` using `scrollHeight + 2` (1px top and bottom borders), and wrapping matches the read-only `.task-title`; the left edge does not jump and the right edge aligns with the right side of the cell; the parent task's collapse arrow stays in place and aligns with the first line, while the count and "+" are hidden during editing; an empty value is restored by the frontend and also rejected by the backend; pasted line breaks are folded into spaces before submission.
- **Dialogs share one modal**: `askConfirm` (confirm after stating the impact), `askText` (enter a name when creating a project / module / submodule) and `askNote` (multi-line note). **Only one dialog may exist at a time** — a new dialog dismisses the previous one as "cancel"; otherwise two `await`s would both hang and compete for the same set of DOM listeners, and one Enter would run two commands (this has happened). A blank name in the input does not close the dialog and is not written.
- **The inline new-project form** is the non-dialog input in the creation flow (sidebar "+ New project"; structure renaming is also non-dialog, but uses the `.side-input` above). It has a single exit `resetProjectForm()` (hide + clear): a successful submit, a click outside the form (`mousedown` on `document`, not closing when inside the form) and Esc all go through it; a listener is attached when opening and removed when closing, leaving no permanent listener. Typed text is discarded along with the form — no draft is kept and it does not enter state.
- **There is only one menu implementation**: `src/menu.ts` handles opening, positioning and closing, shared by three places — the right-click menus of sidebar project names/scope nodes, the task row's right-click menu, and the priority `.menu` popover. The entry point only supplies `items` and a position (pointer point or element anchor), and the caller provides each item's behavior. The task row right-click gives top-level tasks "Set status… / Move to… / Demote to subtask… / Delete task" and subtasks "Set status… / Promote to top level / Delete task"; "Set status…" expands a three-state subpanel with the current status checked, and there is a divider before deletion. Items with `children` ("Move to…", "Demote to subtask…") do not expand inline; instead, on hover/click a separate `.menu` subpanel pops out to the right of the parent row; if there is not enough room on the right it flips to the left, and the parent panel layout does not reflow. Only one panel path is retained at a time: opening a sibling subpanel first closes the old one; scrolling the root panel closes subpanels. Opening a menu first closes the previous one; a click outside / Esc / scrolling / window blur closes it; the up and down keys cycle within the current panel, right-click enters a subpanel, left-click returns to the parent row, and Esc returns focus to the button that opened it. If the anchor cannot fit below, it flips above and is clamped within the viewport. Native `contextmenu` is `preventDefault()`-ed by default inside the app (attached once in `boot()`), and only the sidebar `contextmenu` listeners open the menus above: the project name (`#project-heading`, offering New module / Rename project / Delete project) and scope nodes each handle their own part, and a fallback listener is attached on `dom.sidebar` to handle the empty area outside project names, scope rows and buttons — when `state.projectId` is not `null` (the sidebar is showing a project and its module tree) it offers "New module" (the project name and scope rows have already `preventDefault()`-ed, and the fallback listener yields via `event.defaultPrevented`, so two menus do not appear). Switching to "My Tasks" does not clear `projectId`, so a module can still be created from empty space for the project still visible in the sidebar; with no project context, empty space shows nothing.
- **Structure rename entry points**: double-clicking a project / module / submodule name and the right-click "Rename…" both go through the same `renameNode`, which replaces the name in place with `.side-input` and shows no dialog. Enter or blur saves, Esc cancels, and an empty / unchanged value is restored by `render()`; on a successful submit it calls `rename_project` / `rename_module` / `rename_submodule` and then `reload()`, and on failure it shows a toast and calls `render()` to withdraw the input. The project name has no single-click behavior, so it uses native `dblclick` directly; a single click on a module/submodule runs `applyScope` → `render()` and replaces the node, and native `dblclick` is unreliable on the replaced element, so instead the click handler uses a module-level `scopeRenameTap` to recognize "a second click on the same node within 400ms", while the first click still switches the view as usual. "All" and "Default" are virtual scopes and have no rename.
- **Module drag reordering**: only module rows carry `data-module-id`; submodule rows do not participate in dragging. Pointer Events are used instead of HTML5 drag-and-drop: pressing only records the start point, and the pointer must move more than 4px to enter the drag state, avoiding a conflict with "single-click to switch scope / double-click to rename". During a drag, the vertical centerline of each module row determines the insertion position (insert before which row; below the centerline of the last row means the end); the dragged row is made more transparent, and a 2px accent-colored thin line appears at the target position; dropping at its current position (or immediately after itself) draws no line and sends no command. Dragging to the top or bottom edge of the sidebar auto-scrolls. On release, "insert before whom" is handed to `move_module` (`before_id` is `null` for the end), and `reload()` runs on success; the drag state exists only in this interaction and is not written to `localStorage`. One `click` is swallowed within 250ms after the drag, preventing a browser-synthesized click from switching the scope or collapsing the module. Dropping on a submodule row is computed by its module's position, and the whole module (including its submodules) moves together.
- **Fallback after deletion**: deleting a task only re-fetches the current container; when deleting a module/submodule, if the current `scope` is in the deleted subtree, return to the project's "All"; when deleting a project, if the current `container` is that project, return to "My Tasks". Before deletion, the `plan_*` count fills the confirmation dialog; the frontend does not recompute the cascade scope.
- **Subtask rendering**: `buildGroups` continues to skip tasks with `parentTaskId !== null` (they do not take a group row, and group counts include parent tasks only); at render time a separate `parentTaskId → tasks` index is built, and indented subtask rows are rendered after the parent task row with the same ordering (priority → `sort_order`). Parent task rows are expanded by default, and `collapsedTasks` only records those collapsed in this session; the collapse arrow before the parent task title and the inline "+" appear only on top-level tasks.
- **Inline subtask input**: `addingSubtaskFor` allows only one parent task at a time, and the input draft is kept in `subtaskDraft` (outside the state object, so it survives re-renders). Enter saves and keeps the input row open (so you can add continuously), Esc cancels, and blur saves non-empty content; the blur callback is deferred by one tick and checks `input.isConnected`, avoiding a re-render replacing the input and being mistaken for a submit.
- **Toast queue**: one queue at the bottom of the table, FIFO, showing one at a time, each disappearing after 3 seconds without blocking any operation. Its appearance is consistent with hover tooltips: white background, body text color 13px, 1px thin border, 4px small radius; error toasts only replace the border with the danger color, and the text stays in the body color. When one operation brings back several notices, they do not replace one another.
- **Startup failure page**: `boot()` first resolves the language, then reads `?startupError=`; if the value is JSON it translates `error` and renders `backupPath` separately, and if it is not JSON (a hand-written URL / old format) it is shown as plain text unchanged. The error page takes over with `document.body.replaceChildren` and returns directly, without even running `render()` / `reload()`.
- **Development reset**: the button is at the bottom of the sidebar, `hidden` by default, and only when `import.meta.env.DEV` is true is it unhidden and given a click binding; `vite build` strips the whole branch, and the release backend does not have this command either. The confirmation dialog uses `askConfirm`; on success the view state is reset to "My Tasks / All" and everything is re-fetched, with no process restart needed.

## Internationalization

- **Language set**: Chinese (`zh-CN`) and English (`en-US`); no third-party i18n library is introduced.
- **Mode and persistence**: the status bar language button offers three states "中文 / English / System"; the mode is stored in `localStorage` under `knot.language.v1`, and first launch defaults to `system`, decided by whether `navigator.language` starts with `zh`. The language preference does not go into the database, so no migration is needed; `system` is resolved only once at startup and does not listen for live system language changes.
- **UI dictionary**: `src/i18n/messages.zh.ts` is the single source of truth for keys, and `messages.en.ts` uses `satisfies Record<MessageKey, string>` to force full coverage; `t(key, params)` does `{name}` interpolation, and `tp(base, count, params)` uses `Intl.PluralRules` to choose `.one` / `.other`. `i18n.test.ts` verifies that the key sets and placeholder sets of the two languages are exactly identical.
- **Static text**: `index.html` keeps no natural-language text, only `data-i18n` / `data-i18n-title` / `data-i18n-aria-label` / `data-i18n-placeholder`; `applyStaticTranslations()` assigns them all at startup and on language switch. An inline script in `<head>` sets `<html lang>` early, avoiding a wrong language on the first frame.
- **Dynamic text**: `main.ts` uses stable token → message key mappings (status, priority, time, column names, sorting); enum values, `data-*`, `aria-sort` and localStorage values are never translated.
- **Backend messages**: see `message.rs` and `backend-keys.json`; `describe(error)` supports `{ key, params }`, strings and `Error` alike. The backend does not receive a language parameter, and switching the language does not affect any command.
- **Dates**: storage and sorting are unchanged; `src/i18n/date.ts` only changes the display, manually splitting `YYYY-MM-DDTHH:MM` and constructing with a local `Date`, avoiding a time-zone offset.
- **Terminology**: the Chinese ↔ English table is in `docs/i18n/glossary.md`; in code the status tokens are always `todo/doing/done` and the priority tokens are always `high/mid/low`.

## Edge cases

- Empty project / empty group: show the empty-state copy. **Empty modules/submodules created by the user are still shown** (count `0/0`) — the structure was created by the user, and hiding it would make it look like it was never created; only **derived** groups such as "Default" are not rendered when empty.
- "My Tasks" view: no module tree and no group rows; the table is a flat list (consistent with the goal document).
- "Default": tasks in the project with `module_id IS NULL` are gathered into one fixed group row placed before the module groups.
- Editing task text to blank: restore the original text and write nothing.
- A task marked done by a cascade / aggregation without ever entering Doing: only an end time is recorded, and the start time stays `NULL` (it really never started); the start time is not fabricated.
- Old tasks from before the upgrade have no creation time: show `—`, and do not pass off the migration time as the creation time.
- Migration failure: the transaction rolls back and the database stays at the old version; the main window is replaced by the error window, showing the reason and the path of the pre-migration backup, and the app never keeps running with a half-migrated database. After the cause is fixed, the next startup tries the migration again.
- The startup failure page only renders and calls no command; the error window does not host the database connection.
