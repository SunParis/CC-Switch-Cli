use std::sync::Arc;

use cc_switch_schema16 as schema16;
use cc_switch_schema17 as schema17;
use cc_switch_schema18 as schema18;
use cc_switch_schema20 as schema20;
use cc_switch_schema23 as schema23;

use crate::error::AppError;
use crate::schema::DirectGeneration;
use crate::state::StateReader;

pub fn switch(reader: &StateReader, provider_id: &str) -> Result<Vec<String>, AppError> {
    let schema = reader.ensure_schema_unchanged()?;
    reader.ensure_direct_settings_supported()?;
    let warnings = match schema.direct_generation() {
        DirectGeneration::V16 => switch_schema16(provider_id),
        DirectGeneration::V17 => switch_schema17(provider_id),
        DirectGeneration::V18 => switch_schema18(provider_id),
        DirectGeneration::V20 => switch_schema20(provider_id),
        DirectGeneration::V23 => switch_schema23(provider_id),
        DirectGeneration::InstalledGui => Err(AppError::Switch(
            "this operation must be dispatched through the installed CC Switch GUI".into(),
        )),
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

fn switch_schema18(provider_id: &str) -> Result<Vec<String>, AppError> {
    let database = Arc::new(schema18::Database::init().map_err(upstream_error)?);
    let (proxy_enabled, _) = database.get_proxy_flags_sync("codex");
    database
        .set_proxy_flags_sync("codex", proxy_enabled, false)
        .map_err(upstream_error)?;

    let state = schema18::AppState::new(database);
    schema18::ProviderService::switch(&state, schema18::AppType::Codex, provider_id)
        .map(|result| result.warnings)
        .map_err(upstream_error)
}

fn upstream_error(_error: impl std::fmt::Display) -> AppError {
    AppError::Switch("the pinned CC Switch operation returned an error".into())
}

fn switch_schema20(provider_id: &str) -> Result<Vec<String>, AppError> {
    let database = Arc::new(schema20::Database::init().map_err(upstream_error)?);
    let state = schema20::AppState::new(database);
    if schema20::mode::current::is_proxy(&schema20::AppType::Codex)
        && state.db.get_proxy_flags_sync("codex").1
        && !schema20::mode::stack::stack_mode_now(&schema20::AppType::Codex)
    {
        return Err(AppError::Switch(
            "Codex auto-failover is active; disable it in CC Switch before switching manually"
                .into(),
        ));
    }
    schema20::ProviderService::switch(&state, schema20::AppType::Codex, provider_id)
        .map(|result| result.warnings)
        .map_err(upstream_error)
}

fn switch_schema23(provider_id: &str) -> Result<Vec<String>, AppError> {
    let database = Arc::new(schema23::Database::init().map_err(upstream_error)?);
    let state = schema23::AppState::new(database);
    if schema23::mode::current::is_proxy(&schema23::AppType::Codex)
        && state.db.get_proxy_flags_sync("codex").1
        && !schema23::mode::stack::stack_mode_now(&schema23::AppType::Codex)
    {
        return Err(AppError::Switch(
            "Codex auto-failover is active; disable it in CC Switch before switching manually"
                .into(),
        ));
    }
    schema23::ProviderService::switch(&state, schema23::AppType::Codex, provider_id)
        .map(|result| result.warnings)
        .map_err(upstream_error)
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

    fn provider18(id: &str, name: &str, key: &str) -> schema18::Provider {
        schema18::Provider::with_id(id.into(), name.into(), provider_settings(key), None)
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
            SchemaVersion::new(16)
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
            SchemaVersion::new(17)
        );
        let database = schema17::Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, false));
    }

    #[test]
    fn schema18_direct_switch_preserves_schema_and_proxy_state() {
        let _lock = TEST_HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home = TestHome::set(temp.path());
        let database = schema18::Database::init().unwrap();
        database
            .save_provider("codex", &provider18("one", "One", "test-one"))
            .unwrap();
        database
            .save_provider("codex", &provider18("two", "Two", "test-two"))
            .unwrap();
        database.set_current_provider("codex", "one").unwrap();
        database.set_proxy_flags_sync("codex", true, true).unwrap();
        drop(database);

        let reader = StateReader::open(Paths::for_home(temp.path().to_path_buf())).unwrap();
        switch(&reader, "two").unwrap();

        assert!(reader.has_converged_to("two").unwrap());
        assert_eq!(
            reader.ensure_schema_unchanged().unwrap(),
            SchemaVersion::new(18)
        );
        let database = schema18::Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, false));
    }

    #[test]
    fn schema20_switch_preserves_latest_settings_schema_and_failover() {
        let _lock = TEST_HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home = TestHome::set(temp.path());
        let paths = Paths::for_home(temp.path().to_path_buf());
        let mut settings = json!({
            "currentProviderCodex": "one",
            "quotaDisplay": "used",
            "proxyClientHost": "proxy.example.invalid",
            "codexStackClassicTools": false,
            "codexStackSubagentModel": "ccs-test/model",
            "codexStackSubagentReasoningEffort": "high",
            "codexStackAutoReviewModel": "ccs-test/review",
            "usageDisplay": {
                "currency": "CNY",
                "exchangeRates": {"CNY": 6.8},
                "sidebarTodayItems": ["cost", "requests"],
                "requestLogColumns": {"statusCode": true, "app": false},
                "trendChartType": "area"
            },
            "localMigrations": {
                "codexLegacyOfficialHistoryBucketV1": {
                    "completedAt": "2026-10-19T00:00:00Z",
                    "targetProviderId": "openai",
                    "migratedJsonlFiles": 2,
                    "migratedStateRows": 3,
                    "codexConfigDir": temp.path().join(".codex")
                }
            }
        });
        std::fs::create_dir_all(paths.settings.parent().unwrap()).unwrap();
        // Write before the upstream adapter initializes its process-wide settings cache.
        std::fs::write(&paths.settings, format!("\u{feff}{settings}")).unwrap();
        let database = schema20::Database::init().unwrap();
        for (id, name, key) in [("one", "One", "test-one"), ("two", "Two", "test-two")] {
            let provider =
                schema20::Provider::with_id(id.into(), name.into(), provider_settings(key), None);
            database.save_provider("codex", &provider).unwrap();
        }
        database.set_current_provider("codex", "one").unwrap();
        database.set_proxy_flags_sync("codex", true, true).unwrap();
        drop(database);
        let reader = StateReader::open(paths.clone()).unwrap();
        assert!(reader.warnings().is_empty());
        assert!(!reader.requires_installed_gui());

        switch(&reader, "two").unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.settings).unwrap()).unwrap();
        settings["currentProviderCodex"] = json!("two");
        for (key, expected) in settings.as_object().unwrap() {
            assert_eq!(&saved[key], expected, "setting was not preserved: {key}");
        }
        assert!(reader.has_converged_to("two").unwrap());
        assert_eq!(
            reader.ensure_schema_unchanged().unwrap(),
            SchemaVersion::new(20)
        );
        let config = std::fs::read_to_string(temp.path().join(".codex/config.toml")).unwrap();
        assert!(config.contains("experimental_bearer_token = \"test-two\""));
        let database = schema20::Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, true));

        assert!(matches!(
            switch(&reader, "missing"),
            Err(AppError::Switch(_))
        ));
        assert!(reader.has_converged_to("two").unwrap());

        // Recheck before writes if a newer release changes settings after backend selection.
        settings["futureSetting"] = json!(true);
        let settings_bytes = serde_json::to_vec(&settings).unwrap();
        std::fs::write(&paths.settings, &settings_bytes).unwrap();
        let database_before = std::fs::read(&paths.database).unwrap();
        assert!(matches!(switch(&reader, "one"), Err(AppError::State(_))));
        assert_eq!(std::fs::read(&paths.settings).unwrap(), settings_bytes);
        assert_eq!(std::fs::read(&paths.database).unwrap(), database_before);
        assert!(reader.has_converged_to("two").unwrap());
    }

    #[test]
    fn schema23_switch_preserves_latest_settings_and_schema() {
        let _lock = TEST_HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home = TestHome::set(temp.path());
        let paths = Paths::for_home(temp.path().to_path_buf());
        let settings: schema23::AppSettings = serde_json::from_value(json!({
            "currentProviderCodex": "one",
            "lightweightMode": true,
            "nativeTrayMenu": true,
            "appOrder": ["codex", "omp", "kimi", "agy"],
            "ompConfigDir": temp.path().join("omp-test"),
            "currentProviderAgy": "unrelated",
            "visibleApps": {"codex": true, "omp": false, "kimi": true, "agy": false},
            "quotaDisplay": "used",
            "proxyClientHost": "proxy.example.invalid",
            "codexStackSubagentModel": "ccs-test/model"
        }))
        .unwrap();
        let mut expected = serde_json::to_value(settings).unwrap();
        std::fs::create_dir_all(paths.settings.parent().unwrap()).unwrap();
        std::fs::write(&paths.settings, serde_json::to_vec(&expected).unwrap()).unwrap();

        let database = schema23::Database::init().unwrap();
        for id in ["one", "two"] {
            let provider =
                schema23::Provider::with_id(id.into(), id.into(), provider_settings(id), None);
            database.save_provider("codex", &provider).unwrap();
        }
        database.set_current_provider("codex", "one").unwrap();
        database.set_proxy_flags_sync("codex", true, true).unwrap();
        drop(database);
        let reader = StateReader::open(paths.clone()).unwrap();
        assert!(reader.warnings().is_empty());
        assert!(!reader.requires_installed_gui());

        switch(&reader, "two").unwrap();

        expected["currentProviderCodex"] = json!("two");
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.settings).unwrap()).unwrap();
        assert_eq!(saved, expected);
        assert!(reader.has_converged_to("two").unwrap());
        assert_eq!(reader.ensure_schema_unchanged().unwrap().number(), 23);
        assert!(
            std::fs::read_to_string(temp.path().join(".codex/config.toml"))
                .unwrap()
                .contains("experimental_bearer_token = \"two\"")
        );
        let database = schema23::Database::init().unwrap();
        assert_eq!(database.get_proxy_flags_sync("codex"), (true, true));
        assert!(matches!(
            switch(&reader, "missing"),
            Err(AppError::Switch(_))
        ));
        assert!(reader.has_converged_to("two").unwrap());
    }

    #[test]
    fn gui_backed_schemas_never_enter_a_bundled_writer_or_migrate() {
        let _lock = TEST_HOME_LOCK.lock().unwrap();
        for number in (1..=15).chain([19, 21, 22, 0, 24, 999]) {
            let temp = tempfile::tempdir().unwrap();
            let _home = TestHome::set(temp.path());
            let paths = Paths::for_home(temp.path().to_path_buf());
            std::fs::create_dir_all(paths.database.parent().unwrap()).unwrap();
            let db = rusqlite::Connection::open(&paths.database).unwrap();
            crate::test_support::create_schema(&db, number);
            drop(db);
            let before = std::fs::read(&paths.database).unwrap();
            let reader = StateReader::open(paths.clone()).unwrap();
            assert!(
                matches!(switch(&reader, "two"), Err(AppError::Switch(_))),
                "schema {number}"
            );
            assert_eq!(std::fs::read(&paths.database).unwrap(), before);
            assert!(!paths.settings.exists());
            assert!(!temp.path().join(".codex").exists());
        }
    }
}
