import {
  type ComponentPropsWithoutRef,
  type CSSProperties,
  type Ref,
  useId,
  useState,
} from "react";
import { fieldControl, fieldShell } from "./fieldChrome";

type NativeProps = Omit<
  ComponentPropsWithoutRef<"textarea">,
  // The wrapper draws the chrome, so its style is not the control's.
  "style" | "className" | "ref"
>;

interface TextareaProps extends NativeProps {
  /** Styles for the wrapper (width, flex), not the inner control. */
  style?: CSSProperties;
  inputRef?: Ref<HTMLTextAreaElement>;
}

/**
 * Multiline sibling of [`Input`], sharing its chrome.
 *
 * It exists because the app had exactly one textarea —
 * `QuickPromptsPane`'s prompt body — and it was bare. `tokens.css` gives
 * `input, textarea` only `font` and `color`, so it rendered with the
 * user-agent border while every other field in the app went through
 * `Input`. In the pane that mattered most the two sat six inches apart.
 *
 * Copying `Input`'s style block would have fixed the pixels and left two
 * chromes to drift, which is the failure this codebase keeps recording.
 * Both read `fieldChrome` instead, so there is one border to change.
 *
 * `resize: vertical` and a floor of three lines are the only things it
 * adds: a prompt body is written, not glanced at, and a control the user
 * cannot grow is worse than one that starts too tall.
 */
export function Textarea({ style, inputRef, rows = 2, onFocus, onBlur, id, ...rest }: TextareaProps) {
  const [focused, setFocused] = useState(false);
  const generatedId = useId();
  return (
    <div
      style={{
        ...fieldShell({ focused, disabled: rest.disabled, fixedHeight: false }),
        ...style,
      }}
    >
      <textarea
        {...rest}
        id={id ?? generatedId}
        ref={inputRef}
        rows={rows}
        // `pm-focus` for the same reason `Input` carries it: the inline
        // style cannot express `:focus-visible`, and the accent border
        // alone fires on mouse focus too, so it is chrome rather than a
        // keyboard-focus indicator.
        className="pm-focus"
        onFocus={(e) => {
          setFocused(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocused(false);
          onBlur?.(e);
        }}
        style={{
          ...fieldControl(),
          resize: "vertical",
          minHeight: "calc(var(--fs-sm) * var(--lh-body) * 3)",
          lineHeight: "var(--lh-body)",
        }}
      />
    </div>
  );
}
