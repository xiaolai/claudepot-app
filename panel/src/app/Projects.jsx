// Projects — read-only.
//
// Every verb this screen could offer (move, rename, clean, repair)
// rewrites path-keyed global Claude Code state and has a rollback
// journal behind it. None of that belongs behind a bearer token on a
// phone, and the endpoint policy is that remote may make the machine
// safer, never less safe.
import { useEffect, useState } from 'react';

import { OfflineError, api } from './api.js';
import { ago, bytes, tightPath } from './format.js';
import { CopyPath, Muted } from './views.jsx';

const { Group, Ico, Item, List } = window;

export function Projects() {
  const [projects, setProjects] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    const ctrl = new AbortController();
    api
      .projects(ctrl.signal)
      .then((d) => setProjects(d?.projects ?? []))
      .catch((e) => {
        if (e?.name === 'AbortError') return;
        setError(e instanceof OfflineError ? 'offline' : 'error');
      });
    return () => ctrl.abort();
  }, []);

  return (
    <div className="sc" style={{ flex: 1, minHeight: 0, padding: '0 var(--gut) var(--s8)' }}>
      <header style={{ padding: 'var(--s6) 0 var(--s2)' }}>
        <h1 className="disp" style={{ fontSize: 'var(--t-hero)' }}>
          Projects
        </h1>
      </header>

      {error === 'offline' && <Muted>Cannot reach this Mac.</Muted>}
      {error && error !== 'offline' && <Muted>Could not read the project list.</Muted>}
      {!error && projects === null && <Muted>Loading…</Muted>}
      {projects?.length === 0 && <Muted>No Claude Code projects on this Mac.</Muted>}

      {projects?.length > 0 && (
        <Group>
          <List>
            {projects.map((p, i) => (
              <Item key={p.path} first={i === 0}>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 'var(--t-sub)', fontWeight: 'var(--w-med)' }}>{p.name}</div>
                  {/* State C under rules/path-display.md: the path is
                      truncated and this panel has no detail view to copy
                      it from, so the tooltip and the copy control both
                      have to live on the row. */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--s1)' }}>
                    <span
                      className="mono selectable"
                      title={p.path}
                      style={{
                        fontSize: 'var(--t-micro)',
                        color: 'var(--fg4)',
                        marginTop: 'var(--s1)',
                        overflow: 'hidden',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {tightPath(p.path)}
                    </span>
                    <CopyPath path={p.path} label={`Copy the path to ${p.name}`} />
                  </div>
                  <div
                    className="mono"
                    style={{ fontSize: 'var(--t-micro)', color: 'var(--fg4)', marginTop: 'var(--s1)' }}
                  >
                    {p.sessions} session{p.sessions === 1 ? '' : 's'}
                    {p.size_bytes ? ` · ${bytes(p.size_bytes)}` : ''}
                    {p.last_modified ? ` · ${ago(p.last_modified)}` : ''}
                    {p.is_orphan ? ' · source path missing' : ''}
                  </div>
                </div>
                {p.is_orphan && <Ico n="alert" s="sm" w="bold" c="var(--wn)" />}
              </Item>
            ))}
          </List>
        </Group>
      )}

      <p style={{ marginTop: 'var(--s6)', fontSize: 'var(--t-micro)', color: 'var(--fg4)', lineHeight: 'var(--lh-body)' }}>
        Read-only. Moving, cleaning and repairing a project rewrites Claude Code state that lives outside
        the project directory — those stay at the machine.
      </p>
    </div>
  );
}
