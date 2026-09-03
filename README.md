# zellij-compact-bar

A [Zellij](https://zellij.dev/) `compact-bar` plugin fork with a configurable
tab label format. Based on the official plugin from
[zellij-org/zellij](https://github.com/zellij-org/zellij) (tag `v0.45.1`).

## What's different

The upstream compact-bar shows tabs as ` name ` ribbons with no numbering. This
fork adds a `tab_format` config option letting you template how each tab label
is rendered.

## Installation

Add the plugin to your `~/.config/zellij/config.kdl`, replacing the stock
`compact-bar`:

```kdl
plugins {
    compact-bar location="https://github.com/masenius/zellij-compact-bar/releases/latest/download/compact-bar.wasm" {
        tab_format "{index}. {name}"
    }
}
```

On first launch Zellij asks the plugin for permission to read application state
— focus the plugin pane (in pane mode) and grant it with `y`. The permission is
remembered afterwards.

## Configuration

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tab_format` | string | `"{index}. {name}"` | Template for each tab label |

### Placeholders

| Placeholder | Description |
|-------------|-------------|
| `{index}` | 1-based tab position (matches `GoToTab N`) |
| `{name}` | Tab name |
| `{process}` | Current foreground process in the tab's active pane — in a shell this is the current folder's basename (e.g. `myproject`); otherwise the process name (e.g. `nvim`). Falls back to the tab name until reported |
| `{cwd}` | Basename of the current working directory of the tab's active pane (e.g. `myproject`; falls back to the tab name until reported) |
| `{flags}` | Status suffixes: ` (FULLSCREEN)`, ` (SYNC)`, ` [!]` (empty if none) |

Any other text in the template is kept as-is.

Examples:

```kdl
// "1. term" style (default)
tab_format "{index}. {name}"

// tmux-like, no number
tab_format "{name}"

// number only
tab_format "{index}"

// smart: folder name in shells, process name in apps
tab_format "{process}"

// always show the current folder
tab_format "{cwd}"

// process name with a number prefix
tab_format "{index}. {process}"

// show fullscreen/sync/bell flags
tab_format "{index}. {name}{flags}"
```

## Limitations

- The `tooltip` config option is not supported for URL/file-based plugins
  (a Zellij limitation), so keybind hint tooltips won't appear.
- Because tab numbers are derived from tab order, they renumber automatically
  whenever tabs are opened, closed, or moved.

## Building

```sh
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
# target/wasm32-wasip1/release/compact-bar.wasm
```

Releases are built and published automatically by GitHub Actions on `v*` tags.

## Updating from upstream

Run the **"Sync upstream compact-bar"** workflow (Actions → workflow_dispatch).
It pulls `default-plugins/compact-bar` from `zellij-org/zellij` main, re-applies
the `tab_format` patch from `patches/tab-format.patch`, verifies the build, and
opens a PR.

## License

MIT. Based on the compact-bar plugin from [zellij-org/zellij](https://github.com/zellij-org/zellij), Copyright (c) 2020 Zellij contributors.