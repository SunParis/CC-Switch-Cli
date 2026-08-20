use std::sync::Arc;

use cc_switch_schema16 as schema16;
use cc_switch_schema17 as schema17;

use crate::error::AppError;
use crate::schema::DirectGeneration;
use crate::state::StateReader;

pub fn switch(reader: &StateReader, provider_id: &str) -> Result<Vec<String>, AppError> {
    let schema = reader.ensure_schema_unchanged()?;
    let warnings = match schema.direct_generation() {
        DirectGeneration::V16 => switch_schema16(provider_id),
        DirectGeneration::V17 => switch_schema17(provider_id),
    }?;
    reader.ensure_schema_unchanged()?;
    Ok(warnings)
}

fn switch_schema16(provider_id: &str) -> Result<Vec<String>, AppError> {
    let database = Arc::new(schema16::Database::init().map_err(upstream_error)?);
    let (proxy_enabled, _) = database.get_proxy_flags_sync("codex");
    database
        .set_proxy_flags_sync("codex", proxy_enabled, false)
        .map_err(upstream_error)?;

    let state = schema16::AppState::new(database);
    schema16::ProviderService::switch(&state, schema16::AppType::Codex, provider_id)
        .map(|result| result.warnings)
        .map_err(upstream_error)
}

fn switch_schema17(provider_id: &str) -> Result<Vec<String>, AppError> {
    let database = Arc::new(schema17::Database::init().map_err(upstream_error)?);
    let (proxy_enabled, _) = database.get_proxy_flags_sync("codex");
    database
        .set_proxy_flags_sync("codex", proxy_enabled, false)
        .map_err(upstream_error)?;

    let state = schema17::AppState::new(database);
    schema17::ProviderService::switch(&state, schema17::AppType::Codex, provider_id)
        .map(|result| result.warnings)
        .map_err(upstream_error)
}

fn upstream_error(_error: impl std::fmt::Display) -> AppError {
    AppError::Switch("the pinned CC Switch operation returned an error".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SchemaVersion;
    use crate::state::Paths;
    use serde_json::json;
    use std::sync::Mutex;

    static TEST_HOME_LOCK: Mutex<()> = Mutex::new(());

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

    fn provider_settings(key: &str) -> serde_json::Value {
        json!({
            "auth": { "OPENAI_API_KEY": key },
            "config": format!(
                "model_provider = \"custom\"\n\
                 [model_providers.custom]\n\
                 name = \"custom\"\n\
                 base_url = \"https://example.invalid/v1\"\n\
                 wire_api = \"chat\"\n"
            )
        })
    }

    fn provider16(id: &str, name: &str, key: &str) -> schema16::Provider {
        schema16::Provider::with_id(id.into(), name.into(), provider_settings(key), None)
    }

    fn provider17(id: &str, name: &str, key: &str) -> schema17::Provider {
        schema17::Provider::with_id(id.into(), name.into(), provider_settings(key), None)
    }

    #[test]
    fn schema16_direct_switch_preserves_schema_and_proxy_state() {
        let _lock = TEST_HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home = TestHome::set(temp.path());
        let database = schema16::Database::init().unwrap();
        database
            .save_provider("codex", &provider16("one", "One", "test-one"))
            .unwrap();
        database
            .save_provider("codex", &provider16("two", "Two", "test-two"))
            .unwrap();
        database.set_current_provider("codex", "one").unwrap();
        database.set_proxy_flags_sync("codex", true, true).unwrap();
        drop(database);

        let reader = StateReader::open(Paths::for_home(temp.path().to_path_buf())).unwrap();
        switch(&reader, "two").unwrap();

        assert!(reader.has_converged_to("two").unwrap());
        assert_eq!(
            reader.ensure_schema_unchanged().unwrap(),
            SchemaVersion::V16
        );
        let database = schema16::Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, false));
    }

    #[test]
    fn schema17_direct_switch_preserves_schema_and_proxy_state() {
        let _lock = TEST_HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home = TestHome::set(temp.path());
        let database = schema17::Database::init().unwrap();
        database
            .save_provider("codex", &provider17("one", "One", "test-one"))
            .unwrap();
        database
            .save_provider("codex", &provider17("two", "Two", "test-two"))
            .unwrap();
        database.set_current_provider("codex", "one").unwrap();
        database.set_proxy_flags_sync("codex", true, true).unwrap();
        drop(database);

        let reader = StateReader::open(Paths::for_home(temp.path().to_path_buf())).unwrap();
        switch(&reader, "two").unwrap();

        assert!(reader.has_converged_to("two").unwrap());
        assert_eq!(
            reader.ensure_schema_unchanged().unwrap(),
            SchemaVersion::V17
        );
        let database = schema17::Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, false));
    }
}
