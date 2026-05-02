# Browser extensions audit — 2026-05-02

**Status:** 18 total extensions across 3 browsers; 2 flagged for review.

## By browser

### Chrome

- **trusted**: 1Password X, uBlock Origin, Vimium, Octotree
- **flagged**: ColorZilla — version unchanged since 2023-08; abandoned-likely. Last 4-star review 2024.

### Safari

- **trusted**: 1Password Safari Extension, Wipr 2, AdGuard
- **flagged**: none

### Brave

- **trusted**: 1Password X, uBlock Origin
- **flagged**: PDF Viewer Pro — manifest requests `<all_urls>` + `tabs` + `history`. Heavy permissions for a PDF previewer; consider Brave's built-in PDF viewer instead.

### Recommended actions

- ColorZilla: remove from Chrome unless you actively use it. (Settings → Extensions → ColorZilla → Remove.)
- PDF Viewer Pro: remove from Brave; use the built-in viewer at brave://settings/content/pdfDocuments.
