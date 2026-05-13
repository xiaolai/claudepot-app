//! Integration test against the real CC binary on disk. Skipped by
//! default — drift detector for the byte-pattern extractor across CC
//! version bumps.
//!
//! Run with `cargo test -p claudepot-core --test cc_tips_real --
//! --ignored`.

use claudepot_core::cc_tips::{extract::extract_from_binary, extract::resolve_cc_binary};

#[test]
#[ignore]
fn extracts_from_real_cc_binary() {
    let bin = resolve_cc_binary().expect("CC binary on PATH");
    let tips = extract_from_binary(&bin).expect("extract");
    eprintln!("extracted {} tips from {}", tips.len(), bin.display());
    for t in &tips {
        eprintln!(
            "  {}: {}",
            t.id,
            &t.prose.chars().take(60).collect::<String>()
        );
        if let Some(b) = &t.prose_b {
            eprintln!("    [variant_b] {}", b.chars().take(60).collect::<String>());
        }
    }
    assert!(tips.len() >= 30, "expected ≥30 tips, got {}", tips.len());
    // Spot-checks for known ids.
    let ids: std::collections::HashSet<_> = tips.iter().map(|t| t.id.as_str()).collect();
    for required in [
        "new-user-warmup",
        "plan-mode-for-complex-tasks",
        "color-when-multi-clauding",
        "memory-command",
        "permissions",
    ] {
        assert!(
            ids.contains(required),
            "expected id `{required}` in extraction output"
        );
    }
}
