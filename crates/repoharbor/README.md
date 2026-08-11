# repoharbor

The native GPUI desktop app — the `repoharbor` binary. All logic lives in
[`repoharbor-core`](../repoharbor-core); this crate is purely the UI (theme, cards,
shell, views) plus the thin async/foreground plumbing. Desktop integration
(tray, notifications, appearance, watcher) lives in
[`repoharbor-platform`](../repoharbor-platform).

See the workspace **[README](../../README.md)** and **[CHANGELOG](../../CHANGELOG.md)**
for DigitsCode fork features (TREE, filters, context menus, top tabs, chrome).

## Run

```bash
cargo run -p repoharbor          # debug build + launch (GPUI cached → ~2s)
cargo run -p repoharbor --release
```

GPUI renders on the GPU (blade/Vulkan) and talks Wayland/X11 directly, so the
build needs the Vulkan/Wayland/xkbcommon/fontconfig headers — `bash
scripts/setup.sh` installs them. Requires the toolchain pinned in
`rust-toolchain.toml` (rustup auto-selects it).

The grid paints from the SQLite cache (`~/.local/share/repoharbor/cache.sqlite`) and
then refreshes live; on first run, use header **+ → Add local path…** (or
**Settings → Workspace roots**) and point it at your project directories.

## Layout

```
src/
  main.rs              app/window setup, key bindings, close-to-tray
  shell.rs             header + top tabs + context sidebar + RepoHarborApp state
  card.rs              the RepoCard (+ context menu)
  drawer.rs            repo detail drawer (Overview/Changes/PR/Notes/Readme)
  fleet.rs             multi-select fleet ops (Fetch/Pull/StageAll/Push/…)
  palette.rs           command palette (Ctrl+K): actions + repos + code/semantic search
  views/               inbox feed explore cleanup agents devtools settings newproject
  theme.rs             the design system as --orr-* tokens → gpui colors
  data.rs              repoharbor_core::model → flat render-ready Row (parent_id, child_count)
  assets.rs            lucide/brand/devicon + gpui-component-assets fallback
  task.rs live.rs      async (tokio) bridge + background→foreground signal wiring
  assets/              embedded fonts + generated icon SVGs (rust-embed)
```

## Hygiene

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

These are the CI gates. Lint policy lives in `[lints]` in each crate's
`Cargo.toml`.
