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

## Safety invariants

Formatting is skipped when any of these is true:

- stdout is not a TTY (unless explicit `--force` was supplied);
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
