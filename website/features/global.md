# Global

Browse your user-wide Claude Code configuration in one place.

![Global tab](/screenshots/global.png)

## What you see

A tree of every artifact Claude Code reads from `~/.claude/`, grouped:

- **Agents** — every installed agent definition.
- **Skills** — every installed skill.
- **Commands** — every slash-command.
- **Plugins** — installed Claude Code plugins.
- **Files** — the rest: `settings.json`, memory (`CLAUDE.md`), managed-policy files, anything else under `~/.claude/`.

Pick any item; the right pane shows the file's contents.

## Read-only

Claudepot doesn't edit these files. The Global tab is an inspection surface — handy when something behaves oddly and you want to know **which config layer is responsible**. You can see, for any setting, exactly where its current value comes from.

## When to use it

- "Why is Claude using Opus by default?" — open Global, find the model setting, see which layer is winning.
- "Is this plugin actually loaded?" — check the plugins list.
- "What's in my global memory?" — read it without `cat`-ing files.
