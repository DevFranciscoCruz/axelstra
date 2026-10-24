# Agent support notes (M8 / M8b: Gemini CLI)

Source text for the README "Agent support" section (main rewrites the README at the end). The code table is
`crates/core/src/agents.rs`.

**Verified against gemini-cli 0.65.0** (`npm install --prefix <temp> @google/gemini-cli`, run on Windows 11 with a
throwaway `HOME`/`USERPROFILE`, `GEMINI_API_KEY` set to a dummy and `GOOGLE_GEMINI_BASE_URL` pointing at a local stub
server, so no login happened and no prompt reached Google; the interactive TUI was driven through the package's own
`node-pty`). "Verified" below means exercised that way, or read in the package's bundled source/docs
(`bundle/docs/hooks/*.md`, `bundle/docs/cli/*.md`); the rows say which.

| | Claude Code | Codex | Gemini CLI (0.65.0) |
| --- | --- | --- | --- |
| Launch from the app / `axelstra agent`, with the first prompt | yes | yes | yes, `gemini -i <prompt>` (verified: TUI opens, prompt is submitted); unattended launches add `--skip-trust` |
| Unattended / dangerous runs | `-p` / `--dangerously-skip-permissions` | `exec` / bypass flag | `--skip-trust -p` with `--approval-mode auto_edit` / `--approval-mode yolo` (verified: both accepted, anything else rejected) |
| Short-answers rules | `--append-system-prompt` | typed into the TUI after boot | typed into the TUI after boot |
| Resume after a restart | `--continue`, follows `/clear` | `resume --last` | `--resume latest`, or `--resume <session uuid>` for a chosen conversation (verified; the hook then reports `source: "resume"`; with no earlier conversation `latest` just opens a fresh TUI) |
| Status hooks (exact waiting dot) | yes, `~/.claude/settings.json` | no (dot comes from the screen) | yes, `~/.gemini/settings.json`: `BeforeAgent` = working, `AfterAgent` = idle, `Notification` = waiting, `AfterTool` = working again (verified end to end, see below) |
| `/axelstration` skill | `~/.claude/skills` | `~/.agents/skills` | `~/.agents/skills` (verified: `gemini skills list` finds it; the model activates it with its `activate_skill` tool, it is not a slash command) |
| Search past conversations, tokens per worktree | yes | yes | yes, `~/.gemini/tmp/<project folder>/chats/*.jsonl` (verified layout, below) |
| Logo and name in the pickers | yes | yes | yes |

## Install the Gemini hooks

`axelstra hook install gemini` (or Settings, "Agentes: estado em tempo real"). It merges four entries into
`~/.gemini/settings.json` (a timestamped backup first, other keys untouched) and is idempotent; entries written by an
older release are rewritten to the current command instead of duplicated. Each entry has a `name`
(`axelstra-afteragent`, ...) so it shows in Gemini's `/hooks panel`.

What the real CLI does with them, and what Axelstra now does about it:

- **Shell.** Gemini starts a hook as `powershell -NoProfile -NonInteractive -Command <command>` on Windows (`bash -c`
  elsewhere). A command of the form `"C:\path\axelstra.exe" hook gemini AfterAgent` is a PowerShell *parse error* there
  (exit 1, Gemini shows a warning and carries on), so the hooks installed by the first M8 never moved a dot on Windows.
  The Windows entry is now `& 'C:\path\axelstra.exe' hook gemini AfterAgent` (single quotes, `'` doubled). Covered by
  `crates/cli/tests/gemini_hook.rs`, which runs the installed command under the real shell.
- **Environment.** `AXELSTRA_SESSION` reaches the hook: Gemini passes its own environment through (secret-looking
  variable names are only stripped when `security.environmentVariableRedaction.enabled` is on, default off, and
  `AXELSTRA_SESSION` would not match the patterns anyway). It adds `GEMINI_SESSION_ID`, `GEMINI_PROJECT_DIR`,
  `GEMINI_CWD`, `GEMINI_PLANS_DIR`, `CLAUDE_PROJECT_DIR`.
- **stdin / stdout.** stdin is one JSON object (`session_id`, `transcript_path`, `cwd`, `hook_event_name`, `timestamp`
  plus the event's fields; fixtures in `crates/cli/tests/fixtures/gemini-0.65.0`). stdout must be JSON: `{}` is "no
  decision", and that is what `axelstra hook gemini ...` prints, exit 0, in well under a second even with no daemon.
  Hooks run synchronously and block the agent, which is why the command bounds itself to 1.5 s.
- **Events.** `BeforeAgent` fires when a prompt is submitted, `AfterAgent` when the turn ends, `BeforeTool`/`AfterTool`
  around each tool call. `Notification` (`notification_type: "ToolPermission"`, `message`, `details` with the command)
  fires when a tool asks for confirmation; it comes *after* `BeforeTool` and nothing fires when the user answers, so
  `AfterTool` (the tool ran, the question is answered) puts the dot back to working. Verified end to end: a real
  Gemini session in a real Axelstra daemon went to "waiting" at the permission prompt and back after Allow.
- **Not fired for everything.** Safe read-only shell commands (`echo`, ...) are auto-allowed by Gemini's policy and give
  no `Notification`; only real confirmations do. In a folder answered "Don't trust" no hook ran at all in the one run
  tried (not even `SessionStart`), so the dot falls back to what the screen shows there.
- **Cost.** One process start per hook; on Windows that includes `powershell.exe -NoProfile`, a few hundred ms per
  event, and `AfterTool` fires once per tool call.

## Folder trust prompt

On the first run in a folder Gemini asks "Do you trust the files in this folder?" (trust folder / trust parent /
don't trust) and then restarts itself to apply the answer. Answers are kept in `~/.gemini/trustedFolders.json`.
Trusting a folder lets its `.gemini/` settings, hooks, MCP servers and skills run.

- A tab the user opens (new worktree, project default agent) keeps the dialog: the first Gemini session of each new
  worktree shows it before it reaches the `-i` prompt (verified: the prompt is still delivered after the restart).
  Trusting the parent folder of the worktrees once covers all of them.
- Runs nobody can answer pass `--skip-trust` (trusts the folder for that session only, writes nothing): one-shot
  `-p` runs, orchestrated workers, automations and `axelstra agent --prompt`. Without it `gemini -p` in an untrusted
  folder exits 1 with "Gemini CLI is not running in a trusted directory" (verified), and an interactive worker would sit
  at the dialog. This is the one place Axelstra decides trust for the user: the folder is a worktree they asked an agent
  to work in. `GEMINI_CLI_TRUST_WORKSPACE=true` is the environment alternative.

## Conversation logs

Verified layout (`crates/daemon/src/agentlog.rs`, fixture `crates/daemon/tests/fixtures/gemini-0.65.0`):

- `~/.gemini/tmp/<folder>/chats/session-<timestamp>-<id8>.jsonl`, one file per conversation. `<folder>` is a short slug
  of the project folder name (`proj`, `proj-1` on a clash), *not* a hash; `~/.gemini/projects.json` maps the
  lower-cased root path to it (`.project_root` inside the folder holds the path as well). Older installs used the
  SHA-256 of the path as folder name and migrate it; both are looked up.
- First line is a header `{sessionId, projectHash, startTime, lastUpdated, kind}` (`projectHash` is the SHA-256 of the
  root path as Gemini saw it; search hits keep using it to find the worktree). Then message records and
  `{"$set":{...}}`, `{"$patch":...}`, `{"$rewindTo":id}` lines (a resume appends `$set` and `$patch` lines).
- A message with tool calls is written **twice under the same id**; the reader keeps the last record of an id, or the
  token totals would count it twice. `tokens` = `{input, output, cached, thoughts, tool, total}`; `input` includes
  `cached`; `output` excludes `thoughts` (the totals add them back).
- The first user record is the injected `<session_context>` block (date, OS, directory tree). It is skipped for titles
  and search, so a search for a file name does not hit every conversation.
- `gemini --list-sessions` / `--resume N|UUID|latest` only see the conversations of the folder they run in.

## Other mechanisms (read, not changed)

- `GEMINI.md` (`~/.gemini/GEMINI.md`, project files, JIT subfolder files) is the context mechanism. Axelstra does not
  write one: the short-answers rules for Gemini stay typed into the TUI after boot.
- Custom commands live in `~/.gemini/commands/*.toml` (a way to get a real `/axelstration` slash command later);
  extensions in `~/.gemini/extensions`. Neither is touched.

## Not verified

- A real model: everything above ran against a stub server speaking the Gemini API (streamed `generateContent`
  replies with `usageMetadata`, one `functionCall`). Token fields come from what Gemini recorded for those replies, not
  from Google's billing.
- Login flows (OAuth, Vertex), `--sandbox`, `-w/--worktree`, `--acp`, plan mode, extensions, MCP.
- macOS and Linux (the shell there is `bash -c`; covered by the same test, not run on those systems).
- The Axelstra UI with a Gemini session on screen (settings panel, sidebar dot): the daemon side was run for real,
  the app was not.
- No per-session hook injection like Claude's `--settings`: Gemini hooks are user-global, so they also fire (and are
  ignored) in sessions outside Axelstra, where `AXELSTRA_SESSION` is unset.
