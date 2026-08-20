use crate::error::AppError;
use cc_switch_upstream::{AppState, AppType, Database, ProviderService};
use std::sync::Arc;

pub fn switch(provider_id: &str) -> Result<Vec<String>, AppError> {
    let database = Arc::new(Database::init().map_err(upstream_error)?);
    let (proxy_enabled, _) = database.get_proxy_flags_sync("codex");
    database
        .set_proxy_flags_sync("codex", proxy_enabled, false)
        .map_err(upstream_error)?;

    let state = AppState::new(database);
    ProviderService::switch(&state, AppType::Codex, provider_id)
        .map(|result| result.warnings)
        .map_err(upstream_error)
}

fn upstream_error(_error: impl std::fmt::Display) -> AppError {
    AppError::Switch("the pinned CC Switch operation returned an error".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Paths, StateReader};
    use cc_switch_upstream::Provider;
    use serde_json::json;

    struct TestHome {
        previous: Option<std::ffi::OsString>,
    }

    impl TestHome {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
            std::env::set_var("CC_SWITCH_TEST_HOME", path);
            Self { previous }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    fn provider(id: &str, name: &str, key: &str) -> Provider {
        Provider::with_id(
            id.into(),
            name.into(),
            json!({
                "auth": { "OPENAI_API_KEY": key },
                "config": format!(
                    "model_provider = \"custom\"\n\
                     [model_providers.custom]\n\
                     name = \"custom\"\n\
                     base_url = \"https://example.invalid/v1\"\n\
                     wire_api = \"chat\"\n"
                )
            }),
            None,
        )
    }

    #[test]
    fn direct_switch_preserves_proxy_enabled_and_disables_failover() {
        let temp = tempfile::tempdir().unwrap();
        let _home = TestHome::set(temp.path());
        let database = Database::init().unwrap();
        database
            .save_provider("codex", &provider("one", "One", "test-one"))
            .unwrap();
        database
            .save_provider("codex", &provider("two", "Two", "test-two"))
            .unwrap();
        database.set_current_provider("codex", "one").unwrap();
        database.set_proxy_flags_sync("codex", true, true).unwrap();
        drop(database);

        switch("two").unwrap();

        let reader = StateReader::open(Paths::for_home(temp.path().to_path_buf())).unwrap();
        assert!(reader.has_converged_to("two").unwrap());
        let database = Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, false));
    }
}
