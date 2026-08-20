use crate::error::AppError;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const SUPPORTED_SCHEMA: i32 = 16;

#[derive(Clone, Debug)]
pub struct Paths {
    pub database: PathBuf,
    pub settings: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self, AppError> {
        let home = dirs::home_dir()
            .ok_or_else(|| AppError::State("cannot determine the home directory".into()))?;
        Ok(Self::for_home(home))
    }

    pub fn for_home(home: PathBuf) -> Self {
        let app_dir = home.join(".cc-switch");
        Self {
            database: app_dir.join("cc-switch.db"),
            settings: app_dir.join("settings.json"),
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
}

impl StateReader {
    pub fn open(paths: Paths) -> Result<Self, AppError> {
        if !paths.database.is_file() {
            return Err(AppError::DatabaseMissing(
                paths.database.display().to_string(),
            ));
        }
        let reader = Self { paths };
        reader.ensure_compatible()?;
        Ok(reader)
    }

    #[cfg(test)]
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn providers(&self) -> Result<Vec<ProviderSummary>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, is_current FROM providers \
                 WHERE app_type = 'codex' \
                 ORDER BY \
                    CASE WHEN sort_index IS NULL THEN 1 ELSE 0 END, \
                    sort_index, \
                    CASE WHEN created_at IS NULL THEN 0 ELSE 1 END, \
                    created_at, name, id",
            )
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
        let db_current = self.db_current_id()?;
        let file_current = self.settings_current_id()?;
        Ok(db_current.as_deref() == Some(provider_id)
            && file_current.as_deref() == Some(provider_id))
    }

    fn ensure_compatible(&self) -> Result<(), AppError> {
        let connection = self.connection()?;
        let found = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
            .map_err(state_error)?;
        if found != SUPPORTED_SCHEMA {
            return Err(AppError::IncompatibleSchema {
                found,
                expected: SUPPORTED_SCHEMA,
            });
        }
        Ok(())
    }

    fn connection(&self) -> Result<Connection, AppError> {
        Connection::open_with_flags(
            &self.paths.database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(state_error)
    }

    fn db_current_id(&self) -> Result<Option<String>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id FROM providers \
                 WHERE app_type = 'codex' AND is_current = 1 \
                 ORDER BY id LIMIT 1",
            )
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
}

fn read_settings_current_id(path: &Path) -> Result<Option<String>, AppError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(state_error)?;
    let value: Value = serde_json::from_str(&text).map_err(state_error)?;
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

    fn fixture() -> (TempDir, StateReader) {
        let temp = TempDir::new().unwrap();
        let paths = Paths::for_home(temp.path().to_path_buf());
        std::fs::create_dir_all(paths.database.parent().unwrap()).unwrap();
        let connection = Connection::open(&paths.database).unwrap();
        connection
            .execute_batch(
                "PRAGMA user_version = 16;
                 CREATE TABLE providers (
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
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, is_current, sort_index)
                 VALUES (?1, 'codex', ?2, ?3, ?4)",
                params!["one", "IPADS", true, 1],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, is_current, sort_index)
                 VALUES (?1, 'codex', ?2, ?3, ?4)",
                params!["two", "CCTQ", false, 2],
            )
            .unwrap();
        drop(connection);
        let reader = StateReader::open(paths).unwrap();
        (temp, reader)
    }

    #[test]
    fn settings_current_takes_precedence_over_database() {
        let (_temp, reader) = fixture();
        std::fs::write(
            &reader.paths().settings,
            r#"{"currentProviderCodex":"two"}"#,
        )
        .unwrap();
        assert_eq!(reader.current().unwrap().unwrap().id, "two");
    }

    #[test]
    fn name_matching_is_exact_and_case_sensitive() {
        let (_temp, reader) = fixture();
        assert_eq!(reader.by_name("IPADS").unwrap().id, "one");
        assert!(matches!(
            reader.by_name("ipads"),
            Err(AppError::ProviderNotFound(_))
        ));
    }

    #[test]
    fn rejects_wrong_schema() {
        let (temp, _reader) = fixture();
        let paths = Paths::for_home(temp.path().to_path_buf());
        let connection = Connection::open(&paths.database).unwrap();
        connection.pragma_update(None, "user_version", 17).unwrap();
        drop(connection);
        assert!(matches!(
            StateReader::open(paths),
            Err(AppError::IncompatibleSchema {
                found: 17,
                expected: 16
            })
        ));
    }
}
