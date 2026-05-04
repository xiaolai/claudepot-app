# What & why

## What

ClauDepot is a desktop app — with a matching command-line tool — that sits next to Claude Code and Claude Desktop and gives you the things they don't have on their own:

- **Account switching.** Keep work and personal accounts side by side; switch in one click.
- **A live view.** See every Claude session that's running right now, including which one is waiting on you.
- **Scheduled prompts.** Have Claude run a prompt every morning, every weekday, or on any cron schedule you like.
- **Searchable history.** Find that chat from last week. Reopen it. Export it.
- **Safe project rename.** `mv`-ing a project folder breaks Claude's session history. ClauDepot doesn't.
- **Disk cleanup.** `~/.claude/` quietly grows to many gigabytes. One click reclaims most of it, with a 7-day undo.
- **Privacy on export.** Tokens, auth headers, and cookies are stripped before any session leaves your machine.

It's macOS-first today. Windows and Linux work, with less polish.

## Why

If you use Claude Code or Claude Desktop daily, you've probably hit at least one of these. Each one is a first-class fix in ClauDepot, not a workaround.

| Pain | What's actually going on |
| ---- | ------------------------ |
| `/login` doesn't switch accounts when one is already signed in | Claude reads from a single keychain slot. There's no concept of "the other account." |
| You renamed a project folder and your old sessions disappeared | Claude indexes sessions by the folder's full path. Rename the folder and the index breaks. |
| `~/.claude/` is using 8 GB and you don't know why | Every chat is kept forever as a transcript file, including image data and tool output. Nothing prunes it. |
| Claude Code freezes on a long conversation | A single transcript over ~50 MB stalls the parser. |
| You can't tell which Claude session needs your attention | Claude has no notion of "I'm waiting on you" — you have to check each terminal. |
| You've leaked tokens by pasting a screenshot or exporting a chat | Tokens appear verbatim in transcripts and exports. |
| You want Claude to run a daily summary at 8am | There's no scheduler. |
| You hit a rate limit you didn't know existed | The 5-hour window, 7-day window, and Opus split are invisible until you trip them. |

## Who it's for

Anyone who uses Claude Code or Claude Desktop every day, has more than one account, runs more than one project, and wants to stop juggling all of it by hand.

## Next

- [Getting started](./getting-started) — install in five minutes.
- [First run](./first-run) — what to do the first time you open the app.
