# Recording a large submission in the diff channel

The diff row is the artifact a reviewer cites, so it must be the change itself
verbatim — not a summary of it. A summary is neither a diff nor a statement
that nothing changed, and a reviewer should refuse that submission on sight.

## Size

A big phase runs to thousands of lines. The CLI carries the file form for
exactly that; the inline MCP body does not scale past a screenful:

```bash
agentboard diff add T<n> --body-file full.diff
```

Build the body from git so it includes new files:

- `git add -N <new paths>` then `git diff HEAD` already carries new files as
  new-file hunks — in that case do NOT also append `git diff --no-index
  /dev/null <path>` per file: the body then contains each new file twice, and
  `git apply` of the record fails on the duplicate creation.
- Alternatively append the `--no-index` hunks and keep intent-to-add out of it.
  Either shape is fine; containing each file exactly once is the acceptance.
- Exclude the manager's untracked scratch notes. They are not part of the
  change and must not enter the record.

## Proving the record is byte-identical

Do not compare a naive digest of `diff get --full` against the file: the render
adds a header line and one trailing newline, so the digests differ by one byte
and the difference looks like a defect. Two proofs that hold:

- strip the header and the trailing render newline before hashing (`tail -n +2`
  then `head -c -1`), or
- better, regenerate the submission from the tree with git and compare those
  bytes against the recorded body (empty `diff -u` = identical). This one does
  not depend on anyone's scratch file surviving, so the reviewer can reproduce
  it.

Order of operations in a fix round: fix the code FIRST, then record the diff,
so the artifact is the code that gets reviewed and not the code before the
fixes. Diffs are append-only — a mistaken body is superseded by a new one, never
edited, and the row's report names which diff is current.
