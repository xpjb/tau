# Antigravity CLI backend

Antigravity is an opt-in backend on the Tau daemon. It runs `agy acp` over
stdio; the existing Tau clients do not need an update.

1. Install [Antigravity CLI](https://antigravity.google/docs/getting-started?tab=cli)
   on the daemon host. Verify that `agy acp --help` lists `--sandbox` and
   `--disable-telemetry`. The backend has been exercised with `agy` 1.5.15.
2. Sign in on that host with `/absolute/path/to/agy acp --login`. Use the
   daemon's OS user and `HOME`. To use a separate CLI profile, create a private
   directory owned by that user, pass it as `HOME` for sign-in, and set
   `TAU_ANTIGRAVITY_HOME` to the same absolute path in the daemon environment.
   Google also documents [Gemini API key sign-in](https://antigravity.google/docs/install?tab=cli).
3. Set `TAU_ANTIGRAVITY_BIN=/absolute/path/to/agy` in the daemon environment and
   start the updated daemon. For Tau Beta, the environment file is
   `/etc/tau2-beta.env`. This does not change Tau's default model.
4. Select `antigravity/default` for a new chat. It uses the CLI's default model.
   To see exact models for the signed-in account, refresh the `antigravity`
   model catalog in Tau. A successful session open also updates the catalog.

Antigravity runs its own tools in `TAU_CWD`. Tau requests the terminal sandbox,
never grants an ACP permission request, and does not offer a permission dialog.
Configure any unattended actions with scoped rules in the CLI profile. Use a
trusted profile and workspace: CLI rules and sandbox availability determine what
its tools can do under the daemon user. Do not put an untrusted executable at
`TAU_ANTIGRAVITY_BIN` or share Tau's CLI profile with another client that can
advance the same Antigravity conversations.

Tau saves the transcript and reopens the CLI conversation after a completed turn,
even across daemon restarts. Forks, interrupted turns, and missing CLI caches
start a separate conversation with a bounded text recap; opaque context and
attachments are not replayed. Stop signals the CLI and ends its subprocess;
it cannot undo effects. Tau does not automatically retry failed CLI turns.

The CLI handles its own project files and compaction. Tau custom and project
instructions are sent as prompt text. Tau-specific tools such as `send_file`
and Tau `/compact` are unavailable on this backend. Thinking levels are applied
only when the selected CLI model offers the requested level.
