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

/**
 * Paper-mono Nerd Font map — the camelCase surface used by the
 * new primitives (`Glyph`, `SidebarItem`, `Button`, etc.). Superset
 * of the kebab-case `ICONS` map above; new code should reach for
 * `NF.xxx` directly.
 */
export const NF = {
  // --- nav
  dashboard:  "\uf0e4",
  folder:     "\uf07b",
  folderOpen: "\uf07c",
  chat:       "\uf075",
  chatAlt:    "\uf4ad",
  settings:   "\uf013",
  sliders:    "\uf1de",
  user:       "\uf007",
  users:      "\uf0c0",
  key:        "\uf084",
  terminal:   "\uf120",
  desktop:    "\uf108",   // Desktop / monitor — the Claude Desktop target
  book:       "\uf02d",
  server:     "\uf233",
  tools:      "\uf7d9",
  package:    "\uf487",
  git:        "\ue702",

  // --- actions
  search:     "\uf002",
  plus:       "\uf067",
  minus:      "\uf068",
  x:          "\uf00d",
  check:      "\uf00c",
  chevronR:   "\uf054",
  chevronD:   "\uf078",
  chevronL:   "\uf053",
  chevronU:   "\uf077",
  ellipsis:   "\uf141",
  arrowR:     "\uf061",
  arrowUpR:   "\uf08e",
  copy:       "\uf0c5",
  trash:      "\uf1f8",
  edit:       "\uf044",
  refresh:    "\uf021",
  download:   "\uf019",
  upload:     "\uf093",

  // --- status
  dot:        "\uf111",
  dotCircle:  "\uf192",
  circle:     "\uf10c",
  star:       "\uf005",
  starO:      "\uf006",
  pin:        "\uf08d",
  lock:       "\uf023",
  unlock:     "\uf09c",
  eye:        "\uf06e",
  eyeSlash:   "\uf070",
  warn:       "\uf071",
  info:       "\uf05a",
  bolt:       "\uf0e7",
  ban:        "\uf05e",   // no-entry — used for "broken/dead credentials"
  clock:      "\uf017",
  calendar:   "\uf073",

  // --- files
  file:       "\uf15b",
  fileCode:   "\uf1c9",
  fileText:   "\uf15c",
  fileMd:     "\ue73e",
  fileJson:   "\ue60b",
  fileJs:     "\ue781",
  fileTs:     "\ue628",
  filePy:     "\ue73c",
  fileRs:     "\ue7a8",
  fileGo:     "\ue626",

  // --- theme — nf-weather pair renders crisper than fa4-sun_o / moon_o,
  // whose fine rays read as a gear shape at chrome icon sizes.
  sun:        "\ue30d",   // nf-weather-day_sunny
  moon:       "\ue32b",   // nf-weather-night_clear

  // --- misc
  home:       "\uf015",
  inbox:      "\uf01c",
  archive:    "\uf187",
  filter:     "\uf0b0",
  sort:       "\uf0dc",
  tag:        "\uf02b",
  tags:       "\uf02c",
  link:       "\uf0c1",
  grip:       "\uf58e",
  layers:     "\uf5fd",
  zap:        "\uf0e7",
  cpu:        "\uf85a",
  globe:      "\uf0ac",
  api:        "\uf085",
  branch:     "\ue725",
  signIn:     "\uf090",
  signOut:    "\uf08b",
  wrench:     "\uf0ad",
  shield:     "\uf132",
  userPlus:   "\uf234",
} as const;

export type NfGlyph = (typeof NF)[keyof typeof NF];

