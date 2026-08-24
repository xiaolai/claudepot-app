// ds-icons.jsx — one stroke family. 24px grid, round caps, uniform optical
// weight. Size and stroke come from tokens: geometry is authored here, scale
// is never hardcoded at a call site.

const IPATH = {
  chevL:  <path d="M15 5 8 12l7 7" />,
  chevR:  <path d="M9 5l7 7-7 7" />,
  chevD:  <path d="M5 9l7 7 7-7" />,
  chevU:  <path d="M19 15l-7-7-7 7" />,
  plus:   <><path d="M12 5v14" /><path d="M5 12h14" /></>,
  x:      <><path d="M6 6l12 12" /><path d="M18 6L6 18" /></>,
  check:  <path d="M4 12.5l5.2 5.2L20 7" />,
  up:     <><path d="M12 19V5" /><path d="M6 11l6-6 6 6" /></>,
  stop:   <rect x="7" y="7" width="10" height="10" rx="2.5" />,
  clock:  <><circle cx="12" cy="12" r="8" /><path d="M12 7.8V12l3 2" /></>,
  alert:  <><path d="M12 4.6 2.9 20h18.2L12 4.6Z" /><path d="M12 10v4.2" /><path d="M12 17.2h.01" /></>,
  info:   <><circle cx="12" cy="12" r="8" /><path d="M12 11v5" /><path d="M12 8h.01" /></>,
  term:   <><rect x="3" y="4.5" width="18" height="15" rx="3" /><path d="M7.5 10l2.4 2.2-2.4 2.2" /><path d="M12.5 14.6h4" /></>,
  folder: <path d="M3.5 7.5a2 2 0 0 1 2-2h3.2l2 2.4h7.8a2 2 0 0 1 2 2v8.1a2 2 0 0 1-2 2H5.5a2 2 0 0 1-2-2V7.5Z" />,
  layers: <><path d="M12 3.4 3.6 7.8 12 12.2l8.4-4.4L12 3.4Z" /><path d="m4.2 12.6 7.8 4.1 7.8-4.1" /><path d="m4.2 16.9 7.8 4.1 7.8-4.1" /></>,
  user:   <><circle cx="12" cy="8.4" r="3.7" /><path d="M4.8 20c.7-3.7 3.7-5.6 7.2-5.6s6.5 1.9 7.2 5.6" /></>,
  bolt:   <path d="M13.4 3 5.6 13.4h4.7L10 21l8-10.6h-4.8L13.4 3Z" />,
  search: <><circle cx="11" cy="11" r="6.4" /><path d="m16 16 4.2 4.2" /></>,
  bell:   <><path d="M6.6 10.4a5.4 5.4 0 0 1 10.8 0c0 3.6 1.3 5.1 1.9 5.7H4.7c.6-.6 1.9-2.1 1.9-5.7Z" /><path d="M10.2 19a2 2 0 0 0 3.6 0" /></>,
  sun:    <><circle cx="12" cy="12" r="4" /><path d="M12 3v2" /><path d="M12 19v2" /><path d="M3 12h2" /><path d="M19 12h2" /><path d="m5.9 5.9 1.4 1.4" /><path d="m16.7 16.7 1.4 1.4" /><path d="m18.1 5.9-1.4 1.4" /><path d="m7.3 16.7-1.4 1.4" /></>,
  moon:   <path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5Z" />,
  laptop: <><rect x="4.5" y="5" width="15" height="10.5" rx="2" /><path d="M2.5 19h19" /></>,
  key:    <><circle cx="8.2" cy="15.8" r="3.6" /><path d="m10.9 13.1 8-8" /><path d="m15.6 8.4 2.2 2.2" /></>,
  shield: <path d="M12 3.4 5 6v5.6c0 4 2.9 7.4 7 8.9 4.1-1.5 7-4.9 7-8.9V6l-7-2.6Z" />,
  branch: <><circle cx="7" cy="6" r="2.4" /><circle cx="7" cy="18" r="2.4" /><circle cx="17" cy="9.5" r="2.4" /><path d="M7 8.4v7.2" /><path d="M17 11.9c0 2.6-2.2 3.9-5.4 4.4" /></>,
  wifiOff:<><path d="m3 3 18 18" /><path d="M8.2 12.4a6 6 0 0 1 3.4-1.6" /><path d="M4.6 8.9a11 11 0 0 1 5-2.6" /><path d="M14.6 6.6a11 11 0 0 1 4.8 2.3" /><path d="M12 18.5h.01" /></>,
  refresh:<><path d="M20 12a8 8 0 1 1-2.6-5.9" /><path d="M20 4.5V10h-5.4" /></>,
  more:   <><circle cx="6" cy="12" r="1.3" /><circle cx="12" cy="12" r="1.3" /><circle cx="18" cy="12" r="1.3" /></>,
  arrowR: <><path d="M5 12h13" /><path d="m13 7 5 5-5 5" /></>,
  sliders:<><path d="M4 8h10" /><path d="M18 8h2" /><path d="M4 16h4" /><path d="M12 16h8" /><circle cx="16" cy="8" r="2.1" /><circle cx="10" cy="16" r="2.1" /></>,
  file:   <><path d="M13.5 3.5H7a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9l-5.5-5.5Z" /><path d="M13.4 3.6V9H19" /></>,
  link:   <><path d="M10 13.8a3.6 3.6 0 0 0 5.1 0l3-3a3.6 3.6 0 0 0-5.1-5.1l-1 1" /><path d="M14 10.2a3.6 3.6 0 0 0-5.1 0l-3 3a3.6 3.6 0 0 0 5.1 5.1l1-1" /></>,
};

const ICO_NAMES = Object.keys(IPATH);

/* Size and weight are token names, never numbers. Numeric props are
   accepted only as a migration shim and snap to the nearest step. */
const I_STEP = { '3xs':'var(--i-3xs)', '2xs':'var(--i-2xs)', xs:'var(--i-xs)', sm:'var(--i-sm)',
  md:'var(--i-md)', lg:'var(--i-lg)', xl:'var(--i-xl)', '2xl':'var(--i-2xl)' };
const I_SNAP = [[11,'3xs'],[12,'2xs'],[13,'2xs'],[14,'xs'],[15,'sm'],[16,'sm'],
  [17,'md'],[18,'md'],[19,'lg'],[20,'lg'],[21,'lg'],[22,'xl'],[26,'2xl']];
const W_STEP = { thin:'var(--stroke-thin)', reg:'var(--stroke)', bold:'var(--stroke-bold)' };

const iSize = (s) => typeof s === 'string' && I_STEP[s] ? I_STEP[s]
  : typeof s === 'number' ? I_STEP[(I_SNAP.find(([n]) => n >= s) || I_SNAP[I_SNAP.length - 1])[1]]
  : I_STEP.lg;
const iWeight = (w) => typeof w === 'string' && W_STEP[w] ? W_STEP[w]
  : typeof w === 'number' ? (w <= 1.65 ? W_STEP.thin : w >= 2 ? W_STEP.bold : W_STEP.reg)
  : W_STEP.reg;

function Ico({ n, s, c, w, style = {} }) {
  const d = iSize(s);
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke={c || 'currentColor'} aria-hidden
      strokeLinecap="round" strokeLinejoin="round"
      style={{ width:d, height:d, strokeWidth:iWeight(w), flexShrink:0, display:'block', ...style }}>
      {IPATH[n]}
    </svg>
  );
}

/* Solid weights, used only where a stroke would read too light: the
   active tab and status glyphs. */
const FPATH = {
  bolt:   <path d="M13.4 3 5.6 13.4h4.7L10 21l8-10.6h-4.8L13.4 3Z" />,
  layers: <path d="M12 3.4 3.6 7.8 12 12.2l8.4-4.4L12 3.4Zm7.8 8.4L12 15.9 4.2 11.8l-.6.3v.9l8.4 4.4 8.4-4.4v-.9l-.6-.3Z" />,
  user:   <path d="M12 4.4a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM4.6 20.4c.6-3.9 3.8-6 7.4-6s6.8 2.1 7.4 6H4.6Z" />,
  folder: <path d="M3.5 7.5a2 2 0 0 1 2-2h3.2l2 2.4h7.8a2 2 0 0 1 2 2v8.1a2 2 0 0 1-2 2H5.5a2 2 0 0 1-2-2V7.5Z" />,
};

function IcoFill({ n, s, c, style = {} }) {
  const d = iSize(s);
  return (
    <svg viewBox="0 0 24 24" fill={c || 'currentColor'} stroke="none" aria-hidden
      style={{ width:d, height:d, flexShrink:0, display:'block', ...style }}>
      {FPATH[n] || IPATH[n]}
    </svg>
  );
}

Object.assign(window, { Ico, IcoFill, IPATH, ICO_NAMES, iSize, iWeight });
