# Issue #40 scope record

Initial ceiling: at most 120 net production lines, two snapshot files, and
zero regressions.

The reviewed implementation stays at 119 net production lines. The full test suite
showed that quote removal is deliberately duplicated across the focused
snapshot suite and the regression corpus. Two semantic cases therefore require
four baseline files:

- multiline quoted label, natural width
- multiline quoted label, constrained width
- focused multiline quoted-label snapshot
- focused long quoted-label snapshot

All four diffs only remove structural quote marks and recalculate the narrower
box/edge geometry. No unrelated snapshot changed. Deleting duplicate fixtures
or replacing their exact snapshots with weaker substring assertions was
rejected because it would reduce regression coverage merely to satisfy a file
count. The bounded snapshot ceiling is therefore revised from two to exactly
four files; production and zero-regression ceilings remain unchanged.
