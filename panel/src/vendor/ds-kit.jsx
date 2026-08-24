// ds-kit.jsx — primitives. No raw numbers: every dimension, colour, radius,
// duration and weight resolves to a token. Size props take step names; the
// numeric forms are a migration shim that snaps to the nearest step.

const { useState, useEffect, useRef, useMemo, useCallback } = React;

const hueColor = (h, l = 'var(--face-l)', c = 'var(--face-c)') => `oklch(${l} ${c} ${h})`;
const byId = (l) => Object.fromEntries(l.map(x => [x.id, x]));

const ST = {
  live:    { label:'Live',    c:'var(--ac)',  wash:'var(--ac-wash)' },
  waiting: { label:'Waiting', c:'var(--wn)',  wash:'var(--wn-wash)' },
  failed:  { label:'Failed',  c:'var(--dg)',  wash:'var(--dg-wash)' },
  done:    { label:'Done',    c:'var(--fg4)', wash:'transparent' },
};

/* Usage thresholds are a system decision, not a per-call one. */
const usageTone = (p) => p >= 85 ? 'var(--dg)' : p >= 70 ? 'var(--wn)' : 'var(--ac)';
const usageToneSoft = (p) => p >= 85 ? 'var(--dg)' : p >= 70 ? 'var(--wn)' : 'var(--ac-dim)';

/* ── breakpoint hook: layout reads its container, not the viewport ── */
function useBP(ref) {
  const [bp, setBp] = useState('sm');
  useEffect(() => {
    const el = ref?.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(([e]) => {
      const w = e.contentRect.width;
      setBp(w >= 900 ? 'lg' : w >= 480 ? 'md' : 'sm');
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return bp;
}

/* ── pressable: one press behaviour for the whole system ─────── */
function usePress(active = true) {
  const [p, setP] = useState(false);
  const h = active ? {
    onPointerDown: () => setP(true),
    onPointerUp: () => setP(false),
    onPointerCancel: () => setP(false),
    onPointerLeave: () => setP(false),
  } : {};
  return [p, h];
}

/* ── Surface: the only card in the system ───────────────────── */
function Surface({ children, tone = 1, glow, style = {}, onClick, pad = 'var(--s5)' }) {
  const [p, ph] = usePress(!!onClick);
  const bg = tone === 2 ? 'var(--sf2)' : tone === 0 ? 'transparent' : 'var(--sf)';
  return (
    <div onClick={onClick} {...ph}
      style={{ position:'relative', background:bg, borderRadius:'var(--r-lg)', padding:pad,
        boxShadow:'inset 0 var(--bw-hair) 0 var(--edge), var(--sh1)', overflow:'hidden',
        transform:p ? 'scale(0.985)' : 'none', transition:'transform var(--d2) var(--e)',
        cursor:onClick ? 'pointer' : 'default', ...style }}>
      {glow && (
        <div aria-hidden style={{ position:'absolute', inset:'calc(var(--s9) * -1) auto auto calc(var(--s7) * -1)',
          width:'calc(var(--s10) * 3)', height:'calc(var(--s10) * 2)',
          background:'radial-gradient(closest-side, var(--ac-glow), transparent)',
          animation:'glow var(--d4) ease-in-out infinite', animationDuration:'4.5s',
          pointerEvents:'none' }} />
      )}
      <div style={{ position:'relative' }}>{children}</div>
    </div>
  );
}

/* ── Face: tinted from the account hue, never a photo ───────── */
const FACE_STEP = { '3xs':'var(--face-3xs)', '2xs':'var(--face-2xs)', xs:'var(--face-xs)',
  sm:'var(--face-sm)', md:'var(--face-md)', lg:'var(--face-lg)', xl:'var(--face-xl)' };
const FACE_SNAP = [[14,'3xs'],[18,'2xs'],[20,'xs'],[22,'sm'],[24,'md'],[34,'lg'],[42,'xl']];
const faceDim = (s) => typeof s === 'string' && FACE_STEP[s] ? FACE_STEP[s]
  : typeof s === 'number' ? FACE_STEP[(FACE_SNAP.find(([n]) => n >= s) || FACE_SNAP[FACE_SNAP.length - 1])[1]]
  : FACE_STEP.lg;

function Face({ name, hue, size, ring }) {
  const d = faceDim(size);
  const small = size === '3xs' || size === '2xs' || size === 'xs' || (typeof size === 'number' && size <= 22);
  return (
    <span style={{ width:d, height:d, borderRadius:small ? 'var(--r-xs)' : 'var(--r-md)',
      background:`linear-gradient(150deg, ${hueColor(hue)}, ${hueColor(hue, 'var(--face-l2)', 'var(--face-c2)')})`,
      color:'var(--face-ink)', display:'grid', placeItems:'center', flexShrink:0,
      fontSize:`calc(${d} * var(--face-ratio))`, fontWeight:'var(--w-semi)',
      letterSpacing:'var(--ls-tight)', lineHeight:'var(--lh-flat)',
      boxShadow:ring
        ? `0 0 0 var(--bw-ring) var(--bg), 0 0 0 var(--bw-halo) ${hueColor(hue)}`
        : 'inset 0 var(--bw-hair) 0 var(--face-gloss)' }}>
      {(name?.[0] || '?').toUpperCase()}
    </span>
  );
}

/* ── Meter: 260° arc with a mono figure inside ──────────────── */
const METER = { sm:{ d:'var(--meter-sm)', sw:'var(--meter-sw-md)', f:'var(--t-head)' },
  md:{ d:'var(--meter-md)', sw:'var(--meter-sw-md)', f:'var(--t-head)' },
  lg:{ d:'var(--meter-lg)', sw:'var(--meter-sw-lg)', f:'var(--t-title)' } };

function Meter({ pct, size = 'md', sub, label }) {
  const M = METER[size] || METER.md;
  const box = useRef(null);
  const [geo, setGeo] = useState(null);
  // The arc needs pixel geometry for stroke-dasharray; read it back from the
  // token-sized box rather than hardcoding a diameter.
  useEffect(() => {
    const el = box.current; if (!el) return;
    const read = () => {
      const cs = getComputedStyle(el);
      const d = el.clientWidth;
      const sw = parseFloat(cs.getPropertyValue('--sw-px')) || 6;
      const sweep = parseFloat(cs.getPropertyValue('--meter-sweep')) || 0.72;
      setGeo({ d, sw, sweep, r:(d - sw) / 2 });
    };
    read();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(read); ro.observe(el); return () => ro.disconnect();
  }, [size]);

  const C = geo ? 2 * Math.PI * geo.r : 0;
  return (
    <div ref={box} style={{ '--sw-px':M.sw, position:'relative', width:M.d, height:M.d, flexShrink:0 }}>
      {geo && (
        <svg width={geo.d} height={geo.d} style={{ transform:'rotate(129deg)', display:'block' }}>
          <circle cx={geo.d/2} cy={geo.d/2} r={geo.r} fill="none" stroke="var(--sf3)"
            strokeWidth={geo.sw} strokeLinecap="round" strokeDasharray={`${C*geo.sweep} ${C}`} />
          <circle cx={geo.d/2} cy={geo.d/2} r={geo.r} fill="none" stroke={usageTone(pct)}
            strokeWidth={geo.sw} strokeLinecap="round"
            strokeDasharray={`${C*geo.sweep*Math.min(pct,100)/100} ${C}`}
            style={{ transition:'stroke-dasharray var(--d3) var(--e-out)' }} />
        </svg>
      )}
      <div style={{ position:'absolute', inset:0, display:'grid', placeItems:'center' }}>
        <span className="mono" style={{ fontSize:M.f, fontWeight:'var(--w-med)', lineHeight:'var(--lh-flat)' }}>
          {label ?? Math.round(pct)}
          <span style={{ fontSize:'0.62em', color:'var(--fg3)' }}>%</span>
        </span>
        {sub && <span style={{ fontSize:'var(--t-micro)', color:'var(--fg3)', lineHeight:'var(--lh-flat)' }}>{sub}</span>}
      </div>
    </div>
  );
}

function Track({ pct }) {
  return (
    <div style={{ flex:1, height:'var(--track-h)', borderRadius:'var(--r-pill)',
      background:'var(--sf3)', overflow:'hidden' }}>
      <div style={{ width:`${Math.min(pct,100)}%`, height:'100%', background:usageToneSoft(pct),
        borderRadius:'var(--r-pill)', transition:'width var(--d3) var(--e-out)' }} />
    </div>
  );
}

/* ── Dot ────────────────────────────────────────────────────── */
const DOT = { sm:'var(--dot-sm)', md:'var(--dot-md)', lg:'var(--dot-lg)' };
const dotDim = (s) => typeof s === 'string' && DOT[s] ? DOT[s]
  : typeof s === 'number' ? (s <= 6 ? DOT.sm : s <= 7 ? DOT.md : DOT.lg) : DOT.md;

function Dot({ c, pulse, size }) {
  const d = dotDim(size);
  return (
    <span style={{ position:'relative', width:d, height:d, flexShrink:0, display:'inline-block' }}>
      {pulse && <span style={{ position:'absolute', inset:'calc(var(--bw-ring) * -1)', borderRadius:'var(--r-pill)',
        background:c, animation:'breathe 2.2s ease-in-out infinite' }} />}
      <span style={{ position:'absolute', inset:0, borderRadius:'var(--r-pill)', background:c }} />
    </span>
  );
}

/* ── Chip ───────────────────────────────────────────────────── */
const CHIP_TONE = {
  quiet:  { bg:'var(--sf2)',      fg:'var(--fg2)' },
  accent: { bg:'var(--ac-wash)',  fg:'var(--ac-ink)' },
  ok:     { bg:'var(--ok-wash)',  fg:'var(--ok)' },
  warn:   { bg:'var(--wn-wash)',  fg:'var(--wn)' },
  danger: { bg:'var(--dg-wash)',  fg:'var(--dg)' },
  bare:   { bg:'transparent',     fg:'var(--fg3)' },
};

function Chip({ children, tone = 'quiet', ico, onClick, size = 'sm', style = {} }) {
  const T = CHIP_TONE[tone] || CHIP_TONE.quiet;
  const El = onClick ? 'button' : 'span';
  const h = size === 'xs' ? 'var(--ctl-xs)' : size === 'md' ? 'var(--ctl-md)' : 'var(--ctl-sm)';
  return (
    <El onClick={onClick} style={{ display:'inline-flex', alignItems:'center', gap:'var(--s1)',
      height:h, padding:'0 var(--s3)', borderRadius:'var(--r-pill)', background:T.bg, color:T.fg,
      fontSize:size === 'xs' ? 'var(--t-micro)' : 'var(--t-meta)', fontWeight:'var(--w-med)',
      whiteSpace:'nowrap', flexShrink:0, ...style }}>
      {ico && <Ico n={ico} s="2xs" w="bold" />}{children}
    </El>
  );
}

/* ── Button ─────────────────────────────────────────────────── */
const BTN_KIND = {
  primary:{ bg:'var(--ac)',      down:'var(--ac-ink)', fg:'var(--on-ac)', sh:'var(--sh-ac)' },
  quiet:  { bg:'var(--sf2)',     down:'var(--sf3)',    fg:'var(--fg)',    sh:'none' },
  ghost:  { bg:'transparent',    down:'var(--sf2)',    fg:'var(--fg2)',   sh:'none' },
  danger: { bg:'transparent',    down:'var(--dg-wash)',fg:'var(--dg)',    sh:'none' },
};

// `...rest` forwards aria-* and the like. Without it an icon-only
// button had no accessible name at all: the caller passed
// `aria-label="Send"`, `Btn` dropped it, and the only child was an
// aria-hidden glyph. Vendored file, but a primitive that silently
// discards accessibility props makes every call site wrong.
function Btn({ children, onClick, kind = 'quiet', ico, full, disabled, big, style = {}, ...rest }) {
  const [p, ph] = usePress(!disabled);
  const K = BTN_KIND[kind] || BTN_KIND.quiet;
  return (
    <button {...rest} onClick={disabled ? undefined : onClick} disabled={disabled} {...ph}
      style={{ display:'inline-flex', alignItems:'center', justifyContent:'center', gap:'var(--s2)',
        height:big ? 'var(--ctl-xl)' : 'var(--ctl-lg)',
        padding:big ? '0 var(--s6)' : '0 var(--s5)', width:full ? '100%' : 'auto',
        fontSize:big ? 'var(--t-body)' : 'var(--t-sub)', fontWeight:'var(--w-semi)',
        letterSpacing:'var(--ls-snug)', background:p ? K.down : K.bg, color:K.fg,
        borderRadius:'var(--r-pill)', boxShadow:K.sh, opacity:disabled ? 'var(--o-off)' : 1,
        transition:'background var(--d1) var(--e)', ...style }}>
      {ico && <Ico n={ico} s="md" w="reg" />}{children}
    </button>
  );
}

/* ── Tap: icon-only target ──────────────────────────────────── */
const TAP = { xs:'var(--tap-xs)', sm:'var(--tap-sm)', md:'var(--tap)' };
const tapDim = (s) => typeof s === 'string' && TAP[s] ? TAP[s]
  : typeof s === 'number' ? (s <= 30 ? TAP.xs : s <= 40 ? TAP.sm : TAP.md)
  : TAP.md;

function Tap({ n, onClick, s, c, label, style = {} }) {
  const [p, ph] = usePress(true);
  return (
    <button onClick={onClick} aria-label={label} {...ph}
      style={{ width:tapDim(s), height:tapDim(s), display:'grid', placeItems:'center',
        borderRadius:'var(--r-pill)', color:c || 'var(--fg2)',
        background:p ? 'var(--sf2)' : 'transparent', flexShrink:0,
        transition:'background var(--d1) var(--e)', ...style }}>
      <Ico n={n} s="lg" w="reg" />
    </button>
  );
}

/* ── Group + List ───────────────────────────────────────────── */
function Group({ title, action, children, style = {} }) {
  return (
    <section style={{ marginTop:'var(--s7)', ...style }}>
      {(title || action) && (
        <header style={{ display:'flex', alignItems:'baseline', gap:'var(--s3)',
          padding:'0 var(--s1) var(--s3)' }}>
          <h2 style={{ flex:1, fontSize:'var(--t-meta)', fontWeight:'var(--w-semi)',
            color:'var(--fg3)', letterSpacing:'var(--ls-wide)' }}>{title}</h2>
          {action}
        </header>
      )}
      {children}
    </section>
  );
}

function List({ children }) {
  return <div style={{ background:'var(--sf)', borderRadius:'var(--r-lg)', overflow:'hidden',
    boxShadow:'inset 0 var(--bw-hair) 0 var(--edge), var(--sh1)' }}>{children}</div>;
}

function Item({ children, onClick, first, style = {} }) {
  const [p, ph] = usePress(!!onClick);
  return (
    <div onClick={onClick} {...ph}
      style={{ display:'flex', alignItems:'center', gap:'var(--s4)', minHeight:'var(--row-min)',
        padding:'var(--s3) var(--s5)', background:p ? 'var(--sf2)' : 'transparent',
        boxShadow:first ? 'none' : 'inset 0 var(--bw-hair) 0 var(--hair)',
        cursor:onClick ? 'pointer' : 'default',
        transition:'background var(--d1) var(--e)', ...style }}>
      {children}
    </div>
  );
}

/* ── Switch + Seg ───────────────────────────────────────────── */
function Switch({ on, onClick }) {
  return (
    <button onClick={onClick} aria-pressed={on} role="switch"
      style={{ width:'var(--sw-w)', height:'var(--sw-h)', borderRadius:'var(--r-pill)',
        flexShrink:0, position:'relative', background:on ? 'var(--ac)' : 'var(--sf3)',
        transition:'background var(--d2) var(--e)' }}>
      <span style={{ position:'absolute', top:'var(--sw-pad)',
        left:on ? 'var(--sw-travel)' : 'var(--sw-pad)',
        width:'var(--sw-thumb)', height:'var(--sw-thumb)', borderRadius:'var(--r-pill)',
        background:'var(--face-ink)', boxShadow:'var(--sh-thumb)',
        transition:'left var(--d2) var(--e-out)' }} />
    </button>
  );
}

function Seg({ opts, v, onChange, style = {} }) {
  return (
    <div role="tablist" style={{ display:'inline-flex', padding:'var(--sw-pad)',
      borderRadius:'var(--r-pill)', background:'var(--sf2)', flexShrink:0, ...style }}>
      {opts.map(o => (
        <button key={o.v} role="tab" aria-selected={v === o.v} onClick={() => onChange(o.v)}
          style={{ padding:'0 var(--s4)', height:'var(--ctl-md)', borderRadius:'var(--r-pill)',
            fontSize:'var(--t-meta)', fontWeight:'var(--w-semi)',
            color:v === o.v ? 'var(--fg)' : 'var(--fg3)',
            background:v === o.v ? 'var(--sf3)' : 'transparent',
            transition:'background var(--d1) var(--e)' }}>{o.n}</button>
      ))}
    </div>
  );
}

/* ── Sheet ──────────────────────────────────────────────────── */
function Sheet({ open, onClose, title, sub, children, foot }) {
  useEffect(() => {
    if (!open) return;
    const k = (e) => e.key === 'Escape' && onClose?.();
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div onClick={onClose} style={{ position:'absolute', inset:0, zIndex:'var(--z-scrim)',
      background:'var(--scrim)', display:'flex', flexDirection:'column', justifyContent:'flex-end',
      backdropFilter:'blur(var(--blur-scrim))', WebkitBackdropFilter:'blur(var(--blur-scrim))',
      animation:'fade var(--d2) var(--e)' }}>
      <div onClick={(e) => e.stopPropagation()} role="dialog" aria-modal="true"
        style={{ background:'var(--bg)', borderTopLeftRadius:'var(--r-xl)',
          borderTopRightRadius:'var(--r-xl)', boxShadow:'var(--sh3)', maxHeight:'var(--sheet-max)',
          display:'flex', flexDirection:'column', animation:'rise var(--d3) var(--e-out)' }}>
        <div style={{ display:'grid', placeItems:'center', paddingTop:'var(--s3)' }}>
          <div style={{ width:'var(--grab-w)', height:'var(--grab-h)',
            borderRadius:'var(--r-pill)', background:'var(--sf3)' }} />
        </div>
        {title && (
          <div style={{ display:'flex', alignItems:'flex-start', gap:'var(--s3)',
            padding:'var(--s4) var(--gut)' }}>
            <div style={{ flex:1, minWidth:0 }}>
              <h3 className="disp" style={{ fontSize:'var(--t-title)' }}>{title}</h3>
              {sub && <p style={{ fontSize:'var(--t-meta)', color:'var(--fg3)',
                marginTop:'var(--s1)' }}>{sub}</p>}
            </div>
            <Tap n="x" onClick={onClose} s="sm" label="Close" />
          </div>
        )}
        <div className="sc" style={{ flex:1, minHeight:0,
          padding:`0 var(--gut) ${foot ? 'var(--s4)' : 'var(--s7)'}` }}>{children}</div>
        {foot && <div style={{ padding:'var(--s4) var(--gut) var(--s6)' }}>{foot}</div>}
      </div>
    </div>
  );
}

/* ── Connection banner ──────────────────────────────────────── */
function Wire({ state, queued, onRetry, host = 'macbook-pro' }) {
  if (state === 'online') return null;
  const off = state === 'offline';
  return (
    <div role="status" style={{ display:'flex', alignItems:'center', gap:'var(--s3)',
      margin:'0 var(--gut) var(--s4)', padding:'var(--s2) var(--s4)', borderRadius:'var(--r-md)',
      minHeight:'var(--ctl-md)', background:off ? 'var(--wn-wash)' : 'var(--sf2)',
      color:off ? 'var(--wn)' : 'var(--fg2)', fontSize:'var(--t-meta)', flexShrink:0,
      animation:'fade var(--d2) var(--e)' }}>
      <Ico n={off ? 'wifiOff' : 'refresh'} s="sm" w="reg"
        style={off ? undefined : { animation:'spin 1.4s linear infinite' }} />
      <span style={{ flex:1, fontWeight:'var(--w-med)' }}>
        {off ? (queued ? `Offline · ${queued} queued` : 'Offline — messages will queue')
             : `Reconnecting to ${host}…`}
      </span>
      {off && <button onClick={onRetry} style={{ fontWeight:'var(--w-semi)', color:'inherit',
        fontSize:'var(--t-meta)' }}>Retry</button>}
    </div>
  );
}

/* ── Badge: count on a tab or row ───────────────────────────── */
function Badge({ n, tone = 'accent' }) {
  if (!n) return null;
  const bg = tone === 'accent' ? 'var(--ac)' : tone === 'warn' ? 'var(--wn)' : 'var(--dg)';
  return (
    <span className="mono" style={{ minWidth:'var(--badge)', height:'var(--badge)',
      padding:'0 var(--s1)', borderRadius:'var(--r-pill)', background:bg, color:'var(--on-ac)',
      fontSize:'var(--t-nano)', fontWeight:'var(--w-bold)', display:'grid', placeItems:'center',
      boxShadow:'0 0 0 var(--badge-halo) var(--bg)' }}>{n}</span>
  );
}

Object.assign(window, {
  hueColor, byId, ST, usageTone, usageToneSoft, useBP, usePress,
  Surface, Face, Meter, Track, Dot, Chip, Btn, Tap,
  Group, List, Item, Switch, Seg, Sheet, Wire, Badge,
  faceDim, tapDim, dotDim,
});
