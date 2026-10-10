use serde::Serialize;
use std::process::ExitCode;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("CC Switch database not found at {0}")]
    DatabaseMissing(String),

    #[error(
        "CC Switch database schema {schema} does not match its compatibility profile: {detail}"
    )]
    IncompatibleSchemaLayout { schema: i32, detail: String },

    #[error("CC Switch database schema changed while switching (opened as {opened}, now {found})")]
    SchemaChanged { opened: i32, found: i32 },

    #[error("failed to read CC Switch state: {0}")]
    State(String),

    #[error("no Codex provider named {0:?}")]
    ProviderNotFound(String),

    #[error("multiple Codex providers are named {name:?}; use --id ({ids})")]
    AmbiguousProvider { name: String, ids: String },

    #[error("no Codex provider with id {0:?}")]
    ProviderIdNotFound(String),

    #[error("CC Switch is running, but its tray D-Bus interface is unavailable: {0}")]
    GuiUnavailable(String),

    #[error("CC Switch GUI request failed: {0}")]
    Gui(String),

    #[error("provider switch failed: {0}")]
    Switch(String),

    #[error("provider switch did not converge within 10 seconds")]
    Timeout,
}

impl AppError {
    pub fn exit_code(&self) -> ExitCode {
        let code = match self {
            Self::DatabaseMissing(_)
            | Self::IncompatibleSchemaLayout { .. }
            | Self::SchemaChanged { .. }
            | Self::State(_) => 3,
            Self::ProviderNotFound(_)
            | Self::AmbiguousProvider { .. }
            | Self::ProviderIdNotFound(_) => 4,
            Self::GuiUnavailable(_) | Self::Gui(_) => 5,
            Self::Switch(_) | Self::Timeout => 6,
        };
        ExitCode::from(code)
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::DatabaseMissing(_) => "database_missing",
            Self::IncompatibleSchemaLayout { .. } => "incompatible_schema_layout",
            Self::SchemaChanged { .. } => "schema_changed",
            Self::State(_) => "state_error",
            Self::ProviderNotFound(_) => "provider_not_found",
            Self::AmbiguousProvider { .. } => "ambiguous_provider",
            Self::ProviderIdNotFound(_) => "provider_id_not_found",
            Self::GuiUnavailable(_) => "gui_unavailable",
            Self::Gui(_) => "gui_error",
            Self::Switch(_) => "switch_failed",
            Self::Timeout => "switch_timeout",
        }
    }
}

#[derive(Serialize)]
pub struct ErrorOutput<'a> {
    pub error: ErrorBody<'a>,
}

#[derive(Serialize)]
pub struct ErrorBody<'a> {
    pub kind: &'a str,
    pub message: String,
}

impl<'a> From<&'a AppError> for ErrorOutput<'a> {
    fn from(error: &'a AppError) -> Self {
        Self {
            error: ErrorBody {
                kind: error.kind(),
                message: error.to_string(),
            },
        }
    }
}
