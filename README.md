# markdown-reader

A terminal-based markdown file browser and viewer built with Rust and [ratatui](https://github.com/ratatui/ratatui).

## Layout

```
+-------------------+----------------------------------------------+
|                   |                                              |
|  Files            |  Preview                                     |
|                   |                                              |
|  ▼ docs/          |  # Document Title                            |
|    README.md      |                                              |
|    guide.md       |  Some rendered **markdown** content with     |
|  ▶ src/           |  syntax highlighting, lists, tables, and     |
|    CHANGELOG.md   |  code blocks displayed inline.               |
|                   |                                              |
|       30%         |                      70%                     |
+-------------------+----------------------------------------------+
| Search [Files] (Tab to toggle)  / query█                        |
+-------------------+----------------------------------------------+
| TREE | readme.md (42%)    Tab:panel  /:search  q:quit           |
+-----------------------------------------------------------------+
```

The interface is split into three sections: a file tree on the left (30% width),
a markdown viewer on the right (70% width), and a status bar at the bottom. A
search bar appears between the main area and the status bar when activated.

## Features

- **File tree browser** -- navigate directories with collapsible folder nodes
- **Rendered markdown preview** -- headings, lists, code blocks, tables, links,
  blockquotes, task lists, and more, all rendered with color and style
- **Vim-style keybindings** -- `j`/`k` navigation, `g`/`G` jump to start/end,
  `d`/`u` for half-page scrolling
- **Fuzzy search** -- search by file name or file content, with result cycling
- **Live file watching** -- the tree and open file automatically reload when
  files change on disk
- **Respects .gitignore** -- uses the `ignore` crate to skip ignored files
- **Async runtime** -- built on Tokio for non-blocking I/O and file watching

## Installation

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (1.85+ recommended, edition 2024)

### One-line install

```sh
cargo install --git https://github.com/leboiko/markdown-reader
```

### Building from source

```sh
git clone https://github.com/leboiko/markdown-reader.git
cd markdown-reader
cargo build --release
```

The binary will be at `target/release/markdown-reader`.

## Usage

```sh
# Browse the current directory
markdown-reader

# Browse a specific directory
markdown-reader ~/projects/my-docs

# Show help
markdown-reader --help
```

Once inside the TUI, use `Tab` to switch between the file tree and the viewer,
`/` to open the search bar, and `q` to quit.

## Keyboard Shortcuts

### Tree Panel

| Key              | Action                            |
|------------------|-----------------------------------|
| `j` / Down       | Move cursor down                  |
| `k` / Up         | Move cursor up                    |
| `l` / Right / Enter | Open file or expand directory  |
| `h` / Left       | Collapse directory                |
| `g`              | Jump to first item                |
| `G`              | Jump to last item                 |
| `Tab`            | Switch focus to viewer            |
| `/`              | Open search                       |
| `q`              | Quit                              |

### Viewer Panel

| Key              | Action                            |
|------------------|-----------------------------------|
| `j` / Down       | Scroll down one line              |
| `k` / Up         | Scroll up one line                |
| `d` / PageDown   | Scroll down half page             |
| `u` / PageUp     | Scroll up half page               |
| `g`              | Scroll to top                     |
| `G`              | Scroll to bottom                  |
| `Tab`            | Switch focus to tree              |
| `/`              | Open search                       |
| `q`              | Quit                              |

### Search Mode

| Key              | Action                            |
|------------------|-----------------------------------|
| Any character    | Append to search query            |
| `Backspace`      | Delete last character             |
| `Tab`            | Toggle between file name / content search |
| `Down` / `Ctrl-n`| Next result                      |
| `Up` / `Ctrl-p` | Previous result                   |
| `Enter`          | Open selected result              |
| `Esc`            | Close search                      |

## Markdown Rendering

| Element          | Rendering                                        |
|------------------|--------------------------------------------------|
| H1               | Cyan, bold, underlined, with `█` prefix          |
| H2               | Blue, bold, with `▌` prefix                      |
| H3               | Magenta, bold, with `▎` prefix                   |
| H4-H6            | White, bold                                      |
| Bold             | Terminal bold                                     |
| Italic           | Terminal italic                                   |
| Strikethrough    | Terminal strikethrough                            |
| Inline code      | Green, bold, wrapped in backticks                 |
| Code block       | Box-drawn border, tinted background               |
| Blockquote       | Gray text with `│` left border                   |
| Unordered list   | Colored bullets: `•`, `◦`, `▪` by depth          |
| Ordered list     | Numbered with yellow markers                      |
| Task list        | Checkbox markers (checked/unchecked)              |
| Table            | Box-drawn grid with bold cyan header              |
| Link             | Blue, underlined                                  |
| Horizontal rule  | Full-width `─` line                               |

## Dependencies

| Crate                  | Purpose                          |
|------------------------|----------------------------------|
| ratatui                | Terminal UI framework            |
| crossterm              | Terminal backend / input events  |
| pulldown-cmark         | Markdown parsing                 |
| ignore                 | .gitignore-aware file discovery  |
| tokio                  | Async runtime                    |
| notify-debouncer-mini  | Filesystem change watching       |
| clap                   | CLI argument parsing             |
| anyhow                 | Error handling                   |

## License

MIT
