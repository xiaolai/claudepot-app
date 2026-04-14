---
description: Scaffold a new React component with test file following Claudepot conventions.
---

# Scaffold Component

Create a new React component following Claudepot's design system and conventions.

**Input:** $ARGUMENTS (component name, e.g., "AccountDetail" or "RefreshButton")

## Step 1: Validate

Parse the component name from the input. PascalCase it if not already. Reject if:
- Name conflicts with an existing file in `src/components/`
- Name is a reserved React term (Fragment, Suspense, etc.)

## Step 2: Read conventions

Read `.claude/rules/react-components.md` and `.claude/rules/accessibility.md` for current standards.

## Step 3: Generate component file

Create `src/components/{Name}.tsx`:

```tsx
import React from "react";

interface {Name}Props {
  // TODO: define props
}

export function {Name}(props: {Name}Props) {
  return (
    <div className="{kebab-name}">
      {/* TODO */}
    </div>
  );
}
```

Rules applied automatically:
- Named export (no default)
- Props interface inline above the component
- className uses kebab-case matching the component name
- No imports from `@tauri-apps/api/core` — data comes via props

## Step 4: Generate test file

Create `src/components/{Name}.test.tsx`:

```tsx
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { {Name} } from "./{Name}";

describe("{Name}", () => {
  it("renders without crashing", () => {
    render(<{Name} />);
    // TODO: assert on visible content
  });
});
```

## Step 5: Add CSS section

Append to `src/App.css`:

```css
/* ---------- {kebab-name} ---------- */
.{kebab-name} {
  /* TODO */
}
```

## Step 6: Report

Print:
- Files created
- Remind to add the component to the parent that will render it
- Remind to update `src/App.css` dark mode section if new colors added
- Run `pnpm test` to verify the scaffold compiles
