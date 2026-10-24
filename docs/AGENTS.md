# Agent support notes (M8: Gemini CLI)

Source text for the README "Agent support" section (main rewrites the README at the end). The code table is
`crates/core/src/agents.rs`. Nothing about Gemini CLI could be run on the dev machine (not installed): every row marked
*unverified* comes from the official docs and fixtures, not from a real install.

| | Claude Code | Codex | Gemini CLI |
| --- | --- | --- | --- |
| Launch from the app / `axelstra agent`, with the first prompt | yes | yes | yes, `gemini -i <prompt>` (unverified) |
| Short-answers rules | `--append-system-prompt` | typed into the TUI after boot | typed into the TUI after boot |
| Resume after a restart | `--continue`, follows `/clear` | `resume --last` | `--resume latest`, or a chosen conversation from search (unverified) |
| Status hooks (exact waiting dot) | yes, `~/.claude/settings.json` | no (dot comes from the screen) | yes, `~/.gemini/settings.json`: `BeforeAgent` = working, `AfterAgent` = idle, `Notification` = waiting (unverified) |
| `/axelstration` skill | `~/.claude/skills` | `~/.agents/skills` | `~/.agents/skills` (Gemini reads it; the model activates it on its own, it is not a slash command) |
| Search past conversations, tokens per worktree | yes | yes | yes, `~/.gemini/tmp/<hash>/chats` (unverified layout) |
| Logo and name in the pickers | yes | yes | yes |

## Install the Gemini hooks

`axelstra hook install gemini` (or Settings, "Agentes: estado em tempo real"). It merges three entries into
`~/.gemini/settings.json` (a timestamped backup first, other keys untouched) and is idempotent. The hook command
prints `{}` because Gemini parses a hook's stdout as JSON.

## Not verified

- Hook environment: that Gemini passes `AXELSTRA_SESSION` to hook processes (it inherits the terminal's env per the docs).
- That an empty-object stdout and exit 0 is accepted on every event; that `Notification` fires for permission prompts.
- `gemini -i`, `--approval-mode`, `--resume` against a real CLI; the chats folder layout.
- No per-session hook injection like Claude's `--settings`: Gemini hooks are user-global, so they fire (and are
  ignored) in sessions outside Axelstra, where `AXELSTRA_SESSION` is unset.
