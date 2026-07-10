# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A native Linux calendar/task app: one shared core library, a background reminder daemon, a TUI (ratatui), and a GUI (iced). Tasks/subtasks/reminders/events are local-first in SQLite, with optional two-way sync to Google Calendar (events) and Google Tasks (tasks/subtasks).

## Commands

```bash
cargo build --workspace          # build everything
cargo build -p <core|daemon|tui|gui>   # build one crate (package name for core is `caldav_core`)
cargo test --workspace           # run all tests (core has the main test suite; gui has a few pure date-math tests; daemon/tui have none)
cargo test -p caldav_core <name> # run a single test by name/substring
cargo clippy --all-targets       # lint everything; keep this clean, it currently has zero warnings
cargo run -p tui                 # run the TUI directly
cargo run -p gui                 # run the GUI directly
cargo run -p daemon -- --install # write + enable the systemd --user unit (see daemon/src/main.rs)
cargo run -p daemon -- --auth    # run the interactive Google OAuth flow
```

There is no separate lint/format tool beyond `cargo clippy` and `cargo fmt`.

## Architecture

Cargo workspace, 4 members. `core` (package name `caldav_core`) is the only crate with real logic and the only one with tests; `daemon`, `tui`, and `gui` are thin frontends over it.

- **`core/src/db.rs`** — `Db` wraps a single `rusqlite::Connection` (WAL mode, `busy_timeout=5000`, `foreign_keys=ON`). All frontends and the daemon open the *same* SQLite file (`$XDG_DATA_HOME/caldav/caldav.db`) directly — there is no IPC layer; concurrency is handled entirely by SQLite's WAL mode + busy_timeout, not application code.
- **`core/src/models.rs`** — `Task` (self-referential `parent_id` for one level of subtasks), `Event`, `Reminder`. `Task`/`Event` carry `external_id`/`etag` columns reserved for sync.
- **`core/src/schema.sql`** — applied via `CREATE TABLE IF NOT EXISTS` on every `Db::open()`; no migration framework, since the schema is expected to stay small and additive.
- **`core/src/auth.rs`** — Google OAuth2: PKCE + a local loopback HTTP server catches the browser redirect (no framework, hand-rolled `TcpListener`). Client credentials come from a user-supplied `$XDG_CONFIG_HOME/caldav/google_client.json` (the file Google Cloud Console downloads directly for a "Desktop app" OAuth client); tokens cache in `tokens.json` (mode 0600) in the same directory.
- **`core/src/sync.rs`** — two-way sync for Google Calendar (events) and Google Tasks (tasks/subtasks). Conflict policy is last-write-wins by comparing `updated_at` timestamps (see `decide()`, which is pure and unit-tested). Deletions propagate via a `sync_tombstones` table: deleting a synced row records its `external_id` there, and the next sync issues a real remote DELETE before doing anything else (so it isn't resurrected by that same sync's pull step). Google Tasks nesting maps onto our `parent_id` via Tasks API's `parent` field, so subtasks are always pushed after their parent has an `external_id`.
- **`daemon/src/main.rs`** — synchronous poll loop (no async runtime; nothing here needs one). Every `CALDAV_POLL_SECS` (default 30s) it fires due reminders via `notify-rust`; every `CALDAV_SYNC_SECS` (default 300s) it calls `core::sync::run` if a Google account is connected. `--install` self-writes a systemd `--user` unit rather than hand-rolling daemonization.
- **`tui/src/main.rs`** and **`gui/src/main.rs`** — each independently implements the same small set of things: task list w/ subtask indent, an Events list/composer, a Calendar preview (Day/Week/Month), and date-string parsing (`parse_remind_at`/`parse_datetime`). These are **deliberately duplicated** rather than factored into `core` — see the `ponytail:` comments at each duplication site. Move something into `core` only once a third consumer needs it.
- **`gui/src/theme.rs`** — the whole GUI visual system lives here: two custom `iced::Theme`s (`adwaita_light`/`adwaita_dark`, accent `#3584e4`) tracking the OS light/dark preference via `iced::system::theme_changes()`, plus spacing/radius/type scales, an `accent_gradient()`, theme-adaptive surface helpers (`surface_color`, `is_dark`), elevation (`card`, shadows), the sidebar chrome (`sidebar`, `brand`, `nav_item`), styled inputs with a focus ring (`input`), a real accent `checkbox_style`, `segment` tabs, and `dot`/`today_cell` calendar bits. **Reuse these tokens/closures for any new GUI widget** — never hardcode a color, radius, or shadow. iced's `svg` feature is enabled (see `gui/Cargo.toml`) specifically so icons/illustrations can render.
- **`gui/src/icons.rs`** — embeds `gui/assets/icons/*.svg` (monochrome line icons, tinted at runtime) and `gui/assets/illustrations/*.svg` (baked-color duotone empty-state art that reads on both themes via `fill-opacity`). Use `icons::icon(bytes, size, color)` for a symbolic icon and `icons::illustration(bytes, width)` for empty states; add new SVGs to the assets dirs and a `const` in the `icon_bytes!` block rather than inventing per-widget drawing.
- **`gui/src/calendar.rs`** — the Day/Week/Month calendar view. As a descendant module of the binary crate root it freely calls private items in `main.rs` (`event_row`, `card_list`, `event_composer`, `empty_state`, etc.) — normal Rust privacy, not a hack; keep reusing those helpers rather than duplicating row-rendering logic. The GUI navigation is a **left icon sidebar** (`sidebar()` in `main.rs`), not top tabs; list rows are stacked elevated cards (`card_list`), and empty states are illustrated (`empty_state` takes an illustration).

### Frontend UI conventions (both TUI and GUI)

- **No color emoji, ever.** Color-emoji renders as a blank fallback dot in this environment's font stack. The GUI uses **monochrome symbolic SVG icons** (`gui/assets/icons/*.svg`, embedded via `gui/src/icons.rs`, tinted at runtime through iced's `svg::Style.color`) — that's the right way to add an icon, not an emoji glyph. The **TUI stays text-glyph-only** (ratatui can't render SVG): reminders are literal text ("(reminder)"), the tab strip and headers carry the color. Don't reintroduce emoji in either frontend.
- **Accent color is GNOME Adwaita blue `#3584e4`**, used consistently across GUI (`theme::adwaita_light`/`adwaita_dark`) and TUI (`const ACCENT` in `tui/src/main.rs`). Keep TUI's `ACCENT`/`WARNING`/`DANGER` consts and GUI's palette in sync if either changes.
- GUI is the primary/richer frontend; TUI gets the same features but a lighter visual/interaction treatment (e.g. TUI's Calendar Week/Day views are hand-rolled read-mostly lists, not full grids).
- **Display brand name is "Dayboard"** (GUI sidebar + window title, TUI header). This is *only* the display string — the workspace crates, the `caldav` package/DB path (`$XDG_DATA_HOME/caldav`), and the `caldavd` CLI keep the `caldav` name. Don't rename those.
- **TUI Nerd Font mode** is opt-in: `Glyphs::new(app.nerd)` swaps ASCII markers (`[x]`, `(reminder)`) for Nerd Font glyphs (checkbox, bell, clock, calendar). It's off by default because the glyphs only render on a terminal using a Nerd Font; users toggle it with `f` or default it on via `CALDAV_TUI_NERD=1`. The GUI has no equivalent — it uses real SVG icons.
- Both frontends carry a small credit to `subhajitpaul.com` (GUI: clickable link in the sidebar footer via `xdg-open`; TUI: dim text in the header).

### Known simplifications (intentional, not bugs)

Marked in-code with `ponytail:` comments. Notable ones:
- Sync conflict resolution is last-write-wins; no merge, no CRDT.
- Sync doesn't re-parent an already-synced Google Task subtask (Google Tasks needs a separate `move` API call for that).
- TUI's Month view is a **hand-rolled 7×6 grid** (`draw_month`/`draw_month_cell` in `tui/src/main.rs`), not ratatui's built-in `Monthly` widget — the built-in was too cramped to read at small terminal font sizes. It shows events/reminders as colored dots (accent = event, warning = reminder) and boxes today + the cursor day.
- No core-level date-range query for the calendar views — both frontends load the full event/reminder list and filter client-side.

## Testing

Real unit tests live only in `core/src/db.rs` (`#[cfg(test)] mod tests` at the bottom of the file) and `core/src/sync.rs` (tests the pure `decide()` conflict-resolution function) and `gui/src/calendar.rs` (pure date-math: `week_start`, `month_grid`, `shift`). When adding non-trivial logic to `core`, add a test in the same style (in-memory SQLite via `Db::open(":memory:")`, no fixtures/mocking framework).

For anything with a visible UI surface, this project has been verified by actually running the binary against a seeded scratch database (`XDG_DATA_HOME` pointed at a temp dir) and screenshotting it (`spectacle -b -n -o <path>` in this sandbox) — there is no automated UI test suite for `tui`/`gui`.
