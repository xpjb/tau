# Bash tool launcher

The Bash tool passes its prefix and command as script-file contents, not as a
`-c` argument. On Unix the shell's argv is `<shellPath> /dev/fd/3`; neither the
prefix nor command text is included. This prevents `pgrep -f` and `pkill -f`
from finding a tool shell merely because its script mentions their pattern,
including shells belonging to other concurrent tool calls.

The source lives in an anonymous temporary file. Its descriptor is close-on-exec
in the daemon and is duplicated to FD 3 only in the child immediately before
exec. The shell opens the script, then the first script line closes FD 3 before
running the configured prefix and command. The file is automatically removed;
there is no persistent script pathname to clean up. Non-Unix builds instead
retain a private named temporary script until the tool call finishes.

Standard input remains `/dev/null`. Unlike feeding the script to `bash -s`,
commands such as `cat` and `read` cannot consume subsequent script lines. Explicit
redirections, pipelines and here-documents continue to work. Output capture,
working directory, credential removal, exit status, process-group cancellation
and timeouts are unchanged.

This uses script-file semantics: `$0` and Bash's `BASH_SOURCE[0]` identify the
script (`/dev/fd/3` on Unix), rather than the shell; `BASH_EXECUTION_STRING` is no
longer set by a `-c` invocation. The transport-closing line also shifts source
line numbers by one on Unix. The full requested command remains in the normal
conversation's tool-call record, not the process listing.

This is accidental-match prevention, not a signal sandbox. Explicitly targeting
the shell or daemon can still kill it, and a command that launches its own
`bash -c` introduces its own command-line text. No `pgrep`/`pkill` aliases or
ancestor-exclusion wrappers are installed.

Regression tests are in `crates/daemon/tests/unit/agent/tools.rs`. Run with:

```sh
/usr/local/bin/cargo nextest run --locked -p taud -E 'test(agent::tools::tests::)'
```
