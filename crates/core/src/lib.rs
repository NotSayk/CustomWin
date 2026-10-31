//! CustomWin core: the reversible tweak engine, its catalog, and the machine analysis.
//!
//! The crate is platform-neutral except for [`windows`], which holds the real system
//! backend and the hardware probes. Everything else runs (and is tested) on any OS
//! against [`backend::mock::MockBackend`].

pub mod analysis;
pub mod backend;
pub mod catalog;
pub mod engine;
pub mod journal;
pub mod model;
#[cfg(windows)]
pub mod windows;

pub use catalog::Catalog;
pub use engine::{OpResult, Operation, TweakState};

/// Request the UI sends to the elevated helper, as a JSON file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HelperRequest {
    pub operation: Operation,
    pub journal_path: std::path::PathBuf,
    /// Create a System Restore point before applying.
    #[serde(default)]
    pub restore_point: bool,
}

/// What the helper writes back next to the request.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HelperResponse {
    pub results: Vec<OpResult>,
    pub warnings: Vec<String>,
    /// Set when the whole operation failed before any tweak ran.
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("journal: {0}")]
    Journal(String),
    #[error("{0}")]
    System(String),
    #[error("access denied: {0}")]
    AccessDenied(String),
    #[error("unavailable on this PC: {0}")]
    Unavailable(String),
    #[error("tweak `{0}` was not applied by CustomWin, nothing to undo")]
    NotInJournal(String),
    #[error("{0}")]
    Invalid(String),
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
