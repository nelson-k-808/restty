# Architecture and safety model

## Phase boundary

v1 deliberately uses explicit shell wrappers (Model A). `restty init bash` and
`restty init zsh` generate functions for a configured command allow-list. A
function checks that stdout is a TTY, invokes the real executable with the
shell's `command` builtin, pipes stdout through `restty render`, and returns the
real command's pipeline status. `ls` is only captured for long-format flags,
`ps` for `aux`, `git` for short `status` and one-line `log`, and `docker` for
`ps`, so interactive, paged, or unbounded variants are never captured.

Model B—running arbitrary children under a PTY and hooking shell command
execution—is deferred. If pursued, it must preserve `isatty` behavior, forward
signals and exit codes, detect alternate-screen applications, and pass through
interactive programs such as editors, pagers, shells, SSH, tmux, and fuzzy
finders. It should be shipped behind an explicit experimental flag.

Additional explicit subcommands—`explore`, `watch`, and structured `--format`
output—extend this same explicit-invocation model (Model A plus `restty
run`) rather than requiring Model B. Each still names a specific command up
front; none of them make interception transparent.

## Data flow

```text
real command -> bounded stdin reader -> parser registry -> responsive renderer -> TTY
                     |                      |
                     +-- limit exceeded ---+
                     +-- disabled ----------+--> original bytes
                     +-- invalid UTF-8 -----+
                     +-- low confidence ----+
```

The reader buffers up to the configured line/byte limits. On crossing a limit,
it writes the buffered bytes and streams the remainder without waiting for EOF.
Otherwise, the registry asks ordered parsers to detect against only the first
few lines and parse the complete bounded input. A parser cannot partially
render: one malformed required row rejects the complete result.

Built-ins and generic fallbacks implement the same `Parser` trait. This keeps
detection, parsing, confidence, fixtures, and registration localized when a
new command is added.

For broad coverage without unsafe shell-wide interception, `restty run --
<command>` is an explicit universal adapter. It attempts dedicated,
aligned-table, key/value, and structured-log parsers. A deny-list executes
interactive/full-screen programs and known follow-mode commands with inherited
stdio instead of capturing them.

## Parser sourcing

The registry orders three sources per command, most specific first: a
dedicated built-in, a `jc`-backed adapter, then the generic fallback. A `jc`
adapter is a thin `Parser` implementation that pipes the same bounded input
into the matching `jc` parser subprocess, deserializes the returned JSON, and
maps each object's fields onto the stable column keys the renderer expects—
label, alignment, and priority are layered on top of `jc`'s existing field
names rather than re-derived by new parsing logic. `jc` is resolved once at
startup by checking `PATH`. Detection for a `jc`-backed adapter is
source-keyed rather than content-sniffed: the wrapper already knows which
real command it captured, so the matching adapter runs directly without
sniffing the first lines, while content-sniffed built-ins and the generic
fallback still detect as described above. When `jc` is absent, `jc`-backed
adapters are simply not registered and matching commands fall through to the
generic fallback, so its absence degrades coverage, not correctness.

## Structured output

Parsed rows carry stable keys independent of the renderer, so emitting them
as JSON, CSV, or YAML is a second terminal node off the parser registry
rather than new parsing work: `restty render --format=json` (and the `run`
equivalent) writes the row list through a serializer instead of the
responsive renderer. This path is explicit script usage, so it is exempt
from the TTY-only safety invariant the same way `--force` is—`--format`
always emits, TTY or not—but the line/byte bounds, UTF-8 validity, and
allow-list checks still apply, since those protect against runaway memory
and mis-detection rather than display concerns.

## Persistent live viewer

`restty live -- <command>` is distinct from transparent Model B interception.
It explicitly runs one non-interactive command, buffers output within the same
line and byte safety limits, and then enters the terminal's alternate screen.
While the viewer is active it polls the live ioctl dimensions, relays out a
retained parsed view on width changes, and provides vertical scrolling. Raw
terminal state, cursor visibility, wrapping, and the main screen are restored
by an RAII guard on every ordinary exit path.

Resize rendering uses a short settle window plus a 16 ms maximum frame cadence.
Parsing happens once before the viewer enters its event loop; a width-keyed line
cache avoids layout work while scrolling. Each frame is composed in memory,
enclosed in terminal synchronized-update mode, and written in one operation. A
retained copy of the prior screen means every update clears and rewrites only
changed rows without clearing the entire screen buffer.

Setting `RESTTY_LIVE=1` tells generated wrappers to call this explicit viewer.
It is opt-in because the shell prompt cannot return until the viewer is closed.
Non-TTY output, interactive deny-listed programs, streaming follow modes,
binary output, and output above configured bounds do not enter the viewer.
Because it restores the main screen through the same RAII guard on every
ordinary exit path, invoking it from inside a tmux popup or similar
transient pane needs no special handling beyond the invocation itself.

## Interactive explorer

`restty explore -- <command>` extends the live viewer's alternate-screen
lifecycle with an input-handling loop: the same RAII guard, resize polling,
and width-keyed layout cache apply, but keypresses additionally mutate a
session-local sort key, an incremental filter string matched against cell
text, and a selected-row index. Sorting and filtering operate on the
already-parsed row list rather than re-invoking the source command, so they
cost a re-layout, not a re-parse. The selected row is tracked independently
from the viewport so filtering and sorting retain predictable navigation.
Parser-declared external actions remain reserved for a later capability-gated
extension; no current parser exposes an action. Explore is a distinct
subcommand rather than default behavior specifically so plain wrapped
commands stay non-interactive and script-safe; the same deny-list and
bounded-input rules from `restty run` gate what it will accept.

## Change detection and snapshots

Every parsed result accepted from an explicit child-command invocation is
written to a per-command cache keyed by a hash of the source command and its
arguments, at
`~/.cache/restty/<hash>.json`, holding the row list and a timestamp. On the
next invocation of the same command, the renderer can diff the new rows
against the cached ones by matching each parser's declared key column: rows
present only in the new result are marked added, rows present only in the
cached result are marked removed, and rows present in both with differing
non-key fields are marked changed. Diff state is a per-cell style flag
layered on top of the semantic style the parser already assigns, so it
composes with responsive column selection rather than needing a separate
rendering path. `restty last` re-emits the most recently cached render for a
command without re-running it. `restty snapshot <name>` copies the current
cache entry to a named, non-overwritten file; `restty diff <name>` compares
the latest cached output against that named snapshot instead of the rolling
last-run cache. `restty diff <name> -- <command>` performs a fresh bounded
invocation instead. The cache is opportunistic: a missing or unreadable entry
is treated as no prior state, never as an error.

## Watch mode and alerts

`restty watch --interval <seconds> -- <command>` reuses the live viewer's
alternate-screen lifecycle but re-invokes the bounded command on a timer
instead of once, feeding each result through the same diff logic used for
snapshots so added, removed, and changed rows are visible between refreshes.
An optional `--alert <pattern>` matches new or changed rows and triggers a
terminal bell or OS notification on first match per session, then keeps
watching. This is deliberately scoped to commands that can be safely re-run
to completion within the bounds `restty run` already enforces (`df`, `ps`,
short `git status`, …); it is unrelated to, and does not relax, the
deny-list that keeps true unbounded follow-mode commands (`tail -f`,
`docker logs -f`) running with inherited stdio instead of being captured.

## Safety invariants

Formatting is skipped when any of these is true:

- stdout is not a TTY (unless explicit `--force` was supplied, or
  `--format` requests structured output, which is always emitted regardless
  of TTY state);
- `RESTTY_DISABLE=1` or `NO_COLOR` exists;
- the source command is outside the configured allow-list;
- line or byte limits are crossed;
- bytes are not UTF-8;
- no parser accepts all required rows;
- parser confidence is below the configured threshold.

Skipped output is byte-for-byte identical, including final newline state.
Diagnostics are not printed for detection or parsing failures. CLI usage and
I/O errors remain ordinary errors.

## Responsive rendering

Parsers assign stable keys, labels, alignment, semantic style, and numeric
priority to columns. The renderer selects columns from terminal width and
priority, then shrinks cell widths by Unicode display cells and truncates with
an ellipsis. Width is read with `TIOCGWINSZ` for each process invocation, never
cached or sourced from `$COLUMNS`. Automatic width reserves the terminal's
final cell so border glyphs never trigger last-column autowrap; explicit
`--width` values remain exact for tests and embedding.

Column metadata may declare a value type beyond plain text: a
bounded-percentage type renders an inline bar alongside its numeric value, a
timestamp type renders a relative label with the absolute form available at
wide breakpoints, and a permission-bits type renders colored glyphs with the
raw symbolic form as fallback. Threshold rules in the config file can
override a cell's semantic style based on its value—a percentage crossing 90
rendering in the destructive style regardless of the parser's default—
independent of which breakpoint dropped or kept the column. Terminal
capability is detected from the locale and `TERM`, with an explicit config
override. Unicode box drawing degrades to ASCII rather than emitting glyphs
the target terminal cannot display. The semantic palette uses portable ANSI
colors, so it requires no true-color fallback conversion.
