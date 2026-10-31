//! The undo journal: the original state of every setting CustomWin changed.
//!
//! An entry is written the first time a tweak is applied and kept until that tweak is
//! reverted, so re-applying a tweak never overwrites the original state.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::{Hive, RegValue, StartType, SysParam};
use crate::Error;

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriorState {
    Registry {
        hive: Hive,
        path: String,
        name: String,
        /// `None`: the value did not exist and will be deleted on revert.
        value: Option<RegValue>,
        /// Topmost key that CustomWin created; pruned on revert if left empty.
        created_root: Option<String>,
    },
    Service {
        name: String,
        startup: StartType,
        was_running: bool,
    },
    PowerPlan {
        scheme: String,
    },
    SystemParameter {
        param: SysParam,
        value: bool,
    },
    Appx {
        package: String,
        store_id: String,
        was_installed: bool,
    },
    NetAdapter {
        name: String,
        /// Adapter instance (`0001`) → original value, `None` if it did not exist.
        values: Vec<(String, Option<RegValue>)>,
    },
    Nvidia {
        setting: u32,
        /// `None`: the driver default applied (no user value).
        value: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub tweak_id: String,
    /// Unix timestamp, seconds.
    pub applied_at: u64,
    /// One per action of the tweak, in the same order.
    pub prior: Vec<PriorState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub entries: Vec<JournalEntry>,
}

impl Default for Journal {
    fn default() -> Self {
        Journal {
            version: VERSION,
            entries: Vec::new(),
        }
    }
}

impl Journal {
    /// Loads the journal, or returns an empty one if the file does not exist yet.
    pub fn load(path: &Path) -> Result<Self, Error> {
        match fs::read(path) {
            Ok(bytes) => {
                let j: Journal = serde_json::from_slice(&bytes)
                    .map_err(|e| Error::Journal(format!("{}: {e}", path.display())))?;
                if j.version != VERSION {
                    return Err(Error::Journal(format!(
                        "unsupported journal version {}",
                        j.version
                    )));
                }
                Ok(j)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Journal::default()),
            Err(e) => Err(Error::Journal(format!("{}: {e}", path.display()))),
        }
    }

    /// Writes the journal atomically (temporary file, flush, rename).
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let io = |e: std::io::Error| Error::Journal(format!("{}: {e}", path.display()));
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self).map_err(|e| Error::Journal(e.to_string()))?;
        let mut f = fs::File::create(&tmp).map_err(io)?;
        f.write_all(&json).map_err(io)?;
        f.sync_all().map_err(io)?;
        drop(f);
        fs::rename(&tmp, path).map_err(io)
    }

    pub fn entry(&self, tweak_id: &str) -> Option<&JournalEntry> {
        self.entries.iter().find(|e| e.tweak_id == tweak_id)
    }

    pub fn contains(&self, tweak_id: &str) -> bool {
        self.entry(tweak_id).is_some()
    }

    pub(crate) fn remove(&mut self, tweak_id: &str) {
        self.entries.retain(|e| e.tweak_id != tweak_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("customwin-journal-{}", std::process::id()));
        let path = dir.join("journal.json");
        assert_eq!(Journal::load(&path).unwrap(), Journal::default());

        let mut j = Journal::default();
        j.entries.push(JournalEntry {
            tweak_id: "a.b".into(),
            applied_at: 1,
            prior: vec![
                PriorState::Registry {
                    hive: Hive::HKCU,
                    path: r"Software\X".into(),
                    name: "V".into(),
                    value: Some(RegValue::String("é".into())),
                    created_root: None,
                },
                PriorState::Service {
                    name: "S".into(),
                    startup: StartType::Automatic,
                    was_running: true,
                },
            ],
        });
        j.save(&path).unwrap();
        assert_eq!(Journal::load(&path).unwrap(), j);
        fs::remove_dir_all(dir).unwrap();
    }
}
