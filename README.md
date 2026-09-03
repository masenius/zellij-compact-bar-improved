# zellij-compact-bar-improved

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
    compact-bar location="https://github.com/masenius/zellij-compact-bar-improved/releases/latest/download/compact-bar.wasm" {
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
| `pane_format` | string | (empty) | Template used to auto-rename each tab's active pane |

### Placeholders

| Placeholder | Description |
|-------------|-------------|
| `{index}` | 1-based tab position (matches `GoToTab N`); for `pane_format`, the pane's 1-based position in the tab |
| `{name}` | Tab name; for `pane_format`, the pane's original name |
| `{process}` | The raw foreground process name of the tab's active pane (e.g. `nvim`, `bash`) |
| `{smart}` | Smart label: in a git repository, or in a shell, shows the current folder's basename (e.g. `myproject`); otherwise the process name |
| `{cwd}` | Basename of the current working directory of the tab's active pane (e.g. `myproject`) |
| `{flags}` | Status suffixes: ` (FULLSCREEN)`, ` (SYNC)`, ` [!]` (empty if none; always empty for `pane_format`) |

Any other text in the template is kept as-is.

If a tab has a manually-set name (not the default `Tab #N`) it is shown as-is, overriding `tab_format`. A pane with a manually-set name (not the default `Pane #N`) is not auto-renamed by `pane_format`.

Examples:

```kdl
// "1. term" style (default)
tab_format "{index}. {name}"

// tmux-like, no number
tab_format "{name}"

// number only
tab_format "{index}"

// smart: folder name in git repos and shells, process name elsewhere
tab_format "{index}. {smart}"

// always show the current folder
tab_format "{index}. {cwd}"

// raw process name with a number prefix
tab_format "{index}. {process}"

// show fullscreen/sync/bell flags
tab_format "{index}. {name}{flags}"

// rename each tab's active pane to its smart label
pane_format "{smart}"

// rename each tab's active pane to "folder - process"
pane_format "{cwd} - {process}"
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