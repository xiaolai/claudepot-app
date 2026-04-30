# Projects

Every project Claude has ever touched, with all its sessions.

![Projects tab](/screenshots/projects.png)

## Browse and filter

Sort by recently used, by total session count, by token spend, by error rate. Filter by date, size, or whether a session crashed. Useful when you remember "the project I was working on last Thursday" but not which folder it was in.

## Cross-project text search

Search across **every transcript Claude has ever written**, in any project. You can find that one snippet you meant to keep, that error message you saw three weeks ago, that prompt you wrote and then lost.

Match results show the surrounding context. Click through to reopen the session.

## Session detail

Open any project, open any session inside it. Read the full transcript. Export it. Share it. Tokens, auth headers, and cookies are stripped before any export leaves the machine.

## Safe project rename

This is the big one.

Claude indexes sessions by the project folder's full path. If you `mv ~/code/old-name ~/code/new-name`, every session under that project becomes orphaned — Claude can't find them anymore. The folder is fine; the session history is gone.

**Rename a project from Claudepot instead.** It rewrites every reference Claude has — session transcripts, the project map, the history file, project memory, settings — in nine journaled phases. If anything fails midway, the operation rolls back. Resumable on crash, fully reversible.

## Repair

If you've already renamed something the wrong way, **Repair** finds orphaned session transcripts and offers to adopt them into the right project. It does the same nine-phase rewrite, in reverse.

## Cleanup, per-project

Right inside the project, you can prune old sessions, slim bulky tool output, or move things to trash — same actions as the global Cleanup view but scoped to one project.
