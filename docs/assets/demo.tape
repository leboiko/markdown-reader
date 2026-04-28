# vhs script for the README demo GIF.
#
# Run: `vhs docs/assets/demo.tape`  → produces docs/assets/demo.gif
#
# Sequence (~50 seconds):
#   1. Open the project directory (file tree on left, picker mode)
#   2. Navigate the tree, open demo.md → show rendered Mermaid + math
#   3. Press `o` for outline picker, jump between sections
#   4. Press `c` for settings, cycle through 2 themes
#   5. Open a NEW tab from the tree (`t`) — README.md
#   6. Tab through to a heading, press Enter on a table → modal
#   7. Press `i` for hybrid live-preview editing — the killer feature
#   8. Press `/` for global content search
#   9. `:q` to quit

Output docs/assets/demo.gif

Set Width 1500
Set Height 850
Set FontSize 16
Set Theme "Dracula"
Set Padding 20
Set TypingSpeed 50ms

# Pre-build invisibly so the recording starts hot.
Hide
Type "cargo build --release -q"
Enter
Sleep 60s
Type "clear"
Enter
Sleep 200ms
Show

# Step 1 — launch the TUI against the project ROOT (file tree visible).
Type "./target/release/markdown-reader ."
Enter
Sleep 2s

# Let viewers see the file tree + welcome state
Sleep 1500ms

# Step 2 — navigate to docs/, expand it, open demo.md
# Tree starts focused.
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 300ms
Type "l"
Sleep 600ms
# Inside docs/, find the demo file (assets/ is one of the entries)
Type "j"
Sleep 200ms
Type "l"
Sleep 600ms
Type "j"
Sleep 200ms
Enter
Sleep 2s

# Switch focus to viewer
Tab
Sleep 800ms

# Scroll past the heading + paragraph to see the rendered Mermaid
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "j"
Sleep 1500ms

# Step 3 — outline picker
Type "o"
Sleep 1500ms
Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "j"
Sleep 500ms
Enter
Sleep 1500ms

# Step 4 — settings: cycle through themes
Type "c"
Sleep 1200ms
Type "j"
Sleep 300ms
Enter
Sleep 1200ms
Type "j"
Sleep 300ms
Enter
Sleep 1200ms
Escape
Sleep 800ms

# Back to tree
Tab
Sleep 600ms

# Step 5 — open README.md in a NEW tab (`t` in tree opens in new tab)
Type "g"
Sleep 200ms
Type "g"
Sleep 400ms
# README.md should be near the top of the tree
Type "j"
Sleep 200ms
Type "j"
Sleep 200ms
Type "t"
Sleep 1500ms

# Step 6 — switch focus to viewer, scroll to a table, open the modal
Tab
Sleep 800ms
Type "G"
Sleep 1s
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Enter
Sleep 2500ms
Escape
Sleep 800ms

# Step 7 — switch back to demo.md tab, demo hybrid mode
Type "1"
Sleep 800ms
Type "g"
Sleep 200ms
Type "g"
Sleep 600ms
Type "i"
Sleep 1500ms
Down
Sleep 300ms
Down
Sleep 300ms
Right@40ms 60
Sleep 400ms
Type " (live!)"
Sleep 1500ms
Down
Sleep 300ms
Down
Sleep 1500ms
Type ":q!"
Enter
Sleep 1500ms

# Step 8 — global content search (Tab from default FileName mode to Content)
Type "/"
Sleep 1000ms
Tab
Sleep 500ms
Type "mermaid"
Sleep 2s
Escape
Sleep 800ms

# Step 9 — quit
Type "q"
Sleep 1s
