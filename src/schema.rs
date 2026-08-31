use crate::error::AppError;
use rusqlite::Connection;

const PROVIDERS_V1_QUERY: &str = "SELECT id, name, is_current FROM providers \
     WHERE app_type = 'codex' \
     ORDER BY \
        CASE WHEN sort_index IS NULL THEN 1 ELSE 0 END, \
        sort_index, \
        CASE WHEN created_at IS NULL THEN 0 ELSE 1 END, \
        created_at, name, id";

const CURRENT_PROVIDER_V1_QUERY: &str = "SELECT id FROM providers \
     WHERE app_type = 'codex' AND is_current = 1 \
     ORDER BY id LIMIT 1";

const PROVIDERS_V1_COLUMNS: &[&str] = &[
    "id",
    "app_type",
    "name",
    "is_current",
    "sort_index",
    "created_at",
];

const SCHEMA_17_REQUIREMENTS: &[LayoutRequirement] =
    &[LayoutRequirement::Table("session_usage_dedup")];

const SCHEMA_18_REQUIREMENTS: &[LayoutRequirement] = &[
    LayoutRequirement::Table("session_usage_dedup"),
    LayoutRequirement::TableColumns {
        table: "session_log_sync",
        columns: &["last_byte_offset", "last_tail_fingerprint"],
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaVersion {
    V16,
    V17,
    V18,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectGeneration {
    V16,
    V17,
    V18,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateLayout {
    ProvidersV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayoutRequirement {
    Table(&'static str),
    TableColumns {
        table: &'static str,
        columns: &'static [&'static str],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CompatibilityProfile {
    number: i32,
    state_layout: StateLayout,
    direct_generation: DirectGeneration,
    requirements: &'static [LayoutRequirement],
}

impl SchemaVersion {
    const SUPPORTED: [Self; 3] = [Self::V16, Self::V17, Self::V18];

    pub fn detect(connection: &Connection) -> Result<Self, AppError> {
        let found = read_schema_number(connection)?;
        let schema = match found {
            16 => Self::V16,
            17 => Self::V17,
            18 => Self::V18,
            _ => {
                return Err(AppError::IncompatibleSchema {
                    found,
                    supported: Self::SUPPORTED
                        .iter()
                        .map(|schema| schema.number())
                        .collect(),
                });
            }
        };
        schema.validate(connection)?;
        Ok(schema)
    }

    pub const fn number(self) -> i32 {
        self.profile().number
    }

    pub const fn providers_query(self) -> &'static str {
        self.state_layout().providers_query()
    }

    pub const fn current_provider_query(self) -> &'static str {
        self.state_layout().current_provider_query()
    }

    pub const fn direct_generation(self) -> DirectGeneration {
        self.profile().direct_generation
    }

    pub fn validate(self, connection: &Connection) -> Result<(), AppError> {
        let profile = self.profile();
        profile.state_layout.validate(connection, self)?;
        for requirement in profile.requirements {
            requirement.validate(connection, self)?;
        }
        Ok(())
    }

    const fn state_layout(self) -> StateLayout {
        self.profile().state_layout
    }

    const fn profile(self) -> CompatibilityProfile {
        match self {
            Self::V16 => CompatibilityProfile {
                number: 16,
                state_layout: StateLayout::ProvidersV1,
                direct_generation: DirectGeneration::V16,
                requirements: &[],
            },
            Self::V17 => CompatibilityProfile {
                number: 17,
                state_layout: StateLayout::ProvidersV1,
                direct_generation: DirectGeneration::V17,
                requirements: SCHEMA_17_REQUIREMENTS,
            },
            Self::V18 => CompatibilityProfile {
                number: 18,
                state_layout: StateLayout::ProvidersV1,
                direct_generation: DirectGeneration::V18,
                requirements: SCHEMA_18_REQUIREMENTS,
            },
        }
    }
}

impl StateLayout {
    const fn providers_query(self) -> &'static str {
        match self {
            Self::ProvidersV1 => PROVIDERS_V1_QUERY,
        }
    }

    const fn current_provider_query(self) -> &'static str {
        match self {
            Self::ProvidersV1 => CURRENT_PROVIDER_V1_QUERY,
        }
    }

    fn validate(self, connection: &Connection, schema: SchemaVersion) -> Result<(), AppError> {
        match self {
            Self::ProvidersV1 => {
                require_table(connection, schema, "providers")?;
                require_columns(connection, schema, "providers", PROVIDERS_V1_COLUMNS)
            }
        }
    }
}

impl LayoutRequirement {
    fn validate(self, connection: &Connection, schema: SchemaVersion) -> Result<(), AppError> {
        match self {
            Self::Table(table) => require_table(connection, schema, table),
            Self::TableColumns { table, columns } => {
                require_table(connection, schema, table)?;
                require_columns(connection, schema, table, columns)
            }
        }
    }
}

pub fn read_schema_number(connection: &Connection) -> Result<i32, AppError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(state_error)
}

fn require_table(
    connection: &Connection,
    schema: SchemaVersion,
    table: &str,
) -> Result<(), AppError> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, bool>(0),
        )
        .map_err(state_error)?;
    if exists {
        return Ok(());
    }
    Err(layout_error(schema, format!("missing table {table:?}")))
}

fn require_columns(
    connection: &Connection,
    schema: SchemaVersion,
    table: &str,
    required: &[&str],
) -> Result<(), AppError> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(state_error)?;
    let columns = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(state_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(state_error)?;
    let missing = required
        .iter()
        .filter(|required| !columns.iter().any(|column| column == **required))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(layout_error(
        schema,
        format!("table {table:?} is missing columns: {}", missing.join(", ")),
    ))
}

fn layout_error(schema: SchemaVersion, detail: String) -> AppError {
    AppError::IncompatibleSchemaLayout {
        schema: schema.number(),
        detail,
    }
}

fn state_error(error: impl std::fmt::Display) -> AppError {
    AppError::State(error.to_string())
}
