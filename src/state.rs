use crate::error::AppError;
use crate::schema::{read_schema_number, SchemaVersion};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub database: PathBuf,
    pub settings: PathBuf,
    pub live_state: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self, AppError> {
        let home = std::env::var_os("CC_SWITCH_TEST_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .ok_or_else(|| AppError::State("cannot determine the home directory".into()))?;
        Ok(Self::for_home(home))
    }

    pub fn for_home(home: PathBuf) -> Self {
        let app_dir = home.join(".cc-switch");
        Self {
            database: app_dir.join("cc-switch.db"),
            settings: app_dir.join("settings.json"),
            live_state: app_dir.join("live-state.json"),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProviderSummary {
    pub id: String,
    pub name: String,
    pub current: bool,
}

#[derive(Debug)]
pub struct StateReader {
    paths: Paths,
    schema: SchemaVersion,
    settings_supported: bool,
}

#[derive(Default, Deserialize)]
struct CodexLiveState {
    mode: Option<String>,
    proxy_route: Option<String>,
    pending: Option<Value>,
}

impl StateReader {
    pub fn open(paths: Paths) -> Result<Self, AppError> {
        if !paths.database.is_file() {
            return Err(AppError::DatabaseMissing(
                paths.database.display().to_string(),
            ));
        }
        let connection = open_database(&paths.database)?;
        let schema = SchemaVersion::detect(&connection)?;
        let settings_supported = settings_supported_by_adapter(&paths.settings, schema)?;
        Ok(Self {
            paths,
            schema,
            settings_supported,
        })
    }

    #[cfg(test)]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings: Vec<_> = self.schema.warning().into_iter().collect();
        if !self.settings_supported {
            warnings.push(
                "unverified_settings: the bundled adapter cannot fully read CC Switch settings; \
                 switching through the installed CC Switch GUI (started if needed) to preserve them"
                    .into(),
            );
        }
        warnings
    }

    pub fn requires_installed_gui(&self) -> bool {
        !self.settings_supported
            || self.schema.direct_generation() == crate::schema::DirectGeneration::InstalledGui
    }

    pub(crate) fn ensure_direct_settings_supported(&self) -> Result<(), AppError> {
        if !settings_supported_by_adapter(&self.paths.settings, self.schema)? {
            return Err(AppError::State(
                "settings require the installed CC Switch GUI; retry to select that backend".into(),
            ));
        }
        Ok(())
    }

    pub fn providers(&self) -> Result<Vec<ProviderSummary>, AppError> {
        let connection = self.connection()?;
        self.ensure_connection_schema(&connection)?;
        let mut statement = connection
            .prepare(self.schema.providers_query())
            .map_err(state_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })
            .map_err(state_error)?;

        let mut providers = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(state_error)?;
        let effective = self.effective_current_id_from(&providers)?;

        Ok(providers
            .drain(..)
            .map(|(id, name, db_current)| ProviderSummary {
                current: effective.as_deref() == Some(id.as_str())
                    || (effective.is_none() && db_current),
                id,
                name,
            })
            .collect())
    }

    pub fn current(&self) -> Result<Option<ProviderSummary>, AppError> {
        Ok(self
            .providers()?
            .into_iter()
            .find(|provider| provider.current))
    }

    pub fn by_name(&self, name: &str) -> Result<ProviderSummary, AppError> {
        let matches: Vec<_> = self
            .providers()?
            .into_iter()
            .filter(|provider| provider.name == name)
            .collect();
        match matches.as_slice() {
            [] => Err(AppError::ProviderNotFound(name.to_string())),
            [provider] => Ok(provider.clone()),
            providers => Err(AppError::AmbiguousProvider {
                name: name.to_string(),
                ids: providers
                    .iter()
                    .map(|provider| provider.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }

    pub fn by_id(&self, id: &str) -> Result<ProviderSummary, AppError> {
        self.providers()?
            .into_iter()
            .find(|provider| provider.id == id)
            .ok_or_else(|| AppError::ProviderIdNotFound(id.to_string()))
    }

    pub fn has_converged_to(&self, provider_id: &str) -> Result<bool, AppError> {
        let live = self.codex_live_state()?;
        if live.pending.is_some() {
            return Ok(false);
        }
        if live.mode.as_deref() == Some("proxy") {
            if let Some(route) = live.proxy_route {
                if self
                    .providers()?
                    .iter()
                    .any(|provider| provider.id == route)
                {
                    return Ok(route == provider_id);
                }
            }
        }
        let db_current = self.db_current_id()?;
        let file_current = self.settings_current_id()?;
        Ok(db_current.as_deref() == Some(provider_id)
            && file_current.as_deref() == Some(provider_id))
    }

    pub(crate) fn ensure_schema_unchanged(&self) -> Result<SchemaVersion, AppError> {
        let connection = self.connection()?;
        self.ensure_connection_schema(&connection)?;
        self.schema.validate(&connection)?;
        Ok(self.schema)
    }

    fn ensure_connection_schema(&self, connection: &Connection) -> Result<(), AppError> {
        let found = read_schema_number(connection)?;
        if found != self.schema.number() {
            return Err(AppError::SchemaChanged {
                opened: self.schema.number(),
                found,
            });
        }
        Ok(())
    }

    fn connection(&self) -> Result<Connection, AppError> {
        open_database(&self.paths.database)
    }

    fn db_current_id(&self) -> Result<Option<String>, AppError> {
        let connection = self.connection()?;
        self.ensure_connection_schema(&connection)?;
        let mut statement = connection
            .prepare(self.schema.current_provider_query())
            .map_err(state_error)?;
        let mut rows = statement.query([]).map_err(state_error)?;
        rows.next()
            .map_err(state_error)?
            .map(|row| row.get::<_, String>(0).map_err(state_error))
            .transpose()
    }

    fn effective_current_id_from(
        &self,
        providers: &[(String, String, bool)],
    ) -> Result<Option<String>, AppError> {
        let live = self.codex_live_state()?;
        if live.mode.as_deref() == Some("proxy") {
            if let Some(route) = live.proxy_route {
                if providers.iter().any(|(id, _, _)| id == &route) {
                    return Ok(Some(route));
                }
            }
        }
        if let Some(local) = self.settings_current_id()? {
            if providers.iter().any(|(id, _, _)| id == &local) {
                return Ok(Some(local));
            }
        }
        Ok(providers
            .iter()
            .find_map(|(id, _, current)| current.then(|| id.clone())))
    }

    fn settings_current_id(&self) -> Result<Option<String>, AppError> {
        read_settings_current_id(&self.paths.settings)
    }

    fn codex_live_state(&self) -> Result<CodexLiveState, AppError> {
        if !self.schema.uses_live_state() {
            return Ok(CodexLiveState::default());
        }
        let text = match std::fs::read_to_string(&self.paths.live_state) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CodexLiveState::default());
            }
            Err(error) => return Err(state_error(error)),
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| AppError::State("invalid live-state.json".into()))?;
        if !value.is_object() || value.get("apps").is_some_and(|apps| !apps.is_object()) {
            return Err(AppError::State("invalid live-state.json structure".into()));
        }
        let Some(codex) = value.get("apps").and_then(|apps| apps.get("codex")) else {
            return Ok(CodexLiveState::default());
        };
        let state: CodexLiveState = serde_json::from_value(codex.clone())
            .map_err(|_| AppError::State("invalid Codex live state".into()))?;
        if !matches!(state.mode.as_deref(), None | Some("direct" | "proxy")) {
            return Err(AppError::State("unrecognized Codex live mode".into()));
        }
        Ok(state)
    }
}

fn open_database(path: &Path) -> Result<Connection, AppError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(state_error)
}

fn settings_supported_by_adapter(path: &Path, schema: SchemaVersion) -> Result<bool, AppError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(state_error(error)),
    };
    let json = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value: Value =
        serde_json::from_str(json).map_err(|_| AppError::State("invalid settings.json".into()))?;
    if !value.is_object() {
        return Err(AppError::State("invalid settings.json structure".into()));
    }
    Ok(schema.direct_generation().supports_settings(&text))
}

fn read_settings_current_id(path: &Path) -> Result<Option<String>, AppError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(state_error)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value: Value =
        serde_json::from_str(text).map_err(|_| AppError::State("invalid settings.json".into()))?;
    Ok(value
        .get("currentProviderCodex")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned))
}

fn state_error(error: impl std::fmt::Display) -> AppError {
    AppError::State(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    fn fixture(schema: SchemaVersion) -> (TempDir, StateReader) {
        let temp = TempDir::new().unwrap();
        let paths = Paths::for_home(temp.path().to_path_buf());
        std::fs::create_dir_all(paths.database.parent().unwrap()).unwrap();
        let connection = Connection::open(&paths.database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE providers (
                    id TEXT NOT NULL,
                    app_type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    is_current BOOLEAN NOT NULL DEFAULT 0,
                    created_at INTEGER,
                    sort_index INTEGER,
                    PRIMARY KEY (id, app_type)
                 );",
            )
            .unwrap();
        if matches!(
            schema,
            SchemaVersion::V17 | SchemaVersion::V18 | SchemaVersion::V20
        ) {
            connection
                .execute_batch(
                    "CREATE TABLE session_usage_dedup (
                        data_source TEXT NOT NULL,
                        request_id TEXT NOT NULL,
                        semantic_id TEXT NOT NULL,
                        has_entry_id INTEGER NOT NULL DEFAULT 0,
                        PRIMARY KEY (data_source, request_id)
                     );
                     CREATE TABLE session_log_sync (
                        file_path TEXT PRIMARY KEY,
                        last_modified INTEGER NOT NULL,
                        last_line_offset INTEGER NOT NULL DEFAULT 0,
                        last_synced_at INTEGER NOT NULL
                     );",
                )
                .unwrap();
        }
        if matches!(schema, SchemaVersion::V18 | SchemaVersion::V20) {
            connection
                .execute_batch(
                    "ALTER TABLE session_log_sync ADD COLUMN last_byte_offset INTEGER;
                     ALTER TABLE session_log_sync ADD COLUMN last_tail_fingerprint INTEGER;",
                )
                .unwrap();
        }
        if schema == SchemaVersion::V20 {
            connection
                .execute_batch(
                    "CREATE TABLE mcp_servers (enabled_mcode BOOLEAN, enabled_pi BOOLEAN);
                 CREATE TABLE skills (enabled_mcode BOOLEAN);",
                )
                .unwrap();
        }
        connection
            .pragma_update(None, "user_version", schema.number())
            .unwrap();
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, is_current, sort_index)
                 VALUES (?1, 'codex', ?2, ?3, ?4)",
                params!["one", "Provider One", true, 1],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, is_current, sort_index)
                 VALUES (?1, 'codex', ?2, ?3, ?4)",
                params!["two", "Provider Two", false, 2],
            )
            .unwrap();
        drop(connection);
        let reader = StateReader::open(paths).unwrap();
        (temp, reader)
    }

    #[test]
    fn settings_current_takes_precedence_over_database() {
        let (_temp, reader) = fixture(SchemaVersion::V16);
        std::fs::write(
            &reader.paths().settings,
            r#"{"currentProviderCodex":"two"}"#,
        )
        .unwrap();
        assert_eq!(reader.current().unwrap().unwrap().id, "two");
    }

    #[test]
    fn name_matching_is_exact_and_case_sensitive() {
        let (_temp, reader) = fixture(SchemaVersion::V16);
        assert_eq!(reader.by_name("Provider One").unwrap().id, "one");
        assert!(matches!(
            reader.by_name("provider one"),
            Err(AppError::ProviderNotFound(_))
        ));
    }

    #[test]
    fn schema17_profile_reads_providers() {
        let (_temp, reader) = fixture(SchemaVersion::V17);
        assert_eq!(reader.providers().unwrap().len(), 2);
    }

    #[test]
    fn schema18_profile_reads_providers() {
        let (_temp, reader) = fixture(SchemaVersion::V18);
        assert_eq!(reader.providers().unwrap().len(), 2);
    }

    #[test]
    fn unknown_schemas_warn_and_read_without_migration() {
        for number in [0, 15, 19, 21, 999] {
            let (_temp, reader) = fixture(SchemaVersion::Unknown(number));
            assert_eq!(reader.providers().unwrap().len(), 2);
            assert_eq!(reader.warnings().len(), 1);
            assert!(reader.requires_installed_gui());
            assert_eq!(reader.ensure_schema_unchanged().unwrap().number(), number);
        }
    }

    #[test]
    fn schema20_reads_without_warning() {
        let (_temp, reader) = fixture(SchemaVersion::V20);
        assert_eq!(reader.current().unwrap().unwrap().id, "one");
        assert!(reader.warnings().is_empty());
        assert!(!reader.requires_installed_gui());
    }

    #[test]
    fn unrecognized_settings_warn_and_require_gui_for_each_adapter() {
        for schema in [
            SchemaVersion::V16,
            SchemaVersion::V17,
            SchemaVersion::V18,
            SchemaVersion::V20,
        ] {
            for extra in [
                r#""futureSecret":"private-test-value""#,
                r#""visibleApps":{"futureSecret":true}"#,
                r#""showInTray":"private-test-value""#,
            ] {
                let (_temp, reader) = fixture(schema);
                let settings = format!(r#"{{"currentProviderCodex":"one",{extra}}}"#);
                std::fs::write(&reader.paths.settings, &settings).unwrap();
                let reader = StateReader::open(reader.paths).unwrap();
                assert!(reader.requires_installed_gui());
                assert!(reader.has_converged_to("one").unwrap());
                assert_eq!(reader.providers().unwrap().len(), 2);
                let warnings = reader.warnings();
                assert_eq!(warnings.len(), 1);
                assert!(warnings[0].starts_with("unverified_settings:"));
                assert!(!warnings[0].contains("futureSecret"));
                assert!(!warnings[0].contains("private-test-value"));
                assert_eq!(
                    std::fs::read_to_string(&reader.paths.settings).unwrap(),
                    settings
                );
            }
        }
    }

    #[test]
    fn bom_settings_are_readable_but_only_supported_by_latest_adapter() {
        for schema in [
            SchemaVersion::V16,
            SchemaVersion::V17,
            SchemaVersion::V18,
            SchemaVersion::V20,
        ] {
            let (_temp, reader) = fixture(schema);
            std::fs::write(
                &reader.paths.settings,
                "\u{feff}{\"currentProviderCodex\":\"one\"}",
            )
            .unwrap();
            let reader = StateReader::open(reader.paths).unwrap();
            assert!(reader.has_converged_to("one").unwrap());
            assert_eq!(
                reader.requires_installed_gui(),
                schema != SchemaVersion::V20
            );
        }
    }

    #[test]
    fn malformed_settings_fail_without_disclosing_contents() {
        for settings in [
            r#"{"secret":"private-test-value",}"#,
            r#"["private-test-value"]"#,
        ] {
            let (_temp, reader) = fixture(SchemaVersion::V20);
            std::fs::write(&reader.paths.settings, settings).unwrap();
            let error = StateReader::open(reader.paths).unwrap_err();
            assert!(matches!(error, AppError::State(_)));
            assert!(!error.to_string().contains("private-test-value"));
        }
    }

    #[test]
    fn tied_sort_indices_preserve_upstream_database_order() {
        let (_temp, reader) = fixture(SchemaVersion::V20);
        Connection::open(&reader.paths.database)
            .unwrap()
            .execute_batch(
                "UPDATE providers SET sort_index = 1, created_at = 10;
             UPDATE providers SET name = 'Z' WHERE id = 'one';
             UPDATE providers SET name = 'A' WHERE id = 'two';",
            )
            .unwrap();
        let ids: Vec<_> = reader
            .providers()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["one", "two"]);
    }

    #[test]
    fn proxy_route_takes_precedence_and_converges_without_changing_direct_pointer() {
        for schema in [SchemaVersion::V20, SchemaVersion::Unknown(21)] {
            let (_temp, reader) = fixture(schema);
            std::fs::write(&reader.paths.settings, r#"{"currentProviderCodex":"one"}"#).unwrap();
            std::fs::write(
                &reader.paths.live_state,
                r#"{"apps":{"codex":{"mode":"proxy","proxy_route":"two"}}}"#,
            )
            .unwrap();
            assert_eq!(reader.current().unwrap().unwrap().id, "two");
            assert!(reader.has_converged_to("two").unwrap());
            assert!(!reader.has_converged_to("one").unwrap());
        }
    }

    #[test]
    fn direct_mode_ignores_saved_proxy_route() {
        let (_temp, reader) = fixture(SchemaVersion::V20);
        std::fs::write(&reader.paths.settings, r#"{"currentProviderCodex":"one"}"#).unwrap();
        std::fs::write(
            &reader.paths.live_state,
            r#"{"apps":{"codex":{"mode":"direct","proxy_route":"two"}}}"#,
        )
        .unwrap();
        assert_eq!(reader.current().unwrap().unwrap().id, "one");
        assert!(reader.has_converged_to("one").unwrap());
    }

    #[test]
    fn pending_switch_does_not_report_success() {
        let (_temp, reader) = fixture(SchemaVersion::V20);
        std::fs::write(
            &reader.paths.live_state,
            r#"{"apps":{"codex":{"mode":"proxy","proxy_route":"two","pending":{}}}}"#,
        )
        .unwrap();
        assert!(!reader.has_converged_to("two").unwrap());
    }

    #[test]
    fn malformed_live_state_still_errors() {
        let (_temp, reader) = fixture(SchemaVersion::Unknown(21));
        std::fs::write(&reader.paths.live_state, "invalid").unwrap();
        assert!(matches!(reader.current(), Err(AppError::State(_))));
    }

    #[test]
    fn unknown_schema_with_unreadable_provider_layout_still_errors() {
        let (_temp, reader) = fixture(SchemaVersion::Unknown(21));
        Connection::open(&reader.paths.database)
            .unwrap()
            .execute_batch("ALTER TABLE providers DROP COLUMN name;")
            .unwrap();
        assert!(matches!(reader.providers(), Err(AppError::State(_))));
    }

    #[test]
    fn schema20_requires_pi_mcp_column() {
        let (_temp, reader) = fixture(SchemaVersion::V20);
        Connection::open(&reader.paths.database)
            .unwrap()
            .execute_batch("ALTER TABLE mcp_servers DROP COLUMN enabled_pi;")
            .unwrap();
        assert!(matches!(
            StateReader::open(reader.paths),
            Err(AppError::IncompatibleSchemaLayout { schema: 20, .. })
        ));
    }

    #[test]
    fn rejects_schema17_without_required_layout() {
        let (temp, _reader) = fixture(SchemaVersion::V16);
        let paths = Paths::for_home(temp.path().to_path_buf());
        let connection = Connection::open(&paths.database).unwrap();
        connection.pragma_update(None, "user_version", 17).unwrap();
        drop(connection);
        assert!(matches!(
            StateReader::open(paths),
            Err(AppError::IncompatibleSchemaLayout { schema: 17, .. })
        ));
    }

    #[test]
    fn rejects_schema18_without_byte_cursor_layout() {
        let (temp, _reader) = fixture(SchemaVersion::V17);
        let paths = Paths::for_home(temp.path().to_path_buf());
        let connection = Connection::open(&paths.database).unwrap();
        connection.pragma_update(None, "user_version", 18).unwrap();
        drop(connection);
        assert!(matches!(
            StateReader::open(paths),
            Err(AppError::IncompatibleSchemaLayout { schema: 18, .. })
        ));
    }

    #[test]
    fn detects_schema_change_after_open() {
        let (_temp, reader) = fixture(SchemaVersion::V16);
        let connection = Connection::open(&reader.paths().database).unwrap();
        connection.pragma_update(None, "user_version", 17).unwrap();
        drop(connection);
        assert!(matches!(
            reader.ensure_schema_unchanged(),
            Err(AppError::SchemaChanged {
                opened: 16,
                found: 17
            })
        ));
    }
}
