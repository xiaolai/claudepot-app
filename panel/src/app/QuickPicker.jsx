// Quick prompts, behind the `…` button.
//
// They used to be a scrolling row of chips above the composer, which
// cost a line of vertical space on every thread and showed maybe four
// of them before running off the edge. Behind a picker the list can be
// as long as the user wants and is searchable, at the price of one tap.
//
// **A tap sends.** That is the difference from the `/` picker, which
// stages: a slash command expands to thousands of words and deserves a
// look before it goes, while a quick prompt is a short phrase its owner
// wrote precisely so it could be fired without ceremony. Sending is the
// whole point of the button.
import { PickerSheet, PICKER_ROW_STYLE } from './PickerSheet.jsx';

export function QuickPicker({ prompts, onSend, onClose }) {
  const send = (p) => {
    onSend(p.text);
    onClose();
  };

  return (
    <PickerSheet
      label="Send a quick prompt"
      placeholder="Filter prompts…"
      items={prompts}
      match={(p, needle) =>
        p.name.toLowerCase().includes(needle) || p.text.toLowerCase().includes(needle)
      }
      empty="No quick prompts. Add them in Claudepot → Settings → Quick prompts."
      noMatch="Nothing matches that."
      onClose={onClose}
    >
      {(p) => (
        <button
          key={p.id}
          onClick={() => send(p)}
          style={{ ...PICKER_ROW_STYLE, display: 'block', width: '100%', textAlign: 'left' }}
        >
          <div style={{ fontSize: 'var(--t-sub)', fontWeight: 'var(--w-semi)' }}>{p.name}</div>
          {/* The text underneath, because the name is a label its author
              chose and may not say what actually gets sent. */}
          <div
            style={{
              fontSize: 'var(--t-micro)',
              color: 'var(--fg3)',
              marginTop: 'var(--s1)',
              lineHeight: 'var(--lh-body)',
            }}
          >
            {p.text}
          </div>
        </button>
      )}
    </PickerSheet>
  );
}
