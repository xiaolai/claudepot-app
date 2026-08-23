// Transcript prose, rendered.
//
// Claude writes markdown. Before this, the thread showed it verbatim —
// `**v0.9.50 is building.**` with the asterisks, and a GFM table as one
// run-on line of pipes, which on a 390px screen is unreadable rather
// than merely ugly. Measured on a real transcript, not imagined.
//
// ## What is rendered and what is not
//
// Prose only: the user's turns, Claude's turns, thinking, summaries.
// **Tool output is deliberately excluded** and stays in a `<pre>`. Tool
// output is arbitrary stdout, and markdown would actively corrupt it — a
// shell comment becomes a heading, a glob becomes emphasis, an ASCII
// table loses its alignment. The one thing a reader needs from a
// command's output is that it is what the command printed.
//
// ## Where the decisions live
//
// In `mdConfig.js`, not here, so `node --test` can reach them: the panel
// has no JSX-capable test runner, and a security posture asserted only
// in a comment is not one. `markdown.test.js` renders real output
// through the same configuration this component uses.
//
// The cost is measured: react-markdown + remark-gfm take the bundle from
// 78 KB to 127 KB gzipped, re-downloaded each load because the panel is
// served `no-store`. Roughly a tenth of a second on wifi, against a
// table that was previously unreadable.
import ReactMarkdown from 'react-markdown';

import { MD_COMPONENTS, REHYPE_PLUGINS, REMARK_PLUGINS } from './mdConfig.js';

export function Markdown({ text }) {
  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        rehypePlugins={REHYPE_PLUGINS}
        components={MD_COMPONENTS}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
