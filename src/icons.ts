/**
 * Nerd Font glyph codepoints for Claudepot's icon system.
 *
 * We use JetBrainsMono Nerd Font Mono for every glyph, so "icons" are
 * just text characters in the body font — no separate SVG library, no
 * stroke-width bookkeeping. Colors and sizes follow the surrounding
 * text by default.
 *
 * Most codepoints are Font Awesome 4 (nf-fa-*) because those are the
 * most widely used and stable across Nerd Fonts versions. A few are
 * substitutes where FA4 lacks a modern semantic.
 *
 * Cheat sheet: https://www.nerdfonts.com/cheat-sheet
 */
export const ICONS = {
  // --- Status / alerts
  "alert-circle":   "\uf06a",     // exclamation-circle
  "alert-triangle": "\uf071",     // exclamation-triangle
  "info":           "\uf05a",     // info-circle
  "check":          "\uf00c",
  "x":              "\uf00d",     // times
  "x-circle":       "\uf057",     // times-circle
  "ban":            "\uf05e",
  "shield":         "\uf132",

  // --- Arrows & chevrons
  "arrow-left":     "\uf060",
  "arrow-right":    "\uf061",
  "chevron-right":  "\uf054",
  "chevron-down":   "\uf078",
  "rotate-ccw":     "\uf0e2",     // undo (counter-clockwise arrow)
  "undo":           "\uf0e2",

  // --- Actions
  "copy":           "\uf0c5",
  "pencil":         "\uf040",
  "refresh":        "\uf021",
  "plus":           "\uf067",
  "search":         "\uf002",
  "trash":          "\uf1f8",     // solid trash
  "trash-2":        "\uf014",     // outline trash (trash-o)
  "wrench":         "\uf0ad",
  "stethoscope":    "\uf0f1",
  "play":           "\uf04b",
  "more-vertical":  "\uf142",     // ellipsis-v — per-row menu trigger

  // --- Identity / files / devices
  "user":           "\uf007",
  "user-plus":      "\uf234",
  "folder":         "\uf07b",
  "folder-open":    "\uf07c",
  "list":           "\uf03a",
  "terminal":       "\uf120",
  "monitor":        "\uf108",     // desktop

  // --- Auth / time
  "lock":           "\uf023",
  "unlock":         "\uf09c",
  "log-in":         "\uf090",     // sign-in
  "log-out":        "\uf08b",     // sign-out
  "clock":          "\uf017",     // clock-o

  // --- Settings / misc
  "settings":       "\uf013",     // cog
  "unlink":         "\uf127",     // chain-broken
  "wifi-off":       "\uf05e",     // substitute: ban (no wifi-off in FA4)
  "circle-dashed":  "\uf10c",     // circle-o (empty circle)
} as const;

export type IconName = keyof typeof ICONS;
