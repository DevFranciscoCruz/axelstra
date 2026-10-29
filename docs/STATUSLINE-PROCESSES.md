# Status line: processes per tick

Claude Code runs `axelstra statusline-tee` on every status line update of a session Axelstra started. The tee saves
Claude's JSON for the app, then shows the status line the user configured (`statusLine.command` in
`.claude/settings.local.json`, `.claude/settings.json` or `~/.claude/settings.json`). Code: `crates/cli/src/statusline.rs`.

## What Axelstra does (target: at most 1 process per tick besides the tee itself)

- **No `reg.exe`.** Git for Windows' install path is read with `RegGetValueW`, and only when `GIT_INSTALL_ROOT`, the
  `git.exe` on PATH and the usual folders found no Git Bash.
- **Bash is remembered.** The path found is kept in `<data dir>\statusline-bash.txt` and used while that file exists.
  No timer: when Git moves or is uninstalled the file is gone, the lookup runs again and the file is rewritten.
  Delete `statusline-bash.txt` to force a new lookup.
- **No double shell.** When the command is a program on PATH plus plain arguments (no quotes, `$`, pipes,
  redirections, globs, `=`, `%`...), it runs directly: `ccstatusline` is not wrapped in `bash -c`. Anything else still goes through
  Git Bash (`sh` elsewhere).
- **Same tick, same output.** If the previous successful output of the session is under 1 s old and came from the same
  command and the same input JSON, it is shown again and nothing is started
  (`<data dir>\statusline-last\<session>.txt`).
- Every spawn goes through `core::proc::hidden` (no console window).

## What the user can reduce (Axelstra does not touch these)

- **A status line command that fails.** When it exits non-zero the tee writes the reason to `statusline.log` in the
  data dir and shows a minimal line. `ccstatusline` was seen exiting with code 1 in several sessions: run the command by hand
  with a sample JSON on stdin (`echo {} | ccstatusline`) and fix or replace it. Each failed run still costs the whole chain
  (`cmd`/`sh`, `node`, `npm` shims) for nothing.
- **A status line that needs `npx`/`npm` or a shell script.** A single native binary (or a `node` script called by its full path)
  starts far fewer processes than a shim that starts `sh`, `uname` and `node`.
- **Orca hooks.** `~/.claude/settings.json` can list `~/.orca/agent-hooks/claude-hook.cmd` for `PreToolUse`, `PostToolUse` and
  similar events. Each one starts `cmd` (and often `bash`/`findstr`) on every tool call of every session. Remove the
  entries of tools you no longer use.
- **Fewer parallel sessions** multiply all of the above.

To see the effect: `statusline.log` should stop growing, and a session should start no more than the tee and one child per
status line update.
