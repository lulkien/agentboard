# Submitting a change to the board's diff channel

The review gate is only as good as the body the coder records. These are the rules
that survived review rounds where the body, not the code, was the defect.

## What the body must be

- **The change itself, verbatim.** A prose summary naming the files is neither a diff
  nor a stated no-change reason, so the reviewer refuses the round on the submission
  alone even when the code is fine. Before accepting a submission, compare what the
  body names against what the tree actually holds: `git status --short` (modified plus
  untracked paths) versus the files the body mentions. Three named files against
  sixteen changed paths is a refusal, not a nit.
- A round that genuinely changed nothing records a body saying so and why. Nothing
  else is admissible.

## Recording a large body

- Use the CLI's file form for anything big: `agentboard diff add T<n> --body-file <path>`
  (also `--body -` for stdin). This is the designed path for a large verbatim body —
  it is not "shelling out for board writes", which is what the MCP-first rule forbids
  for ordinary verbs. The MCP `diff_add` takes an inline string only, so a 90 KB change
  cannot be recorded through it without retyping, and a retyped record is not verbatim.
- Build the file so **each new path appears once**. `git add -N <new paths>` makes
  `git diff HEAD` already carry the new files as new-file hunks; appending
  `git diff --no-index /dev/null <file>` for each of them then repeats them, and while
  every byte is present, `git apply` fails on the duplicate creation. Either reset the
  intent-to-add or drop the append, then check the count per path.

## Proving it is verbatim

- Do not trust a digest recipe you have not run end to end, and do not accept one from
  the coder either. The rendering is not the bytes: `diff get D<n> --full` prints a
  header line **and appends one trailing newline** the stored body does not have, so
  `diff get --full | tail -n +2 | sha256sum` differs from `sha256sum file` by one byte
  and looks like corruption when it is the renderer.
- The strongest check needs neither side's scratch file: the tree is frozen while the
  review runs, so regenerate the submission from git (tracked diff plus each new file)
  and diff those bytes against the recorded body with the header stripped and the
  render newline dropped. An empty `diff -u` is the proof; a claimed digest is not.
- Reviewers may confirm a byte match but cannot reproduce a mutation that requires
  writing. Say "coder-reported, unverified" for those instead of accepting them.

## Order in a fix round

Fix the code first, then record the diff, so the artifact is the code that gets reviewed. Diffs are append-only: a mistaken body is superseded by a new one, and the report names which diff is current. Leave the manager's untracked scratch files out of the body.
