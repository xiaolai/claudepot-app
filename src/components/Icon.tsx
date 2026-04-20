/**
 * Kebab-case icon API — a thin Lucide-backed wrapper kept for the
 * `<Icon name="foo" />` call sites that predate the paper-mono
 * `<Glyph g={NF.x} />` surface. Both render the same underlying
 * Lucide icon; new code should prefer `Glyph` + `NF.*`.
 */
import {
  AlertCircle,
  ArrowLeft,
  ArrowRight,
  Ban,
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  Clock,
  Copy,
  Folder,
  FolderOpen,
  Info,
  Link2Off,
  ListTree,
  Lock,
  LogIn,
  LogOut,
  Monitor,
  MoreVertical,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  SlidersHorizontal,
  Stethoscope,
  Terminal,
  Trash,
  Trash2,
  TriangleAlert,
  Undo2,
  Unlock,
  User,
  UserPlus,
  WifiOff,
  Wrench,
  X,
  XCircle,
  type LucideIcon,
} from "lucide-react";

const REGISTRY: Record<string, LucideIcon> = {
  "alert-circle":   AlertCircle,
  "alert-triangle": TriangleAlert,
  "arrow-left":     ArrowLeft,
  "arrow-right":    ArrowRight,
  "ban":            Ban,
  "chevron-down":   ChevronDown,
  "chevron-right":  ChevronRight,
  "check":          Check,
  "circle-dashed":  Circle,
  "clock":          Clock,
  "copy":           Copy,
  "folder":         Folder,
  "folder-open":    FolderOpen,
  "info":           Info,
  "list":           ListTree,
  "lock":           Lock,
  "log-in":         LogIn,
  "log-out":        LogOut,
  "monitor":        Monitor,
  "more-vertical":  MoreVertical,
  "pencil":         Pencil,
  "play":           Play,
  "plus":           Plus,
  "refresh":        RefreshCw,
  "rotate-ccw":     Undo2,
  "search":         Search,
  "settings":       Settings,
  "sliders":        SlidersHorizontal,
  "stethoscope":    Stethoscope,
  "terminal":       Terminal,
  "trash":          Trash,
  "trash-2":        Trash2,
  "undo":           Undo2,
  "unlink":         Link2Off,
  "unlock":         Unlock,
  "user":           User,
  "user-plus":      UserPlus,
  "wifi-off":       WifiOff,
  "wrench":         Wrench,
  "x":              X,
  "x-circle":       XCircle,
};

export type IconName = keyof typeof REGISTRY;

interface IconProps {
  name: IconName | (string & {});
  /** Rendered px size (icon is a square). Defaults to 14. */
  size?: number;
  className?: string;
  "aria-label"?: string;
  title?: string;
  strokeWidth?: number;
}

export function Icon({
  name,
  size = 14,
  className,
  "aria-label": ariaLabel,
  title,
  strokeWidth = 1.75,
}: IconProps) {
  const Lucide = REGISTRY[name];
  if (!Lucide) {
    if (import.meta.env?.DEV) {
      // eslint-disable-next-line no-console
      console.warn(`<Icon name="${name}" />: not in registry`);
    }
    return null;
  }
  const decorative = ariaLabel === undefined;
  return (
    <Lucide
      size={size}
      strokeWidth={strokeWidth}
      className={className}
      aria-label={ariaLabel}
      aria-hidden={decorative || undefined}
      role={ariaLabel ? "img" : undefined}
    >
      {title ? <title>{title}</title> : null}
    </Lucide>
  );
}
