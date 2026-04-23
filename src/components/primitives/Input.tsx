import {
  type ChangeEvent,
  type CSSProperties,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
  useState,
} from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "./Glyph";

interface InputProps {
  glyph?: NfIcon;
  placeholder?: string;
  value?: string;
  onChange?: (e: ChangeEvent<HTMLInputElement>) => void;
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void;
  onFocus?: (e: FocusEvent<HTMLInputElement>) => void;
  onBlur?: (e: FocusEvent<HTMLInputElement>) => void;
  suffix?: ReactNode;
  type?: string;
  autoFocus?: boolean;
  disabled?: boolean;
  style?: CSSProperties;
  /**
   * Forward a ref to the inner `<input>` element. Callers programmatic-
   * ally focus the field (e.g. ⌘F in the Config section) by assigning
   * the ref and calling `.focus()` / `.select()`.
   */
  inputRef?: Ref<HTMLInputElement>;
  "aria-label"?: string;
}

/**
 * Minimal mono input. Leading `glyph` and trailing `suffix` slots.
 * Accent-bordered on focus. Matches the `--input-height` (32px)
 * token so inputs and buttons align in toolbars.
 */
export function Input({
  glyph,
  placeholder,
  value,
  onChange,
  onKeyDown,
  onFocus,
  onBlur,
  suffix,
  type = "text",
  autoFocus,
  disabled,
  style,
  inputRef,
  ...aria
}: InputProps) {
  const [focused, setFocused] = useState(false);
  return (
    <label
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        height: "var(--input-height)",
        padding: "0 var(--sp-10)",
        background: "var(--bg-raised)",
        border: `var(--bw-hair) solid ${focused ? "var(--accent-border)" : "var(--line)"}`,
        borderRadius: "var(--r-2)",
        transition: "border-color var(--dur-fast) var(--ease-linear)",
        opacity: disabled ? "var(--opacity-dimmed)" : 1,
        ...style,
      }}
    >
      {glyph && <Glyph g={glyph} color="var(--fg-faint)" />}
      <input
        ref={inputRef}
        value={value ?? ""}
        onChange={onChange}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        type={type}
        autoFocus={autoFocus}
        disabled={disabled}
        onFocus={(e) => {
          setFocused(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocused(false);
          onBlur?.(e);
        }}
        {...aria}
        style={{
          flex: 1,
          minWidth: 0,
          border: "none",
          outline: "none",
          background: "transparent",
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
        }}
      />
      {suffix}
    </label>
  );
}
