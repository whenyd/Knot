# Follow-up Tasks

[中文](ROADMAP.md) | English

Execution reference for all follow-up work. Basis: `docs/specs/goal-and-requirements.en.md` (what is wanted), `docs/specs/architecture.en.md` (how it is layered, command list), and the actual current state of the code.

Before changing a capability, first check whether the corresponding task here has an unresolved "Open question"; after the change, write the spec back into goal/architecture, then come back and tick off the task.

---

## Current Status

**Implemented (23 release commands + 1 development-only command)**

```
list_projects / create_project / rename_project / plan_delete_project / delete_project
list_modules / create_module / create_submodule / move_module / rename_module / rename_submodule
plan_delete_node / delete_node
list_tasks / create_task / rename_task / set_note / move_task / set_priority
plan_set_status / apply_set_status
plan_delete_task / delete_task
reset_database   // registered in dev builds only; not registered in release, and no entry is shown in the frontend
```

Interface: sidebar (My Tasks + projects + scope tree + **right-click menus / double-click in-place rename / module drag reordering**), a table always grouped by module (group collapse, subtree counts, **automatic title wrapping**),
clicking status cycles the three states, priority popover, double-click in-place rename, **new tasks land in the current scope, plus two module / priority dropdowns to the right of the input**, in-app confirmation dialogs, input dialogs and
**multi-line note dialogs**, **three columns for Created / Start / End (recorded automatically, read-only)**, **draggable column widths**, **task row right-click menu** (set status + move to any classification + delete),
**three-level structure deletion** (the confirmation dialog states the number of affected tasks; the view falls back automatically after deletion),
**one level of subtasks** (inline "+" on the parent row for continuous adding, indented by 4 spaces, expanded by default and collapsible),
**parent-child status aggregation** (triggered by three events, cascade confirmation, FIFO toast queue),
**status / priority filtering** (done hidden by default, parent and child matched separately, a non-matching parent greyed out as context, full counts),
**data safety** (automatic pre-migration backup keeping 5 copies, a migration-failure error window with the reason and backup path, database reset and rebuild in dev builds),
**Chinese/English internationalization** (three-state language switch in the status bar, type-safe dictionaries, structured backend messages, date localization, installer language selection).

**In the spec but not in the code**

| Capability (goal-and-requirements) | Gap |
| --- | --- |
| Three-level structure can be created | ✅ Done (Phase 1 T1/T2) |
| Three-level structure can be renamed | ✅ Done (Phase 1 T3) |
| Deletion (task / module / submodule / project) | ✅ Done (Phase 2 T4–T7) |
| Classification and cross-container moves | ✅ Done (Phase 3 T9) |
| Note | ✅ Done (Phase 3 T8) |
| Created time | ✅ Done (Phase 8 T25) |
| Start / end times (recorded automatically with status) | ✅ Done (Phase 8 T23/T24) |
| Default window width | ✅ Done (Phase 9 T26) |
| Adjustable column widths | ✅ Done (Phase 9 T27) |
| Task right-click menu (status / move / delete) | ✅ Done (Phase 9 T28, Phase 13 T41) |
| Quick-add module / priority dropdowns (two) | ✅ Done (Phase 20 T53) |
| Subtasks (one level) | ✅ Done (Phase 4 T10/T11) |
| Parent-child status aggregation | ✅ Done (Phase 4 T12–T14) |
| Parent-child conversion (demote to subtask / promote to top level) | ✅ Done (Phase 9 T37) |
| Status / priority filtering | ✅ Done (Phase 5 T15/T16) |
| Migration safety | ✅ Done (Phase 6 T17–T19) |
| Engineering | ✅ Frontend unit tests (T20); release exe actually run (T21; installer skipped per the archive convention) |

---

## Open Questions

### D1 · When a module is deleted, where do its tasks go? — **Decided: deleted along with it after confirmation**

- Conclusion: deleting a module/submodule **really deletes** the tasks inside it (the count is stated in a dialog first), and they are not returned to Default.
- Already synced: the "Deletion" section of `docs/specs/goal-and-requirements.en.md` no longer says "(inferred)"; the schema in `docs/specs/architecture.en.md` changes `task.module_id` / `submodule_id` to `ON DELETE CASCADE` and explains why a v2 migration is needed (v1 was released as `SET NULL`, and historical entries cannot be changed).
- Associated conclusion: deleting a project likewise "takes all of its tasks" — `project_id` was already `CASCADE`, so no migration is needed.

### D2 · Entry points for creating submodules and modules — **Decided: sidebar right-click menus**

- Project row right-click → New module; module row right-click → New submodule.
- Feasible: `contextmenu` is an ordinary DOM event in Tauri's webview; `preventDefault` the native menu and show a self-built menu (the same DOM layer as the confirmation dialog).

### D3 · How to rename projects / modules — **Decided: right-click menu + sidebar double-click**

- Sidebar project name right-click → "Rename project"; project names, module names and submodule names can all be **renamed directly by double-clicking**.
- A single click still "switches the view": the first click on a module/submodule re-renders the sidebar, and native `dblclick` is unreliable, so "a second click on the same node within 400ms" is used to recognize it; the project name has no single-click behavior and uses native `dblclick`.
- Double-click applies only to structure names; double-clicking a task title is still in-place editing.
- Renaming uses **in-place input**: the name itself becomes a single-line `.side-input`; Enter / blur saves, Esc cancels, and an empty or unchanged value is restored; no dialog, and the task title's multi-line textarea is not reused.

### D4 · The default view changes when filtering lands

The goal states that the status chips are **checked by default only for "Todo" and "Doing"**, which means that once filtering is done, "Done" is **hidden** by default, whereas today they are always shown (greyed out + strikethrough). This is the behavior required by the spec; do not treat it as a bug when the time comes.

### D5 · Right-click is undiscoverable (unconfirmed; suggested remedy)

The right-click menus have no visual hint, so first-time users will not know they can right-click; it is also unfriendly to keyboard-only users. Suggestion: attach the same menu to a **`⋯` button that appears on hover** (one menu component, two triggers); the cost is small. **Not yet confirmed.**

### D6 · Internationalization — **Decided: Chinese/English bilingual + structured backend messages**

- Language entry: a three-state "中文 / English / System" switch in the status bar, defaulting to following the system, stored in `knot.language.v1`; no new database settings table.
- Copy ownership: the frontend dictionary `src/i18n/**`; all backend `Err`s, `plan_set_status.reason` and `notices` change to `Message { key, params }`, translated by the frontend; the contract file is `src-tauri/backend-keys.json`.
- English terminology: Todo / Doing / Done, High / Medium / Low, Default, My Tasks; stable tokens (`todo/doing/done`, `high/mid/low`) and `data-*` are not translated.
- Dates: localized at the display layer; storage and sorting still use `YYYY-MM-DDTHH:MM`.
- Comments: source comments are in English; released migrations only change comments, with not a single character of SQL changed (the wording in CLAUDE.md is relaxed accordingly).
- Docs: Chinese is the authoritative version, and the English mirror sits next to the Chinese; the glossary is in `docs/i18n/glossary.md`.
- Installers: NSIS with Simplified Chinese + English and a language selector; MSI's `zh-CN` needs real testing, and if it fails, fall back to English or publish NSIS only.

---

## Tasks

The numbers are the suggested execution order; tasks within the same phase can run in parallel, and the "Depends on" column names those that cannot be advanced.

### Phase 1 · Sidebar menus and structure maintenance

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T1~~ | ✅ **Sidebar right-click menu skeleton** (done, `src/menu.ts`) | `main.ts`: custom menu component (`contextmenu` → `preventDefault` → position → close: click outside / Esc / scroll); attached to project rows and scope nodes separately | Right-clicking a project row, module row or submodule row all opens it; only one menu at a time; the menu never runs outside the viewport | — |
| ~~T2~~ | ✅ **New module / submodule** (done; the toolbar "+ Module" button has been removed and the entry is unified into the right-click menu) | `domain::create_submodule` (validate the module exists, sort order `MAX+1` within the group) + `commands` + `api.ts` + the menu items from T1 | After creation both the sidebar scope tree and the table sub-group rows show it; duplicate names are not blocked (the spec does not require uniqueness) | T1 |
| ~~T3~~ | ✅ **Rename project / module / submodule** (done, via dialog input) | `domain::rename_project` / `rename_module` / `rename_submodule` (reuse `require_name`'s trim + reject empty) + menu item → dialog input | A blank name is rejected and the database is unchanged; the sidebar and table group rows update together | T1 |

### Phase 2 · Deletion (two-step) — Done

Always the `plan_*` / `apply_*` shape; the confirmation dialog reuses the existing `askConfirm`, and the entry reuses the T1 menu.

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T4~~ | ✅ **Delete task** (done) | `plan_delete_task(id) -> { taskCount }` / `delete_task(id)` + a "…" menu at the end of the task row (only delete in the first version) | The confirmation dialog states the count (including subtasks); after deletion the list has one fewer row | — |
| ~~T5~~ | ✅ **Delete module / submodule** (done) | **Migration v2**: create a new task table → copy data → drop the old table → rename → rebuild indexes, changing `module_id` / `submodule_id` to `ON DELETE CASCADE`; then `plan_delete_node(kind, id)` / `delete_node(kind, id)` + menu items (`kind = module` / `submodule`) | An existing database can upgrade to v2 and pass `foreign_key_check`; deleting a module makes its submodules and tasks disappear together; the count includes tasks in submodules; if the deleted scope is currently open, fall back to "All" | D1, T1 |
| ~~T6~~ | ✅ **Delete project** (done) | `plan_delete_project(id)` / `delete_project(id)` + menu item | The count includes all tasks in the project; after deletion, if the current view is that project, fall back to "My Tasks" | D1, T1 |
| ~~T7~~ | ✅ **Merge the remaining two menus into `src/menu.ts`** (done) | `src/menu.ts` is already a generic component (built in T1); the task row menu (introduced in T4) uses it directly; the priority popover (the original `.picker` in `main.ts`) also switches to it | Only one "open/position/close" implementation; the priority popover's arrow-key and Esc behavior is unchanged | T1, T4 |

### Phase 3 · Notes and classification moves — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T8~~ | ✅ **Note dialog** (done) | `domain::set_note` (store as-is; empty string = clear) + click the note cell to open a dialog (reuse the modal skeleton; `askNote` uses a textarea, and Enter is a newline) | After saving arbitrary text (including newlines), the table shows a single-line truncated summary; an empty note shows `—` | — |
| ~~T9~~ | ✅ **Classification move** (done) | `domain::move_task(id, target)` (ownership validation shares `require_task_target` with `create_task`; subtasks follow their parent to a new container) + the "…" menu → "Move to…" submenu | Movable both ways; after moving, `sort_order` takes the end of the target group; for a cross-container move both views must be re-fetched | T7 |

### Phase 4 · Subtasks and status aggregation — Done

The order inside this phase **cannot be changed**: without T10 / T11, T12 has no data to aggregate.

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T10~~ | ✅ **New subtask** (done) | `create_task` gains `parent_task_id` and returns `{ task, notices }`; the backend validates the one-level limit and container consistency, forces `module_id`/`submodule_id = NULL` for subtasks, and queues the sort order as `MAX+1` within the parent task group | A subtask cannot have a subtask (the backend rejects it); a subtask never appears in any group row | — |
| ~~T11~~ | ✅ **Subtask rendering** (done) | `main.ts` builds an index by `parentTaskId` and renders indented subtask rows both in the flat My Tasks view and in the grouped table; the parent row's collapse arrow is expanded by default, and the inline "+" adds subtasks (Enter adds continuously, Esc cancels, blur saves non-empty) | Group counts still include parent tasks only | T10 |
| ~~T12~~ | ✅ **Parent-child status aggregation** (done) | `aggregate_parent_status` in `domain.rs` + three triggers (subtask status change, subtask added, subtask deleted), same transaction; the literal rule includes vacuous truth for zero subtasks; `showNotice` changes to a FIFO queue | A toast appears without blocking; **manually moving a parent task back leaves its subtasks unchanged**; multiple notices queue instead of replacing one another | T10, T11 |
| ~~T13~~ | ✅ **Cascade confirmation becomes live** (done) | The frontend's existing `needs_confirm` branch and `askConfirm` become actually reachable for the first time after T10; add tests | When a parent task has unfinished subtasks and is marked done → a dialog states the count, the confirm button is "OK" → on confirmation they are completed together | T10 |
| ~~T14~~ | ✅ **Deleting a subtask goes through aggregation** (done) | `delete_task` changes to delete + aggregate in one transaction, returning `DeleteOutcome { notices }` | Deleting the last unfinished subtask → the parent becomes `done` + toast | T12, T4 |

Phase 4 implementation decisions:

- The subtask entry point uses the inline input opened by the "+" on the parent task row; after Enter the input row stays open so you can add continuously; the parent task row has a collapse arrow and is expanded by default, and the collapsed state is remembered only within the session.
- Aggregation is implemented by the literal rule: with zero subtasks `all()` is true, so deleting the last unfinished subtask makes the parent `done`.
- A subtask's priority is capped by its parent: it defaults to "Medium" on creation and follows the parent when the parent is lower; when the parent is lowered, subtasks still above it are cascaded down, and old data is normalized by the v3 migration.
- The return structures of `create_task` / `delete_task` change to objects carrying `notices`; notices are produced only when a parent task is changed automatically.
- Toasts change to a FIFO queue, showing one at a time (each for 2s).

### Phase 5 · Filtering — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T15~~ | ✅ **Two groups of chips at the top** (done) | `main.ts` toolbar + `state.statusFilter` / `state.priorityFilter` + filter functions in `grouping.ts` (chained with the existing `selectScope` in the same pipeline) | By default only Todo/Doing are checked for status and all priorities are checked; the two groups and the scope apply **together** | — |
| ~~T16~~ | ✅ **Finish the done-task presentation** (done) | Same as above; when a parent task does not match but a subtask does, the parent row is kept as greyed-out context, and subtasks are filtered independently | When "Done" is checked, greyed out + strikethrough; when unchecked the row disappears but **group and scope-tree counts are still shown** (e.g. `3/7`) | T15 |

Phase 5 implementation decisions:

- The default view changes from "done tasks visible" to "done tasks hidden" (D4, implemented).
- Status and priority filters are kept across scopes and projects within the session and are not written to the database.
- When a parent task does not match but any subtask does, the parent is greyed out as context and only matching subtasks are rendered; the whole tree is hidden only when neither parent nor children match.
- Filtering allows a group to have nothing selected and shows a filter empty state; user-created groups are kept, and counts are always based on unfiltered data.

### Phase 6 · Data safety — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T17~~ | ✅ **Automatic pre-migration backup** (done) | `db.rs` / `migrations.rs`: before upgrading an old database, copy it to `tasks.sqlite.bak-v<old version>-<epoch milliseconds>`, keeping only the latest 5 | Starting a database with an old `user_version` → a backup is generated; after 6 consecutive runs only 5 remain; manually placed `.bak-import-*` files are not cleaned up | — |
| ~~T18~~ | ✅ **Migration failure does not crash bare** (done) | `lib.rs` setup no longer uses `?`/panic: hide the main window, open an error window, load `index.html?startupError=…`; the frontend `boot()` takes over rendering on its first line | On failure the window directly shows the backup path and the reason, calling no commands; the database stays at the old version | T17 |
| ~~T19~~ | ✅ **Dev database reset and rebuild** (done) | Register `reset_database` in debug only + a button at the bottom of the sidebar (shown only when `import.meta.env.DEV`); the whole reset holds the lock, swaps connections, deletes the live database and rebuilds | Release builds do not register the command and do not show the button; in dev builds clicking it returns the view to an empty database | — |

Phase 6 implementation decisions:

- A backup is generated only when `0 < user_version < latest version`; a new database and a startup at the same version are not backed up. If the backup fails, migration stops.
- `PRAGMA foreign_key_check` moves into the migration transaction, before committing; on failure it rolls back `user_version` as well, and the next startup still refuses.
- The error window is created independently with `WebviewUrl::App("index.html?startupError=…")` and does not depend on the timing of `window.url()` in the setup phase; the reason takes only the first line of the SQLite error, and the backup path comes first, so both are visible without scrolling.
- Database reset and rebuild does not restart the process: hold the lock and swap the connection out → delete the main file and sidecar files → reopen and migrate → put it back; `.bak-*` backups are kept, and if the main file cannot be deleted the original database is reconnected.
- New unit tests cover backup generation / keeping 5 copies / failure with a path / reset and rebuild / FK check rollback; after internationalization all 117 backend tests pass.

### Phase 7 · Engineering

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T20~~ | ✅ **Frontend pure-function unit tests** (done) | Introduced vitest 5; `src/grouping.test.ts` covers `sortTasks` / `buildGroups` / `selectScope` / `resolveTarget` / `countGroup` in `grouping.ts` (plus the filter functions) | `npm test` green (30 tests after internationalization); the `resolveTarget` submodule case guards "moduleId must also be provided", and deliberately breaking the implementation turns that case red | — |
| ~~T21~~ | ✅ **Release build actually run** (done; installer skipped per the archive convention) | `npx tauri build --no-bundle` (no bundler); release exe | Start the release exe once: after creating a project/module/task via CDP and deleting the module, `plan.taskCount=1`, and in the database `tasks_in_module=0`, `total_tasks=0`, `foreign_key_check=[]`, `user_version=3`; database path `%APPDATA%\knot\tasks.sqlite`. The system has no WiX/NSIS, the project is archived, and no bundler is downloaded anymore | T17 |
| T22 | (Optional) CI | GitHub Actions: `cargo test` + `tsc` + `vitest` | Runs automatically on every PR | T20 |

Phase 7 implementation decisions:

- Frontend unit tests use vitest 5: `npm test` is `vitest run`, configured in `vitest.config.ts`, collecting only `src/**/*.test.ts`, running in the node environment without starting Tauri or touching the DOM.
- Test data is created directly as `Task` / `Module` with factory functions; the `resolveTarget` submodule case asserts both `moduleId` and `submoduleId`, serving as the guard case that "turns red if the implementation degenerates to only returning submoduleId".
- The test file is under `src/`, so `tsc --noEmit` also checks it; `vite build` does not pack tests into the output.
- T21 uses only capabilities already on the system: there is no WiX/NSIS on this machine, the project is archived, and no bundler is downloaded anymore; instead `npx tauri build --no-bundle` verifies the release exe and the database/FK behavior, and the installer part is skipped.
- UI wording is unified as "My Tasks" and "Default": the former is the display name of the virtual container (before internationalization it came from the backend `TODO_NAME`; it is now rendered by the frontend from `isTodo`), and the latter is the derived group with an empty module classification; source code, error copy, docs and tests are changed together, avoiding two names telling different stories.
- The status filter chips lose the round pseudo-element, and the task priority column and popover lose the dots: text and color already convey enough, and duplicate dots make the filter row look crowded; the row-start status button is kept, otherwise the status cannot be changed.
- The native context menu is globally disabled inside the app, and only the custom menus on sidebar project/scope nodes take over `contextmenu`; the table and empty space no longer pop up irrelevant browser items such as "Save as / Inspect".

### Phase 8 · Task times — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T23~~ | ✅ **Start / end time fields and two-column display** (done) | Migration v4 adds `start_at` / `end_at` to `task`; the table gains two read-only summary columns "Start" and "End" | After an old database is upgraded the two columns are NULL; the two columns show `YYYY-MM-DD HH:MM`, and `—` when not recorded | — |
| ~~T24~~ | ✅ **Times change to automatic recording with status** (done) | `write_status` in `domain.rs` maintains the two times on status changes (entering Doing records the start, entering Done records the end, reverting to Todo clears both); the manual-write command and edit dialog are deleted; aggregation / cascade automatic changes also go through this path | When cycling the three states the times appear and clear according to the rules; cascade completion and parent auto-done / restore all maintain times; there is no manual edit entry in the UI; times do not participate in sorting, filtering or counting | T23 |
| ~~T25~~ | ✅ **Created time column** (done) | Migration v5 adds `created_at` to `task`; `create_task` writes it once; the table adds a read-only "Created" column before "Start" | A new task (including subtasks) immediately has a created time; status changes do not change it; old tasks from before the upgrade show `—` | T24 |

Phase 8 implementation decisions:

- Times are stored as local-time text `YYYY-MM-DDTHH:MM`, not as timestamps: a single machine and single user need no time-zone conversion, and with a fixed format string comparison is chronological order; the clock uses `chrono::Local`.
- Times are a **by-product of status writes**, not independent fields: `write_status` is the only entry point for changing status and writes `status` / `start_at` / `end_at` in the same `UPDATE`, so the rule lives in one place only.
- Writing the same status again does not overwrite existing times; reverting to Todo clears both, so every restart is a new round.
- A task marked done by a cascade / aggregation without ever starting gets only an end time, keeps the start `NULL`, and does not fabricate a start.
- Parent tasks and subtasks use the same rules; aggregation automatically changing a parent task also goes through `write_status`, so time and status never separate.
- The table time columns are read-only and cannot be clicked into; the previous manual-edit command `set_task_schedule` and the `askSchedule` dialog have been deleted, avoiding two write entry points for the same field.
- Migration v4 only does `ALTER TABLE ADD COLUMN` and does not touch existing rows.
- The created time is appended as `created_at` by v5, likewise only adding a column; tasks from before the upgrade stay `NULL`, and the migration time is not passed off as the creation time. A new task's created time is written in the same INSERT as the task itself in `create_task`.
- The three time columns in the table are all read-only summaries; "Created" is fixed before "Start", the column widths are tightened and the time font is reduced to 11.5px, leaving room for the task column.
- In a real window, a temporary task was created via CDP and walked through `Todo → Doing → Done → Todo`: entering Doing showed only the start time, on completion the start time stayed and the end time appeared, and reverting to Todo cleared both; the created time stayed unchanged at every step; each step read the fields back from the real backend to verify, and the task was deleted afterwards.

### Phase 9 · Window and table interactions — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T26~~ | ✅ **Increase the default window width** (done) | `src-tauri/tauri.conf.json`: default width 1040 → 1280 | Newly started windows are wider; height and minimum size unchanged | — |
| ~~T27~~ | ✅ **Draggable column widths** (done) | `index.html` header dividers + `main.ts` drag logic + `styles.css` handle styles; the task column absorbs the remaining width; non-task column widths stored in `localStorage` | Dragging only adjusts the columns on both sides of the line; the task column flexes with the window; column widths persist after restart; when the window is too narrow the task column has a minimum width and the table can scroll horizontally | — |
| ~~T28~~ | ✅ **Move / delete tasks via row right-click** (done) | Remove the trailing "…" and the action column; `renderTaskRow` attaches `contextmenu`, with menu items "Move to…" (top-level tasks only) and "Delete task" | Right-clicking anywhere on a task row opens the menu; subtasks only have delete; right-clicking in an input does not steal the menu; the move menu reopens at the right-click position | T27 |
| ~~T29~~ | ✅ **Column resize interaction improvements** (done) | Add permanent thin dividers between columns; hovering highlights both headers; dragging shows both column widths; minimum column width relaxed | Dividers are visible in both the header and the body; both sides resize while dragging with a width hint; a narrow time column is truncated with an ellipsis and the full value is visible on hover | T27 |
| ~~T30~~ | ✅ **Time font size and column order** (done) | Time column font 11.5px → 13px, default width 116 → 128px; column order changed to Status / Task / Priority / Created / Start / End / Note, and header drag dividers moved accordingly | Time readability improves and is not truncated by default; priority comes before created; after reordering, dragging still only affects the two sides | T29 |
| ~~T31~~ | ✅ **Task title and note summary font size** (done) | `.task-title` / `button.note` / `.title-input` / `.subtask-input` font size 14px → 13px | Title and summary look the same height; font size does not jump when double-click editing or adding a subtask inline | T30 |
| ~~T32~~ | ✅ **Default group moved forward and subtree highlight** (done) | `grouping.ts` places unclassified before modules; `main.ts` syncs the sidebar scope tree "All / Default / modules"; after a priority change and "Move to…", the parent row + subtask rows highlight together for 1 second | "Default" is before modules in both the sidebar and the table; the whole subtree flashes when subtasks move with their parent; the new position also highlights when visible after a successful move | T31 |
| ~~T33~~ | ✅ **Toast duration and copy convergence** (done) | In `main.ts` all toasts are 2 seconds; the delete-task confirmation dialog and the parent auto-status notice no longer carry the task title; the auto-done / restore notices in `domain.rs` change to generic copy | All toasts disappear after 2 seconds; the delete confirmation only states the count and that it cannot be undone; auto-done / restore notices do not include a specific task name | T32 |
| ~~T34~~ | ✅ **Completion-hide feedback** (done) | In `main.ts`, `cycleStatus`, when a task is marked done and the current "Done" filter is unchecked so the row leaves the view, adds "Task completed and hidden from the current view"; the copy does not include the task title | When completing a task makes the row disappear there is still 2-second feedback; if "Done" is already checked, the original behavior is kept with no extra notice | T33 |
| ~~T35~~ | ✅ **Hover tooltips and table micro-interactions** (done) | `tooltip.ts` takes over the native `title` and adds `.hover-tip`; `beginRename` replaces only the title element; `beginColumnResize` changes to cascade width allocation in column order | Hover tooltips and toasts both become white background, body text 13px, thin border and small radius; the double-click rename input's left edge does not jump and its right edge aligns with the cell, the parent task arrow stays in place, and the count/+ are hidden during editing; dragging a column to the minimum then continues to pull the adjacent columns, and vice versa | T34 |
| ~~T36~~ | ✅ **Subtask progress drives the parent task** (done) | `domain.rs` `aggregate_parent_status` derives the parent task from the subtasks' overall status: all `done` → `done`; some `doing` / `done` but not all done → `doing`; all `todo` → `todo`; a notice only when the status is actually rewritten | As soon as a subtask enters doing or is completed, the parent task immediately shows doing; when all subtasks are unfinished the parent returns to unfinished; when all are done it becomes done directly; when the parent is already the target status no notice is repeated | T12 |
| ~~T37~~ | ✅ **Parent-child conversion** (done) | `domain.rs` adds `ReparentOutcome` and `reparent_task`: `Some(target)` demotes, `None` promotes; `commands.rs` / `lib.rs` registration; `api.ts` adds `reparentTask`; `main.ts` adds "Demote to subtask… / Promote to top level" to the task right-click menu, with targets grouped by the current container | A top-level task can be demoted to a subtask of any top-level task in the same container, with module/submodule cleared, placed at the end of the target's subtasks, and priority not higher than the new parent; a parent task with subtasks is refused demotion with a notice; a subtask can be promoted to a top-level task, inheriting the original parent's classification and placed at the end of the group; both cases recompute the parent task's status and highlight/notify | T36 |

Phase 9 implementation decisions:

- The default window width changes only `width`; the height 720 and the minimum width 720 stay unchanged; the column-width adjustment makes narrow windows usable too.
- Column widths are saved as pixel values for fixed columns; the `Task` column is not saved: it is always computed as "container width − the other columns", so window resizing only affects it while the other columns keep the user's settings.
- The header only adds dividers between adjacent columns and does not provide "drag outside the window to widen" behavior; dragging shrinks and grows both sides at the same time, with the total width unchanged.
- The task row right-click pops the menu at the context coordinates; choosing "Move to…" from the menu closes the root menu and reopens the move tree at the original coordinates. Right-clicking an inline input is reserved for the input and does not pop the task menu.
- After removing the trailing action column, the `colSpan` of group rows and the subtask input row changes from 8 to 7.
- Divider lines are permanent: a 1px dark line in the header and a 1px light line in the body; the drag handle is 9px wide, and on hover it lights up the headers on both sides of the line.
- During a drag an overlay shows the current widths of both columns (e.g. `Created 152px · Start 80px`), making "both sides change together" visible; it disappears on release.
- The minimum column width is relaxed to 36–100px, leaving room for dragging; when a time column is too narrow it uses an ellipsis, and the full value is still in the hover title.
- The time column font goes from 11.5px to 13px and the default width from 116px to 128px, ensuring that a 16-character time is fully displayed in the default layout.
- The column order moves priority before created / start / end; the header dividers change accordingly to `title:priority`, `priority:created`, etc.; the drag logic itself is unchanged.
- Task titles and note summaries are unified at 13px; the inputs for double-click rename and inline subtask creation are also 13px, avoiding a font-size jump when entering edit mode.
- The "Default" group is placed before modules by `buildGroups`, and the sidebar scope tree renders in the same order, avoiding two orderings between the table and the navigation.
- The scope of the sort flash expands from a single row to the parent task + its subtasks (subtasks move with the parent); "Move to…" goes through the same flash, and the CSS animation duration is unified at 1 second.
- Toasts no longer distinguish dwell time by info / error and are uniformly 2 seconds; errors also queue at the same pace, avoiding one error blocking subsequent operation feedback for a long time.
- The delete-task confirmation dialog keeps only "task / subtask count + cannot be undone", and the parent auto-done / restore notices only state the status change, no longer concatenating the task title.
- When a task is completed and "Done" is currently unchecked, the row immediately leaves the view; if after the re-render the row is no longer in the DOM, a 2-second toast explains that the task was completed and hidden, avoiding a click that appears not to take effect.
- The border and radius of the native `title` are drawn by the system and cannot be overridden with CSS; `tooltip.ts` takes over the text and removes `title` when the pointer enters, drawing a 1px thin border and 4px small radius with `.hover-tip`, and restores it on leave, avoiding two tooltips appearing at once.
- Double-click rename replaces only the title `span` with an input, no longer clearing the whole cell: the collapse arrow before the parent task title stays in place, the input's left edge aligns with the original text and its right edge extends to the right of the cell; the count and "+" are temporarily hidden during editing, avoiding squeezing the input narrow. Subtasks are already full-cell width, so their behavior is unchanged.
- Column-width dragging changes to a cascade from a starting snapshot in column order: dragging right makes the right side yield space from near to far and the left side only gains what is actually yielded; dragging left is the reverse. After one side reaches the minimum width, continuing to drag pulls farther columns, and the total width always stays unchanged; at the end only the fixed column widths are written back to `localStorage`, and the task column is still computed from the container's remaining width.
- The table removes cell borders: `thead th` / `tbody td` no longer draw vertical lines between columns or horizontal lines between rows, and group rows no longer draw top/bottom lines; only a single divider below the header remains. Column boundaries are distinguished by alignment, whitespace and hover background; the drag handle position is unchanged, and vertical lines still appear on hover / drag to indicate the boundary.
- The candidates for "Demote to subtask…" are sorted with `sortParentCandidates` in `grouping.ts`: unfinished / doing first, done after; within the same status by created time ascending, falling back to `sortOrder` / `id` when times are equal (or not recorded). The menu is still grouped by Default / module / submodule, and sorting applies only within a group.
- The in-group ordering of the table and subtasks changes to "status → priority → created time": the unfinished / doing group comes first and the done group after, each ordered internally by priority → created time ascending (`sortTasks`); start / end times still do not participate in the default sort. An empty created time (old data from before the upgrade) counts as the earliest, falling back to `sortOrder` / `id` for the same time.
- The priority / created / start / end columns gain sort buttons (`index.html` header + `cycleColumnSort` / `renderSortIndicators` in `main.ts`): clicking cycles ascending → descending → default, with the arrow synced to `aria-sort`; `sortTasks` accepts `TaskSort | null`, and once a column is selected the whole group is sorted by it (no longer moving done tasks last), while unrecorded times always go last. Sorting only affects the render order, does not change `sortOrder` and does not write to the database; grouping is still preserved, parent tasks are sorted within the group, and subtasks are sorted under their respective parents by the same rules.
- The sidebar width is draggable: a zero-width `.sidebar-resizer` sits in `.workspace`, with a 10px transparent drag area straddling both sides of the sidebar border; dragging changes `flex-basis` live, the width is 180–420px and the main area keeps at least 360px, and on release it is written to `localStorage` (`knot.sidebarWidth.v1`). A vertical line appears only on hover / drag / keyboard focus; the left and right arrow keys adjust by 16px each time.
- When the window is resized, fixed columns keep their pixel width and only the "Task" column flexes; the remaining width is read from `.grid-scroll.clientWidth`, avoiding reading the table's own width which would make the table not shrink when the window shrinks and leave only horizontal scrolling. A sidebar width change also triggers the same recomputation.
- The table font is enlarged overall: body / task title / note 13.5px → 14.5px, time 11.5px → 12.5px, header 12px → 12.5px, priority 12.5px → 13.5px, secondary text such as counts 12px → 12.5px; the edit inputs for titles, subtasks and notes are synced to 14.5px, avoiding a font-size jump when entering edit mode. The default width of the three time columns goes from 132px to 140px, ensuring a 16-character time is fully displayed at 12.5px.
- The parent task status is derived from the overall status of all subtasks: all `done` → `done`; otherwise, as soon as any subtask has started (`doing` / `done`) → `doing`; all `todo` → `todo`. When the parent is already the target status, no notice is repeated; when reverting from `done` the copy is "restored to…", and all other automatic rewrites use "marked as … automatically".
- Parent-child conversion shares one `reparent_task`: a value in `parent_task_id` means demote, `None` means promote to top level. Only the same container and a one-level relationship are allowed; a parent task with subtasks cannot be demoted directly, avoiding grandchild tasks; demotion clears the module/submodule and lowers the priority to no higher than the new parent, while promotion inherits the original parent's classification; both directions update and recompute the parent task's status in the same transaction.

### Phase 10 · Sidebar shortcut entry — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T38~~ | ✅ **Right-click empty sidebar space to create a module** (done) | `main.ts` attaches a `contextmenu` fallback on `dom.sidebar`: skip project rows / scope rows / buttons (they handle their own or have already `preventDefault()`-ed), and when there is a current project show "New module" | In a project view, right-clicking any empty space in the sidebar opens "New module" through the same `createModuleIn` dialog; in the "My Tasks" view it also works if the sidebar still shows a project context, and shows nothing if no project has ever been selected; right-clicking a project row / scope row still opens only its own menu, with no duplicate | — |

Phase 10 implementation decisions:

- The empty-space entry only adds "New module" and does not duplicate rename / delete: those operations must land on a definite node, and empty space has no "current node".
- The project context comes from `state.projectId`: switching to "My Tasks" does not clear it (the sidebar still shows the project name and module tree), so right-clicking empty space still creates a module for the project shown in the sidebar; it only shows nothing when no project has ever been selected or the project was deleted so it is `null`.
- The fallback listener is attached to `aside.sidebar` and first checks `event.defaultPrevented`: the listeners on project rows and scope rows have already `preventDefault()`-ed and shown their own menu, so the fallback yields when the event bubbles up. Then `closest(".side-item, .scope-row, button, input")` excludes right-clicks on buttons such as the dev-build "Reset database".

### Phase 11 · Task title wrapping — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T39~~ | ✅ **Automatic wrapping in the task column + edit box adaptation** (done) | `styles.css`: `.task-title` changes to `white-space: normal` + `overflow-wrap: anywhere`, and the parent task row aligns by its first line; `main.ts`: `beginRename` changes to an auto-growing `textarea.title-input`, and pasted line breaks are folded into spaces | Long titles (including long strings without spaces) wrap within the task column and the row height grows with the content; after double-clicking the edit box also wraps with the column width and grows with input; the existing behaviors of Enter to save, Esc to cancel and blur to save are unchanged | — |

Phase 11 implementation decisions:

- Only the task column wraps; time and note columns still keep single-line truncation: these columns are fixed-width summaries, and wrapping would make the table lose alignment.
- The parent task title row is a flex row: `align-items` changes from `center` to `flex-start`, keeping the collapse arrow, count and "+" on the first line; `margin-top` compensates for the height difference between the 20px line height and the 14 / 18px icons.
- The edit box changes from `<input>` to `<textarea rows=1>`, with `resize: none` + `overflow: hidden`; on the `input` event the height is reset to zero and then expanded by `scrollHeight + 2` (top and bottom borders); `font: inherit` keeps the global 20px line height, so the wrap position is the same before and after editing.
- Enter still submits rather than inserting a newline; when pasting text with line breaks, `replace(/\s*\n+\s*/g, " ")` is applied first and then trim, keeping the title itself single-line; line breaks are only for display.

### Phase 12 · Sidebar structure renaming — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T40~~ | ✅ **Project name right-click + double-click structure name in-place rename** (done) | `main.ts`: attach a right-click menu (New module / Rename project / Delete project) and double-click to `#project-heading`; `renderScopeNode` recognizes a second click on the same node within 400ms in the module / submodule click → `renameNode`; `renameNode` replaces the name in place with `.side-input`, no dialog; `.side-heading` gains `user-select: none` | Right-clicking the project name shows "Rename project"; double-clicking a project / module / submodule name all turn it into an in-place input, Enter / blur saves, Esc cancels, with no dialog at any point; the first click of a module double-click still switches the scope as usual; double-clicking "All" / "Default" does not enter rename; right-clicking the project name shows only one menu | — |

Phase 12 implementation decisions:

- The project name right-click is completed into a full project menu (New module / Rename project / Delete project), consistent with the entries on empty sidebar space and the "All" node; renaming always goes through `renameNode`.
- Renaming shows no dialog: `renameNode` finds the name element and replaces it in place with `.side-input`; the project name replaces the content of `#project-heading`, and a module / submodule replaces the `.scope-name` button. Enter / blur submits, Esc cancels; an empty value or one equal to the original name is restored by `render()`.
- Submission goes through the existing `rename_project` / `rename_module` / `rename_submodule`, and on success `reload()`; on failure a toast explains the reason and `render()` withdraws the input. The single-line input cannot write a newline into a structure name.
- The first click on a module / submodule calls `render()` and rebuilds the sidebar, replacing the original element, so native `dblclick` is unreliable; instead a module-level `scopeRenameTap` records `kind:id` and the time, and clicking the same node again within 400ms starts the in-place rename, while the first click still switches the scope normally.
- Double-clicking the project name does not trigger a re-render, so native `dblclick` is kept; `.side-heading` gains `user-select: none`, so double-clicking does not incidentally select words in the name (the input itself uses `user-select: text`).
- While an in-place rename is active, right-clicking a scope row / project name no longer opens the structure menu (the native menu is still globally disabled), avoiding interrupting the input.
- "All" and "Default" are virtual scopes with no editable name, so double-clicking does not enter rename.

### Phase 13 · Direct status selection in the task right-click menu — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T41~~ | ✅ **Right-click "Set status…" to select one of the three states directly** (done) | `main.ts`: `cycleStatus` extracts the shared `changeStatus(task, next)`; `openTaskMenu` adds a "Set status…" subpanel at the top (three states, current one checked), and a divider before delete | Right-clicking on both top-level tasks and subtasks allows directly choosing "Todo / Doing / Done"; choosing the current status writes nothing; marking as done a parent task with unfinished subtasks still goes through the confirmation dialog; "Set status…" and clicking the status cell give the same result | — |

Phase 13 implementation decisions:

- Only a frontend entry is added, with no new command: the two-step `plan_set_status` / `apply_set_status` (including cascade confirmation) is reused as-is, and rules still live only in `domain.rs`.
- The status cell's cycle entry is kept, and both call the same `changeStatus`; "Set status…" solves the trouble of cycling twice when "I only want to step back / jump one extra step".
- "Set status…" uses the menu component's `children` + `checked`, with the same hover / keyboard interaction as "Move to…"; all three states are listed, the current item is checked, and choosing the current item returns directly.
- After completing a task and the row leaving the view due to filtering, the "Task completed and hidden from the current view" toast is still reused.

### Phase 14 · Sidebar module ordering — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T42~~ | ✅ **Sidebar module drag reordering** (done) | `domain::move_module` (ownership validation + whole-project `sort_order` renumbering) → `commands.rs` / `lib.rs` / `api.ts`; `main.ts` drags module rows with Pointer Events (4px threshold, centerline insertion test, auto-scroll at the top and bottom edges, swallowing one click after the drag); `styles.css` grab cursor and insertion indicator line | Modules can be dragged before any module or to the end, and the sidebar and table grouping reorder and persist together; submodules move with the whole module and cannot be dragged separately; dragging back to the original position writes nothing; cross-project / non-existent targets are rejected by the backend; dragging does not accidentally switch the scope or collapse a module | — |

Phase 14 implementation decisions:

- Only the module level is done: in the user's mind a submodule's ownership and order are "follow the module", and dragging it separately would add a hidden dimension of state to the sidebar; `sort_order` was already queued by module within a project, so it is reused directly.
- Pointer Events are used instead of HTML5 drag-and-drop: sidebar rows are full of buttons, and HTML5's `draggable` behaves unstably on interactive controls; pointer dragging lets us control the threshold ourselves and separate the single-click / double-click / drag gestures. Pressing does not immediately enter the drag state; it starts only after moving more than 4px, so single and double clicks are unaffected.
- The insertion position is computed by each module row's centerline: the pointer on the upper half of module i → insert before it; on the lower half → insert after it (equivalent to before module i+1); the lower half of the last row → the end. Submodule rows draw no line, and dropping on one is computed by its module's position.
- When the drop point equals the current position (including "immediately after itself"), no indicator line is drawn and nothing is written; after the drag, a one-time capture-phase click listener swallows the browser's synthesized click, avoiding switching the scope or collapsing by accident.
- The command only reports "insert before whom" (`before_id`, `null` = end), and `domain.rs` uniformly rewrites the sort orders to `0..n-1`; dragging before itself is treated as no change. The frontend stores no order state; the single source of truth for order is `module.sort_order` in the database, and both `list_modules` and `buildGroups` follow it.
- Verification: the backend adds `moves_modules_within_project` / `move_module_rejects_foreign_target`; mockIPC + headless Edge dragged "move to the end / drag back to the front / small drag in place" with real pointer events, confirming the command parameter (`beforeId`), the sidebar and table group order, and that the selected scope was not touched.

### Phase 15 · Chinese/English internationalization — Done

| # | Task | Where to change | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T43~~ | ✅ **i18n infrastructure and three-state language** (done) | Add `src/i18n/index.ts` / `messages.zh.ts` / `messages.en.ts` / `static.ts` / `date.ts`; change `index.html` to a `data-i18n*` skeleton; status bar language button and three-state menu | The language menu switches; `<html lang>` is correct; static copy is bilingual and the first frame does not flash the wrong language; `tsc` / `vitest` green | — |
| ~~T44~~ | ✅ **Full migration of frontend copy** (done) | In `main.ts`, constant tables change to token → key; `Group.label` in `grouping.ts` allows `null`, and `scopeLabel` moves to the UI layer; tooltip / aria / toast / empty state / About / startup error page all go through `t()`; column widths and menu widths adapt to English | In English mode, no Chinese except user data; Chinese and English dates correct; 1280×720 and 720×480 screenshots show no header/time/status-bar truncation | T43 |
| ~~T45~~ | ✅ **Structured backend messages** (done) | Add `message.rs` and `backend-keys.json`; all `Err`, `reason` and `notices` in `domain.rs` / `commands.rs` change to `Message`; delete `TODO_NAME`, and the virtual to-do `name` returns an empty string; wire up `api.ts` / `describe` / `showNotice` | `cargo test` green; the Rust catalog matches the two frontend tables; Chinese and English error toasts and confirmation reasons work | T43 |
| ~~T46~~ | ✅ **Startup / migration error localization** (done) | `db.rs` / `migrations.rs` change to `Message`; `StartupFailure { error, backupPath }`; `lib.rs` serializes JSON through the query, and the window title is the neutral `Knot`; the frontend error page parses the JSON and localizes it | The migration failure test asserts the key; the bilingual error page; the backup path is shown separately; plain-text legacy parameters are still displayable | T45 |
| ~~T47~~ | ✅ **Installers and metadata** (done; the packaging tool is skipped per the archive convention) | `tauri.conf.json`: NSIS `["English","SimpChinese"]` + language selector, WiX `["en-US","zh-CN"]`, publisher / copyright / description; `Cargo.toml` description and authors | `tauri build --no-bundle` parses the config successfully; with an NSIS toolchain the installer shows a language selector; this machine has no WiX/NSIS, so packaging itself follows T21's skip convention | T45 |
| ~~T48~~ | ✅ **Comments, docs and gates** (done) | Source comments in English (migrations only change comments); add `README.en.md`, `docs/specs/*.en.md`, `docs/ROADMAP.en.md`, `CLAUDE.en.md`, `docs/i18n/glossary.md`; update the Chinese specs / README / CLAUDE | The comment scan finds no remaining Chinese; Chinese and English docs cross-link; all quality gates pass | T44–T47 |

Phase 15 implementation decisions:

- No i18n library is introduced: the Chinese dictionary is the single source of truth for keys, and English is forced to cover it with `satisfies Record<MessageKey, string>`; `i18n.test.ts` further verifies the key set, placeholders and the `data-i18n*` in `index.html`.
- The backend does not receive a language parameter: `Message` only carries stable keys and parameters, and the technical details of `db.*` / `startup.*` (SQLite errors, paths) are embedded as parameters unchanged, with the frontend responsible for the language of the outer sentence.
- `Message.params` supports strings, numbers and nested messages; nesting lets "reopening the database also failed: {inner error}" still localize in one pass.
- `plan_set_status.reason` changes from an empty string to `null`; the frontend decides whether to show the dialog only by `needs_confirm`, no longer relying on whether the string is empty.
- The startup error window title stays `Knot` (localStorage cannot be read when the window is created), while the body and `document.title` use the current language; no extra `setTitle` permission was added for the title.
- Switching the language clears the column-width cache and re-clamps the old widths with `COLUMN_MIN` under the new language; times are shown as `2026/11/04 09:00` (24-hour clock) in both Chinese and English, with the time column defaulting to 176px and a 80px minimum (when narrowed, dates are ellipsized and the full value remains in the hover tooltip); the task column minimum is 100px.
- Migration files only translate comments; SQL and released structures are not changed by a single character; the rule in CLAUDE.md changes to "SQL structure and logic cannot be changed; comments can be translated".
- When comments are Englished, Chinese data in test fixtures (project/module/task names) is kept, since it verifies Unicode and real input; test descriptions and assertion messages change to English.

### Stage 16 · Module duplicate-name validation — Completed

| # | Task | Files | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T49~~ | ✅ **Detect duplicate names when creating / renaming modules** (done) | `create_module` / `rename_module` in `domain.rs` + `message.rs` / `backend-keys.json` / `src/i18n/backend.ts` | A trimmed duplicate within the same project is rejected with a localized message; the same name in another project is allowed; renaming a module to its own current name still works | — |

Stage 16 implementation decisions:

- The rule lives only in `domain.rs`: `ensure_module_name_free` compares trimmed names within one project, and `except_id` allows a rename to its own current name.
- Create and rename share the same rule so a rename cannot bypass it; project and submodule names are not restricted yet.
- No database unique index: existing databases may already contain duplicates, and adding a constraint would need a migration plus cleanup; validation stays in the domain layer for now.
- Errors use the structured message `validation.moduleNameDuplicate { name }`; the frontend shows a warning dialog with only a confirm button instead of a toast, while other errors still use toasts.

### Stage 17 · Toast duration adjustment — Completed

| # | Task | Files | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T50~~ | ✅ **All toasts last 3 seconds** (done) | `showNotice` timer in `main.ts`, 2000 → 3000ms | Both info and error toasts disappear after 3 seconds; FIFO queueing is unchanged | — |

Stage 17 implementation decisions:

- Only the dwell time changes; info and error still share the same rhythm rather than being differentiated by kind.
- The "2 seconds" entries recorded under Stage 9 remain historical decisions; current behavior follows this stage.

### Stage 18 · Parent completion confirmation — Completed

| # | Task | Files | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T51~~ | ✅ **Ask before completing a parent when all subtasks are done** (done) | `aggregate_parent_status` / outcome structs in `domain.rs` + `ParentStatusProposal` in `api.ts` + `confirmParentProposal` in `main.ts` | The parent is not written when all subtasks are done; it becomes `done` only after the dialog is confirmed; cancelling keeps the current state; doing / todo still update automatically with a notice | — |

Stage 18 implementation decisions:

- `aggregate_parent_status` returns a `ParentStatusProposal` for `done` without writing; `doing` / `todo` are still written automatically and return a notice.
- The frontend's `confirmParentProposal()` asks with `askConfirm`; after confirmation it reuses `apply_set_status(parentTaskId, status, false)`. Cancelling writes nothing, and the next subtask event evaluates the parent again.
- Covered for subtask status changes, subtask deletion and reparenting; a newly created subtask is always `todo` and cannot complete a parent.
- The dialog body uses the structured message `confirm.parentAutoDone { title }`, translated through the existing tables.

### Stage 19 · Subtask retargeting — Completed

| # | Task | Files | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T52~~ | ✅ **Move a subtask directly under another parent in the same container** (done) | `reparent_task` and the `parent_proposals` array in `domain.rs`; the subtask context menu and `parentTargetItem` in `main.ts` | A subtask's context menu offers "Move to…"; the current parent is excluded; both the old and new parent are re-aggregated; two completion proposals are asked one after another | — |

Stage 19 implementation decisions:

- `Some(target)` in `reparent_task` no longer requires the task itself to be top-level: a top-level task is still demoted, while an existing subtask is retargeted to another target. The target must still be in the same container, have no parent and not be the task itself; the moved task cannot have subtasks.
- Retargeting re-aggregates both the old and the new parent, so `parentProposals` changes from a single value to an array; targeting the current parent is a no-op.
- The frontend reuses the same candidate-parent tree and excludes the current parent and every completed task; targets may be parents that already have subtasks or leaf tasks. The toast says "Moved … under …", and priority is still clamped to the new parent's level.
- New backend tests: `subtask_can_move_to_another_parent`, `retargeting_a_subtask_can_propose_both_parents`, and the `ParentStatusProposal` wire-shape test.

### Stage 20 · Quick-add module / priority — Completed

| # | Task | Files | Acceptance | Depends on |
| --- | --- | --- | --- | --- |
| ~~T53~~ | ✅ **Split into two dropdowns, module and priority, to the right of the quick-add input** (done) | `#task-form` in `index.html` (`#task-module` / `#task-priority`) + `composer` / `openComposerModuleMenu` / `openComposerPriorityMenu` / `resetComposer` in `main.ts`; `priority` added to `create_task` in `api.ts`, `commands.rs` and `domain.rs`; new copy in `messages.zh.ts` / `messages.en.ts` | The module follows the current scope by default ("All" → Default, a module / submodule → that node), and priority defaults to Medium; any module / submodule and all three priority levels can be chosen; both return to their defaults after an add and when the scope or project changes; when the task lands outside the current view, a toast says so | — |

Stage 20 implementation decisions:

- `composer` is session-only frontend state: it is not written to the database or `localStorage`. It resets after a successful add, in `applyScope`, `selectProject`, database reset, and view fallbacks caused by deletion.
- Module and priority are two independent dropdowns, each reusing a flat `openMenu` panel: submodules appear as `Module › Submodule`, avoiding nested side panels that would overflow at the bottom of the window; in "My Tasks" the module panel only offers "Default".
- `create_task` takes `priority` explicitly from the caller; when creating a subtask, `domain.rs` still clamps it to the parent's level, sharing the same ceiling rule as `set_priority`.
- A selection redraws only the trigger and the input placeholder, not the table; when the task lands where the current view cannot show it (another module or hidden by filters), a 3-second toast says where it went.
- Verification: a stateful mockIPC backend plus headless Edge covered "All → Default · Mid", "change priority / module → create arguments", "switch to a module scope → default follows", and "create outside the scope → toast"; new backend tests `create_task_keeps_requested_priority` / `create_subtask_priority_is_clamped_to_parent`.

---

## Verification Conventions

Every task is wrapped up according to this routine; do not just report "it compiles":

1. `cargo test` (backend rules) + `npm test` (frontend pure functions) + `npx tsc --noEmit` (frontend types) + `vite build`.
2. **Changes involving the UI**: use `mockIPC` from `@tauri-apps/api/mocks` to mount `src/main.ts` onto the structure of `index.html`, start `npx vite --port 1420`, and then take screenshots with headless Edge for manual review.
   - When verifying a call chain, make the mock backend **stateful** (`create_task` really appends to the array, `list_tasks` really returns it), and have the script simulate clicks/input.
   - **When wrapping up, find the PID by port and then `taskkill /F /PID`**: `background_command stop` cannot kill the child process spawned by `npx`, which will hold port 1420 and prevent `tauri dev` from starting.
3. Go through the interactions in a real window with `npm run tauri dev` — screenshots can prove rendering, but not "whether the data is correct after clicking".
4. Write spec changes back into `docs/specs/goal-and-requirements.en.md` / `docs/specs/architecture.en.md`, then update the status in this file.
