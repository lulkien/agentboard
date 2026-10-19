# Rewriting an argv layer without changing behaviour

Use when replacing hand-rolled argument parsing (a `flag()`/`has()`/positional scan)
with clap, argh or lexopt — on agentboard or any Rust CLI whose flags scripts and
agents already depend on.

## Why hand-rolled parsing breaks at this size

- **`--flag=value` is silently dropped.** A next-argument scan never sees the value
  inside `--db=/path`, and a positional scan that skips anything starting with `--`
  throws it away. Result: the command runs against the default DB/path and reports
  success.
- **Unknown flags are silently ignored.** A typo (`--asignee`) does not assign, and its
  value lands in the positional list, so the *next* parse reads it wrongly.
- **A value-flag list is a second source of truth.** Every new flag that takes a value
  must be added to it or its value becomes a positional. It will be forgotten.

Silent misuse is the bug class. The rewrite's job is to make misuse loud.

## The parity contract (put this in the task row)

1. **Default no-argument mode.** Many tools use "no args" for their real job — agentboard
   serves MCP on stdio. clap would rather print help; `arg_required_else_help` must stay
   off and the `None` subcommand handled explicitly.
2. **Global flags before *and* after the subcommand.** `--db X add …` and `add … --db X`
   both work today. This needs `global = true`; forget it and half the existing
   invocations die.
3. **Aliases survive.** `--spec` = `--description`, and any verb alias already in use.
4. **Exit codes do not move.** Keep 0 success, and any tool-specific codes (3 for a
   timeout, for example). clap exits `2` on a usage error by default, so parse with
   `try_parse` and map: `DisplayHelp`/`DisplayVersion` → print, exit 0; anything else →
   `eprintln!("<tool>: {e}")`, exit 1.
5. **No ANSI.** Set `ColorChoice::Never` (or drop the colour feature). Agents read panes
   and tests compare text; escape bytes are noise, and `--help | od -c` is the check.
6. **Every spelling unchanged, plus the two fixes.** `--flag=value` now works, unknown
   flags now error. Nothing else about the surface moves.
7. **Validation stays where it was.** CSV/enumerated values validated in the store (with
   their aliases and error text) stay there — do not move them into `value_parser` in the
   same change. Typing a *numeric* flag is fine and improves errors, but say so: a silent
   fallback-to-default becoming an error is a behaviour change.
8. **Version** from `CARGO_PKG_VERSION`.
9. **The hyphen contract must be symmetric, not per-argument.** Setting
   `allow_hyphen_values` only on free-text *positionals* fixes `note T3 "- bullet"` while
   leaving `--description "- bullet"` rejected — the same value accepted in one slot and
   refused in the other, which is worse than the old uniform silence. Set it on **every
   free-text argument** (title, description, goal, note/report/ask/answer/block text,
   done/review summary, status note, send text) plus `allow_negative_numbers` on the
   numeric flags so `--priority -5` works like `--priority=-5`, and state the rule in one
   line of help: values beginning with `-` are literal, prefer `--flag=value` when unsure.
   The accepted cost: a mistyped flag sitting in a text slot becomes the text.
   `last(true)` is not the fix — it forces `--` for every text value and breaks the
   invocations you were preserving.
10. **Keep the entry file thin**: derive types and help text in their own module, `main`
    becomes parse → dispatch. Delete the old helpers and confirm nothing else referenced
    them.
11. **Do not leave a second copy of the help text.** `USAGE` strings and hand-written
    README command blocks drift from the generated surface within one change. Point the
    README at `agentboard --help` / `<tool> <verb> --help` and keep only what help
    cannot carry: the exit-code contract, the output markers, and the hyphen rule.

## Verification that proves parity

A green suite proves the suite, not the surface. Run, and paste:

- `--db=/tmp/x`, `--db /tmp/x`, and `<verb> … --db /tmp/x` all land in the **same** DB,
  and the real DB's row count is unchanged before/after.
- an unknown flag exits 1, names the flag, and writes nothing (check the row list).
- a bad numeric value exits 1 with an actionable message.
- `--help | od -c | rg '033'` prints no escape bytes, and help still lists every verb
  plus the exit-code note.
- the default no-arg mode is exercised by a real client (a stdio JSON-RPC handshake), not
  inferred from `--help`.
- test counts and clippy warning counts named **before and after** the change.
- **The parse surface needs its own test; a protocol suite does not cover it.** A stdio
  MCP/service test spawns the binary and never looks at stderr or exit codes, so the
  defects this change fixes stay hand-checked. Add a shell test (`tests/cli.sh`) that
  builds nothing, uses a scratch DB, traps cleanup, prints `N/N passed`, and asserts: the
  `--flag=value` form, unknown flag → exit 1 plus the suggestion tip, a bad numeric →
  exit 1, aliases, `--help`/`--version`/`<verb> --help` → 0 with no 0x1b bytes, a global
  flag placed after and between positionals, and the hyphen cases from the contract.
- **Prove the test bites.** Strip the attributes it guards (the hyphen pair, the exit
  mapping) and show the failure count drop, then restore. A 39/39 that also passes on the
  broken code is decoration.

## Dependency note

clap pulls `anstream`/`anstyle`/`colorchoice` to colour help and errors, which is wasted
when the output is machine-read. Measured on clap 4.6: `default-features = false` with
`features = ["std", "help", "usage", "error-context", "suggestions", "derive"]` keeps
the `a similar argument exists: '--assignee'` tip, keeps `--help`/`--version`, and drops
`anstream`, `anstyle-query`, `colorchoice`, `is_terminal_polyfill` and `utf8parse`
(`anstyle` itself stays, via `clap_builder`). `suggestions` must be in that list or the
tip disappears. If you take the trim you must also delete any `ColorChoice::Never`:
`ColorChoice` is gated behind the colour feature, so the reference fails to compile
(E0599) rather than silently losing colour.
