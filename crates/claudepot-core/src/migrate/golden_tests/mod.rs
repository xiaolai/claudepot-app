//! Integration goldens for `migrate`. See spec §11 for the full
//! matrix.
//!   - `path_shapes` — §11.1 cross-OS path-rewrite goldens.
//!   - `fs_shapes`   — §11.2 filesystem-shape goldens (case-insensitive
//!                    collisions, slug clashes, long-path budget).
//!   - `cc_state`    — §11.3 CC-state goldens (uuid collision, schema
//!                    version drift, file-history repath).
//!   - `trust_gates` — §11.4 trust-gate goldens (hooks split, mcp
//!                    scrub, integrity tamper, encryption refused).

#[cfg(test)]
mod cc_state;
#[cfg(test)]
mod fs_shapes;
#[cfg(test)]
mod path_shapes;
#[cfg(test)]
mod trust_gates;
