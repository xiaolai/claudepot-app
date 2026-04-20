/**
 * Icon registry — thin wrapper over `lucide-react`.
 *
 * The old Nerd Font pipeline (codepoints rendered in JBMono NF) was
 * replaced because AppKit menus don't respect custom fonts, which
 * forced us to rasterize every glyph to PNG by hand. Lucide ships a
 * consistent SVG set that renders crisply in both the webview and
 * (via `lucide-static`) in the Tauri tray menu.
 *
 * The `NF` name is kept for backward-compatibility with existing call
 * sites — the value is now a React component reference instead of a
 * string codepoint. `<Glyph g={NF.user} />` still works as written;
 * `Glyph` renders the component instead of a text character.
 */
import {
  Archive,
  ArrowRight,
  ArrowUpRight,
  Ban,
  Calendar,
  Check,
  CheckCircle,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  Circle,
  Clock,
  Copy,
  Cpu,
  Dot,
  Download,
  Edit,
  Eye,
  EyeOff,
  File,
  FileCode,
  FileText,
  FileType,
  Filter,
  Folder,
  FolderOpen,
  GitBranch,
  Globe,
  Grip,
  Home,
  Inbox,
  Info,
  Key,
  Layers,
  Link,
  Lock,
  LogIn,
  LogOut,
  MessageSquare,
  Monitor,
  Moon,
  MoreVertical,
  Package,
  Pin,
  Play,
  Plus,
  RefreshCw,
  Search,
  Server,
  Shield,
  SlidersHorizontal,
  SortAsc,
  Star,
  Sun,
  Tag,
  Tags,
  Terminal,
  Trash2,
  TriangleAlert,
  Unlock,
  Upload,
  User,
  UserPlus,
  Users,
  Wrench,
  X,
  XCircle,
  Zap,
  type LucideIcon,
} from "lucide-react";

/**
 * Icon registry. The keys match the legacy Nerd Font map so existing
 * `<Glyph g={NF.user} />` call sites keep compiling. Add icons to this
 * map as they're needed — most Lucide names are `kebab-case` in their
 * registry and `PascalCase` in the React import.
 */
export const NF = {
  // --- nav
  dashboard:  Layers,
  folder:     Folder,
  folderOpen: FolderOpen,
  chat:       MessageSquare,
  chatAlt:    MessageSquare,
  settings:   SlidersHorizontal,
  sliders:    SlidersHorizontal,
  user:       User,
  users:      Users,
  key:        Key,
  terminal:   Terminal,
  desktop:    Monitor,
  book:       FileText,
  server:     Server,
  tools:      Wrench,
  package:    Package,
  git:        GitBranch,

  // --- actions
  search:     Search,
  plus:       Plus,
  minus:      X,          // rarely used — alias for "remove" glyph
  x:          X,
  check:      Check,
  checkCircle: CheckCircle,
  chevronR:   ChevronRight,
  chevronD:   ChevronDown,
  chevronL:   ChevronLeft,
  chevronU:   ChevronUp,
  ellipsis:   MoreVertical,
  arrowR:     ArrowRight,
  arrowUpR:   ArrowUpRight,
  copy:       Copy,
  trash:      Trash2,
  edit:       Edit,
  refresh:    RefreshCw,
  download:   Download,
  upload:     Upload,
  play:       Play,

  // --- status
  dot:        Dot,
  dotCircle:  Circle,
  circle:     Circle,
  star:       Star,
  starO:      Star,
  pin:        Pin,
  lock:       Lock,
  unlock:     Unlock,
  eye:        Eye,
  eyeSlash:   EyeOff,
  warn:       TriangleAlert,
  info:       Info,
  bolt:       Zap,
  ban:        Ban,
  clock:      Clock,
  calendar:   Calendar,
  xCircle:    XCircle,

  // --- files
  file:       File,
  fileCode:   FileCode,
  fileText:   FileText,
  fileMd:     FileType,
  fileJson:   FileCode,
  fileJs:     FileCode,
  fileTs:     FileCode,
  filePy:     FileCode,
  fileRs:     FileCode,
  fileGo:     FileCode,

  // --- theme
  sun:        Sun,
  moon:       Moon,

  // --- misc
  home:       Home,
  inbox:      Inbox,
  archive:    Archive,
  filter:     Filter,
  sort:       SortAsc,
  tag:        Tag,
  tags:       Tags,
  link:       Link,
  grip:       Grip,
  layers:     Layers,
  zap:        Zap,
  cpu:        Cpu,
  globe:      Globe,
  api:        Key,           // fallback — "api" wasn't a Lucide icon
  branch:     GitBranch,
  signIn:     LogIn,
  signOut:    LogOut,
  wrench:     Wrench,
  shield:     Shield,
  userPlus:   UserPlus,
} as const satisfies Record<string, LucideIcon>;

/** Component reference for a single icon (e.g. the return of `NF.user`). */
export type NfIcon = LucideIcon;
