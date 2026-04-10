# User guide

This guide walks through the features of `markdown-reader` in the order
you're likely to encounter them. For a flat reference of every keybinding,
see the "Keyboard shortcuts" section of `README.md` or press `?` inside
the app.

## Starting out

Point the binary at a directory:

```bash
markdown-reader ~/projects/my-docs
```

With no argument, it browses the current directory. The first screen
shows the file tree on the left and an empty viewer on the right. If
you've used the app in this directory before, the viewer already shows
the tab(s) you had open last time, at the exact scroll position where
you left off — see "Session resume" below.

Press `?` at any point for a popup listing every keybinding. Press any
key to dismiss it.

## Navigating the file tree

The tree respects `.gitignore` and only shows markdown files (`.md`,
`.markdown`). Use `j`/`k` (or the arrow keys) to move the cursor, `l`
or `Enter` to enter a directory or open a file, and `h` to collapse a
directory. `gg` jumps to the first item, `G` to the last — pure vim.

Press `Tab` to move focus to the viewer. Press `Tab` again to come back.
If you'd rather hide the tree altogether, press `H` (capital) and it
disappears, giving the viewer the full width. `H` again restores it.
`[` and `]` resize the tree panel in 5% steps.

## Working with tabs

Tabs let you keep multiple markdown files open at once. There are two
ways to open a file:

1. **In the current tab** — highlight the file in the tree and press
   `Enter` (or `l`). This replaces the active tab's contents, matching
   the behavior before tabs existed. If no tab is open yet, this opens
   the first one.
2. **In a new tab** — highlight the file and press `t` (lowercase). A
   new tab is created and activated.

Once you have several tabs, the tab strip appears above the viewer,
showing each tab's index (1–9, hidden beyond 9) and filename. The
active tab is highlighted. If you have more tabs than fit, the strip
scrolls to keep the active tab visible and shows ` +N ` on the right
for the hidden count.

### Switching tabs

| Key | Action |
|---|---|
| `gt` | Next tab (vim convention) |
| `gT` | Previous tab |
| `1`–`9` | Jump to tab N |
| `0` | Jump to the last tab |
| `` ` `` (backtick) | Jump to the previously active tab |

`gt` and `gT` work from both the tree and the viewer — you don't need
to move focus first. The `` ` `` shortcut is useful when you're
bouncing between two tabs.

### Closing tabs

Press `x` in the viewer to close the active tab. If it was the last
tab, the viewer becomes empty and focus returns to the tree. Closing
the active tab falls back to the most recently active tab (or the
nearest neighbor if there's no previous).

### Tab picker

Press `T` (capital) in the viewer to open a centered overlay listing
every open tab. Navigate with `j`/`k` or arrows, press `Enter` to
activate, press `x` to close a tab without leaving the picker, and
`Esc` (or `T` again) to dismiss. For sessions with many tabs the
picker is usually faster than counting indices.

### Tab cap

A maximum of 32 tabs can be open at once. Attempting to open a 33rd
is silently ignored. Close an existing tab first.

### Duplicate handling

Opening a file that's already open activates its existing tab rather
than creating a duplicate. This applies to `t` in the tree, to global
search confirmation, and to session restore.

## Reading a document

Inside the viewer, scrolling is vim-like:

| Key | Action |
|---|---|
| `j` / `k` | One line down / up |
| `d` / `u` | Half page down / up |
| `PageDown` / `PageUp` | Full page down / up |
| `gg` | Top |
| `G` | Bottom |

The mouse wheel scrolls three lines per tick. Click inside the viewer
to focus it, or click a file in the tree to open it directly.

### Jumping to a line

Press `:` to open a prompt at the bottom of the viewer. Type a line
number and press `Enter` to jump. `Esc` cancels. The prompt only
accepts digits and is capped at nine characters. The target is the
display line — the same number you see in the line-number gutter when
it's enabled.

### Finding text in the document

Press `Ctrl+f` to open an in-document find bar. Type a query; matches
highlight in yellow, with the current match in orange. Press `Enter`
to confirm (keeps highlights, closes the bar), `Esc` to cancel
(clears highlights). Once confirmed, `n` and `N` cycle forward and
backward through matches.

Each tab keeps its own find state. Switching tabs with an active find
preserves every tab's highlights independently — return to a previous
tab and its matches are still there.

## Global search

Press `/` from the tree or viewer to open the global search overlay.
This is different from `Ctrl+f`: it searches across files in the
current root. Press `Tab` to toggle between file-name and content
modes. Use the arrow keys (or `Ctrl+n`/`Ctrl+p`) to cycle results.
`Enter` opens the selected result **in a new tab** — the current tab
is preserved so you don't lose context while exploring results. `Esc`
closes the search.

## Themes and settings

Press `c` from the tree or viewer to open the settings modal. Two
sections:

- **Theme** — pick from six built-in palettes: Default, Dracula,
  Solarized Dark, Nord, Gruvbox Dark, GitHub Light. Highlight a theme
  and press `Enter` to apply. Every open tab re-renders immediately
  with the new colors.
- **Markdown** — toggle the line-number gutter on or off with `Enter`.
  The gutter appears on the left of the viewer with display line
  numbers aligned with what `:` and `Ctrl+f` use.

Navigate with `j`/`k` or arrows, `Enter` to apply, `Esc` or `c` to
close the modal. All changes are persisted to disk immediately so the
next launch reopens with your choices.

## Session resume

When you quit, the app writes a small file recording:

- The list of currently open tabs (their canonical file paths),
- The scroll position of each,
- Which tab was active.

The record is keyed by the canonical path of the directory you
opened. Running `markdown-reader ~/projects/my-docs` the next day
restores the same tabs, scroll positions, and active tab. A different
directory has its own independent session.

If a file from a previous session no longer exists on disk, it's
silently skipped on restore. The app never refuses to launch because
of a stale or missing entry.

### Where the files live

- `config.toml` — the settings modal's choices:
  - Linux: `~/.config/markdown-reader/config.toml`
  - macOS: `~/Library/Application Support/markdown-reader/config.toml`
  - Windows: `%APPDATA%\markdown-reader\config.toml`
- `state.toml` — per-project session state:
  - Linux: `~/.local/state/markdown-reader/state.toml`
  - macOS: `~/Library/Application Support/markdown-reader/state.toml`
  - Windows: `%LOCALAPPDATA%\markdown-reader\state.toml`

Delete `state.toml` to start every project with a clean slate. Delete
`config.toml` to reset themes and line-number settings. Both files are
plain TOML and human-editable if you prefer.

## Live reload

A filesystem watcher runs in the background. When a file in the root
directory changes, the tree is rebuilt and any open tab showing that
file is re-rendered — preserving its scroll position — so you can
edit in another editor and see the change without leaving the reader.

## Quitting

Press `q` from any normal mode to quit. The current session is saved
to disk first, so reopening the directory later restores the state
exactly. `Ctrl+C` also works but skips the save — prefer `q` unless
the app is stuck.
