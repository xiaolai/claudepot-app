use crate::account::AccountStore;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ResolveError {
    #[error("no account matching '{0}'")]
    NoMatch(String),

    #[error("'{input}' is ambiguous: {}", candidates.join(", "))]
    Ambiguous {
        input: String,
        candidates: Vec<String>,
    },

    #[error("store error: {0}")]
    StoreError(String),
}

/// Resolve a user-provided string to a registered email.
///
/// Resolution: find all registered emails where input is a prefix.
/// Exactly one match → return it. Zero → error. Multiple → error.
pub fn resolve_email(store: &AccountStore, input: &str) -> Result<String, ResolveError> {
    let accounts = store
        .list()
        .map_err(|e| ResolveError::StoreError(e.to_string()))?;

    let input_lower = input.to_lowercase();
    let mut matches: Vec<String> = accounts
        .iter()
        .filter(|a| a.email.to_lowercase().starts_with(&input_lower))
        .map(|a| a.email.clone())
        .collect();

    match matches.len() {
        0 => Err(ResolveError::NoMatch(input.to_string())),
        1 => Ok(matches.remove(0)),
        _ => Err(ResolveError::Ambiguous {
            input: input.to_string(),
            candidates: matches,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Account, AccountStore};
    use chrono::Utc;
    use uuid::Uuid;

    fn test_store() -> (AccountStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = AccountStore::open(&db_path).unwrap();

        for email in [
            "lixiaolai@gmail.com",
            "xiaolaidev@gmail.com",
            "xiaolaiapple@gmail.com",
        ] {
            store
                .insert(&Account {
                    uuid: Uuid::new_v4(),
                    email: email.to_string(),
                    org_uuid: None,
                    org_name: None,
                    subscription_type: None,
                    rate_limit_tier: None,
                    created_at: Utc::now(),
                    last_cli_switch: None,
                    last_desktop_switch: None,
                    has_cli_credentials: true,
                    has_desktop_profile: false,
                    is_cli_active: false,
                    is_desktop_active: false,
                    verified_email: None,
                    verified_at: None,
                    verify_status: "never".to_string(),
                })
                .unwrap();
        }
        (store, dir)
    }

    #[test]
    fn test_resolve_exact_email() {
        let (store, _dir) = test_store();
        assert_eq!(
            resolve_email(&store, "lixiaolai@gmail.com").unwrap(),
            "lixiaolai@gmail.com"
        );
    }

    #[test]
    fn test_resolve_prefix_unique() {
        let (store, _dir) = test_store();
        assert_eq!(resolve_email(&store, "li").unwrap(), "lixiaolai@gmail.com");
        assert_eq!(
            resolve_email(&store, "xiaolaid").unwrap(),
            "xiaolaidev@gmail.com"
        );
        assert_eq!(
            resolve_email(&store, "xiaolaia").unwrap(),
            "xiaolaiapple@gmail.com"
        );
    }

    #[test]
    fn test_resolve_ambiguous() {
        let (store, _dir) = test_store();
        let err = resolve_email(&store, "xiaolai").unwrap_err();
        assert!(matches!(err, ResolveError::Ambiguous { .. }));
    }

    #[test]
    fn test_resolve_no_match() {
        let (store, _dir) = test_store();
        let err = resolve_email(&store, "nobody").unwrap_err();
        assert!(matches!(err, ResolveError::NoMatch(_)));
    }

    #[test]
    fn test_resolve_case_insensitive() {
        let (store, _dir) = test_store();
        assert_eq!(
            resolve_email(&store, "LiXiao").unwrap(),
            "lixiaolai@gmail.com"
        );
    }
}
