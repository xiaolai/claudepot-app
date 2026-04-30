# Keys

All your API keys and OAuth tokens, in one inventory.

![Keys tab](/screenshots/keys.png)

## What lives here

Anthropic API keys, OAuth tokens, and any third-party model keys you've added (OpenAI, Google, OpenRouter, etc.). One row per key with:

- A label you choose.
- A safe preview (`sk-ant-oat01-Abc…xyz`) — never the full secret.
- Created / last-used time.
- The slot or third-party route it's wired to.

## Where the secrets live

Always in the OS keychain — never in a plain file, never logged, never sent to the UI layer. The value you see is a truncated preview the renderer assembles from metadata; the secret itself stays Rust-side until you copy it.

## Self-clearing clipboard

When you **Copy** a key, Claudepot writes it to the clipboard — and starts a **30-second timer**. After the timer fires, the clipboard is wiped (only if it still holds the value Claudepot wrote, so it won't clobber something you copied later).

You get a small countdown next to the key while the timer is running.

## Adding a key

Paste the key once into the **Add** modal. Claudepot stores it in the keychain and immediately wipes the in-memory string — both the renderer's input field and every owned copy on the Rust side are zeroized.
