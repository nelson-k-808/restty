# restty

`restty` turns familiar command output into compact, color-coded tables that
adapt to the terminal's current width. It is a fast, one-shot formatter: no
daemon, no shell replacement, and no runtime dependency. An explicit `live`
mode keeps selected output open when resize-aware viewing is useful.

The safety contract is the important part. Redirected output, disabled
commands, binary or uncertain input, and oversized output are emitted with the
original bytes unchanged. The generated shell wrappers only run the formatter
when stdout is an interactive terminal.

## Status

The Model A wrapper architecture is implemented for bash and zsh. Built-in
parsers cover GNU/BSD-shaped `ls -la`, `ps aux`, `df -h`, `du -sh`, `git status
--short`, `git log --oneline`, and `docker ps` output. Unknown commands can use
the conservative aligned-table, key/value, and structured-log fallback parsers.

Transparent PTY interception (Model B) and live `--follow` rendering are not
part of v1. See [Architecture](docs/architecture.md).

## Install

From source:

```sh
cargo install --path .
```

From a release (the publishing repository can be overridden):

```sh
curl -fsSL https://raw.githubusercontent.com/restty-cli/restty/main/install.sh | sh
```

The installer verifies the release archive against `checksums.txt` before
installing to `${RESTTY_INSTALL_DIR:-$HOME/.local/bin}`.

Enable wrappers with exactly one shell startup line:

```sh
# ~/.bashrc
eval "$(restty init bash)"

# ~/.zshrc
eval "$(restty init zsh)"
```

For oh-my-zsh, copy `plugins/oh-my-zsh/restty` into the custom plugins
directory and add `restty` to the plugin list. The oh-my-bash equivalent lives
under `plugins/oh-my-bash/restty`.

## Use

Once initialized, supported non-interactive shapes are wrapped automatically:
long-format `ls`, `ps aux`, `df`, `du`, short `git status`, one-line `git log`,
`docker ps`, `lsblk`, `free`, and `ip route`. Other invocations call the
original executable directly.
Explicit use is also available:

```sh
restty run -- lsblk
restty run -- ip route
restty run -- systemctl list-units
some-command --list | restty render --source=generic --force
git status --short | restty render --source=git --force
```

`restty run -- <command> ...` is the universal opt-in path for non-interactive
Linux commands. It automatically tries a dedicated parser and then the generic
fallbacks. Editors, terminal UIs, pagers, SSH, and known `--follow` commands
are executed directly because capturing them would change or break their
behavior. There is intentionally no unsafe claim that every executable can be
transparently intercepted.

### Live reflow viewer

Use `live` when output should remain responsive after the command has
completed:

```sh
restty live -- ps aux
restty live -- lsblk
restty live -- free -h
```

The command runs once. Its bounded output is parsed once and stays in an
alternate-screen viewer that redraws when the terminal dimensions change. Use
the arrow keys or `j`/`k` to scroll, Page Up/Down for pages, Home/End to jump,
and `q`, Esc, or Ctrl-C to return to the shell. The original command exit code
is returned after the viewer closes.

Command output is parsed once when the viewer opens, then retained while the
terminal is resized. Resize updates recalculate only the layout, are coalesced
to avoid redraw storms, capped near 60 frames per second during continuous
dragging, and emitted as one synchronized update. Only changed terminal rows are
rewritten, avoiding the full-screen clear flash that makes bordered tables
appear to jitter. Scrolling at the same width reuses the rendered line cache.

To make the existing safe automatic wrappers open the viewer:

```sh
export RESTTY_LIVE=1
eval "$(restty init bash)" # or zsh
```

Unset it to restore one-shot output:

```sh
unset RESTTY_LIVE
```

Live mode intentionally bypasses editors, pagers, SSH, terminal UIs, and
follow-mode commands. When stdin or stdout is redirected, it degrades to direct
execution instead of emitting terminal-control sequences.

`--force` only opts redirected stdout into formatting. `NO_COLOR` and
`RESTTY_DISABLE=1` remain absolute passthrough switches. `--width` is useful
for snapshots and `--color=auto|always|never` controls restty's own styling.

At widths below 60 columns, only the most important value is shown. From
60–99, core columns remain. At 100–160, all parsed columns are shown with
moderate padding; wider terminals receive more padding. Every invocation reads
the live TTY width via `ioctl`.

Tables use rounded Unicode borders and styled headers by default. Set
`border_style` to `sharp`, `ascii`, or `none` when a different terminal style
is preferable. Borders, padding, and truncated cells are all included in the
width calculation, so a rendered row never exceeds the detected terminal.

## Configuration

Copy [config.example.toml](config.example.toml) to
`~/.config/restty/config.toml`. The supported schema is:

```toml
enabled_commands = ["ls", "ps", "df", "du", "git", "docker", "lsblk", "free", "ip"]
disabled_parsers = []
max_lines = 10000
max_bytes = 8388608
min_confidence = 0.72
theme = "default" # or "none"
border_style = "rounded" # rounded, sharp, ascii, or none

[breakpoints]
compact = 60
core = 100
wide = 160

[columns.ls]
name = 0
owner = 2
```

`RESTTY_ENABLED_CMDS="ls,ps,df,git"` overrides the configured command list for
quick per-shell changes. Column priority zero is most important. Built-in keys:

- `ls`: `mode`, `links`, `owner`, `group`, `size`, `modified`, `name`
- `df`: `filesystem`, `size`, `used`, `available`, `use`, `mounted`
- `du`: `size`, `path`
- `git status`: `status`, `path`; `git log`: `commit`, `subject`
- `ps` and `docker ps`: normalized lowercase header names
- `lsblk`: lowercase versions of its displayed headers
- `free`: `type`, `total`, `used`, `free`, `shared`, `buff/cache`, `available`
- `ip route`: `destination`, `gateway`, `device`, `protocol`, `scope`, `source`, `metric`, `details`

Invalid or unknown config entries are ignored silently so a configuration
mistake cannot interfere with command output.

## Adding a parser

Implement the small `Parser` trait in `src/parser/`:

```rust,ignore
pub trait Parser {
    fn name(&self) -> &'static str;
    fn detect(&self, source: &str, first_lines: &str) -> bool;
    fn parse(&self, input: &str) -> Option<View>;
    fn confidence(&self) -> f32 { 0.96 }
}
```

Detection must be structural and conservative. Parsing returns `None` if any
required row is malformed. Add the parser ahead of the generic fallbacks in
the registry, then add a real fixture and 48/80/120-column snapshots to
`tests/golden/all.txt`.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

The test suite includes parser snapshots at three responsive widths,
Unicode-width checks, deterministic arbitrary-input/property coverage, shell
generation checks, and exact-byte passthrough assertions.

## License

MIT
