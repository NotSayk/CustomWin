//! Applies and reverts tweaks.
//!
//! Safety rules:
//! 1. The original state of every action is captured *before* anything changes, and the
//!    journal entry is persisted to disk before the first write. A crash mid-tweak still
//!    leaves enough information to undo it.
//! 2. A tweak is all-or-nothing: if one action fails, the actions already done are rolled
//!    back.
//! 3. The journal is untrusted input (it lives in a user-writable folder and is replayed by
//!    an elevated process), so an entry is only replayed if every target it touches is one
//!    the catalog's tweak declares.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::backend::{key_prefixes, SystemBackend};
use crate::catalog::Catalog;
use crate::journal::{Journal, JournalEntry, PriorState};
use crate::model::{Action, Tweak};
use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum TweakState {
    Applied,
    NotApplied,
    /// Some actions hold their target value, others do not.
    Partial,
    /// The tweak cannot be applied on this machine (missing service, plan…).
    Unavailable(String),
}

pub fn state(b: &dyn SystemBackend, tweak: &Tweak) -> TweakState {
    let mut matched = 0;
    for a in &tweak.actions {
        match action_matches(b, a) {
            Ok(true) => matched += 1,
            Ok(false) => {}
            Err(e) => return TweakState::Unavailable(e.to_string()),
        }
    }
    match matched {
        0 => TweakState::NotApplied,
        n if n == tweak.actions.len() => TweakState::Applied,
        _ => TweakState::Partial,
    }
}

fn action_matches(b: &dyn SystemBackend, a: &Action) -> Result<bool, Error> {
    Ok(match a {
        Action::Registry(r) => b.reg_read(r.hive, &r.path, &r.name)?.as_ref() == Some(&r.value),
        Action::Service(s) => match b.service_query(&s.name)? {
            Some(st) => st.startup == s.startup,
            None => return Err(Error::Unavailable(format!("service {} not found", s.name))),
        },
        Action::PowerPlan(p) => b.power_active_scheme()?.eq_ignore_ascii_case(&p.scheme),
        Action::SystemParameter(p) => b.sysparam_get(p.param)? == p.value,
    })
}

fn capture(b: &dyn SystemBackend, a: &Action) -> Result<PriorState, Error> {
    Ok(match a {
        Action::Registry(r) => {
            let mut created_root = None;
            for prefix in key_prefixes(&r.path) {
                if !b.reg_key_exists(r.hive, prefix)? {
                    created_root = Some(prefix.to_string());
                    break;
                }
            }
            PriorState::Registry {
                hive: r.hive,
                path: r.path.clone(),
                name: r.name.clone(),
                value: b.reg_read(r.hive, &r.path, &r.name)?,
                created_root,
            }
        }
        Action::Service(s) => {
            let st = b
                .service_query(&s.name)?
                .ok_or_else(|| Error::Unavailable(format!("service {} not found", s.name)))?;
            PriorState::Service {
                name: s.name.clone(),
                startup: st.startup,
                was_running: st.running,
            }
        }
        Action::PowerPlan(_) => PriorState::PowerPlan {
            scheme: b.power_active_scheme()?,
        },
        Action::SystemParameter(p) => PriorState::SystemParameter {
            param: p.param,
            value: b.sysparam_get(p.param)?,
        },
    })
}

fn execute(b: &dyn SystemBackend, a: &Action) -> Result<(), Error> {
    match a {
        Action::Registry(r) => b.reg_write(r.hive, &r.path, &r.name, &r.value),
        Action::Service(s) => {
            b.service_set_startup(&s.name, s.startup)?;
            if s.stop && b.service_query(&s.name)?.is_some_and(|st| st.running) {
                b.service_stop(&s.name)?;
            }
            Ok(())
        }
        Action::PowerPlan(p) => b.power_set_active_scheme(&p.scheme),
        Action::SystemParameter(p) => b.sysparam_set(p.param, p.value),
    }
}

fn restore(b: &dyn SystemBackend, prior: &PriorState) -> Result<(), Error> {
    match prior {
        PriorState::Registry {
            hive,
            path,
            name,
            value,
            created_root,
        } => {
            match value {
                Some(v) => b.reg_write(*hive, path, name, v)?,
                None => b.reg_delete_value(*hive, path, name)?,
            }
            if let Some(root) = created_root {
                // Delete the keys we created, deepest first, stopping at the first one
                // that something else has put content in.
                let created: Vec<&str> = key_prefixes(path)
                    .skip_while(|p| !p.eq_ignore_ascii_case(root))
                    .collect();
                for key in created.into_iter().rev() {
                    if !b.reg_delete_key_if_empty(*hive, key)? {
                        break;
                    }
                }
            }
            Ok(())
        }
        PriorState::Service {
            name,
            startup,
            was_running,
        } => {
            b.service_set_startup(name, *startup)?;
            if *was_running && b.service_query(name)?.is_some_and(|st| !st.running) {
                b.service_start(name)?;
            }
            Ok(())
        }
        PriorState::PowerPlan { scheme } => b.power_set_active_scheme(scheme),
        PriorState::SystemParameter { param, value } => b.sysparam_set(*param, *value),
    }
}

/// Persists the journal. Called before the first system change and after each change
/// to the journal itself.
pub type Persist<'a> = dyn FnMut(&Journal) -> Result<(), Error> + 'a;

pub fn apply(
    b: &dyn SystemBackend,
    tweak: &Tweak,
    journal: &mut Journal,
    persist: &mut Persist,
) -> Result<(), Error> {
    let priors = tweak
        .actions
        .iter()
        .map(|a| capture(b, a))
        .collect::<Result<Vec<_>, _>>()?;

    let newly_journaled = !journal.contains(&tweak.id);
    if newly_journaled {
        journal.entries.push(JournalEntry {
            tweak_id: tweak.id.clone(),
            applied_at: now(),
            prior: priors.clone(),
        });
        persist(journal)?;
    }

    for (i, action) in tweak.actions.iter().enumerate() {
        if let Err(e) = execute(b, action) {
            for p in priors[..=i].iter().rev() {
                let _ = restore(b, p);
            }
            if newly_journaled {
                journal.remove(&tweak.id);
                persist(journal)?;
            }
            return Err(e);
        }
    }
    Ok(())
}

pub fn revert(
    b: &dyn SystemBackend,
    catalog: &Catalog,
    tweak_id: &str,
    journal: &mut Journal,
    persist: &mut Persist,
) -> Result<(), Error> {
    let entry = journal
        .entry(tweak_id)
        .cloned()
        .ok_or_else(|| Error::NotInJournal(tweak_id.to_string()))?;
    let tweak = catalog
        .tweak(tweak_id)
        .ok_or_else(|| Error::Journal(format!("journal references unknown tweak `{tweak_id}`")))?;
    validate_entry(tweak, &entry)?;

    let mut first_error = None;
    for p in entry.prior.iter().rev() {
        if let Err(e) = restore(b, p) {
            first_error.get_or_insert(e);
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => {
            journal.remove(tweak_id);
            persist(journal)
        }
    }
}

/// Checks that a journal entry only touches the targets its catalog tweak declares.
pub fn validate_entry(tweak: &Tweak, entry: &JournalEntry) -> Result<(), Error> {
    let reject = || {
        Err(Error::Journal(format!(
            "journal entry for `{}` does not match the catalog",
            tweak.id
        )))
    };
    if entry.prior.len() != tweak.actions.len() {
        return reject();
    }
    for (prior, action) in entry.prior.iter().zip(&tweak.actions) {
        let ok = match (prior, action) {
            (
                PriorState::Registry {
                    hive,
                    path,
                    name,
                    created_root,
                    ..
                },
                Action::Registry(r),
            ) => {
                *hive == r.hive
                    && path.eq_ignore_ascii_case(&r.path)
                    && name.eq_ignore_ascii_case(&r.name)
                    && created_root.as_deref().is_none_or(|root| {
                        key_prefixes(&r.path).any(|p| p.eq_ignore_ascii_case(root))
                    })
            }
            (PriorState::Service { name, .. }, Action::Service(s)) => {
                name.eq_ignore_ascii_case(&s.name)
            }
            (PriorState::PowerPlan { scheme }, Action::PowerPlan(_)) => {
                crate::catalog::is_guid(scheme)
            }
            (PriorState::SystemParameter { param, .. }, Action::SystemParameter(p)) => {
                *param == p.param
            }
            _ => false,
        };
        if !ok {
            return reject();
        }
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A batch of work, as sent by the UI to the elevated helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "ids", rename_all = "snake_case")]
pub enum Operation {
    Apply(Vec<String>),
    Revert(Vec<String>),
    RevertAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpResult {
    pub tweak_id: String,
    pub error: Option<String>,
}

/// Runs an operation against the journal stored at `journal_path`.
pub fn run(
    b: &dyn SystemBackend,
    catalog: &Catalog,
    journal_path: &Path,
    op: &Operation,
) -> Result<Vec<OpResult>, Error> {
    let mut journal = Journal::load(journal_path)?;
    let mut persist = |j: &Journal| j.save(journal_path);
    let result = |id: &str, r: Result<(), Error>| OpResult {
        tweak_id: id.to_string(),
        error: r.err().map(|e| e.to_string()),
    };

    Ok(match op {
        Operation::Apply(ids) => ids
            .iter()
            .map(|id| {
                let r = catalog
                    .tweak(id)
                    .ok_or_else(|| Error::Invalid(format!("unknown tweak `{id}`")))
                    .and_then(|t| apply(b, t, &mut journal, &mut persist));
                result(id, r)
            })
            .collect(),
        Operation::Revert(ids) => ids
            .iter()
            .map(|id| result(id, revert(b, catalog, id, &mut journal, &mut persist)))
            .collect(),
        Operation::RevertAll => {
            // Most recent first, so overlapping tweaks unwind in the right order.
            let ids: Vec<String> = journal
                .entries
                .iter()
                .rev()
                .map(|e| e.tweak_id.clone())
                .collect();
            ids.iter()
                .map(|id| result(id, revert(b, catalog, id, &mut journal, &mut persist)))
                .collect()
        }
    })
}

/// Whether running `op` needs an elevated process.
pub fn requires_admin(catalog: &Catalog, journal: &Journal, op: &Operation) -> bool {
    let needs = |id: &String| catalog.tweak(id).is_some_and(Tweak::requires_admin);
    match op {
        Operation::Apply(ids) | Operation::Revert(ids) => ids.iter().any(needs),
        Operation::RevertAll => journal.entries.iter().any(|e| needs(&e.tweak_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::model::{Hive, RegValue, StartType, SysParam};

    fn catalog() -> Catalog {
        Catalog::embedded().unwrap()
    }

    fn no_persist() -> impl FnMut(&Journal) -> Result<(), Error> {
        |_| Ok(())
    }

    const ADV: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

    #[test]
    fn apply_then_revert_restores_existing_value() {
        let c = catalog();
        let b = MockBackend::new();
        b.set(Hive::HKCU, ADV, "TaskbarAl", RegValue::Dword(1));
        let t = c.tweak("interface.taskbar_left").unwrap();
        let mut j = Journal::default();

        assert_eq!(state(&b, t), TweakState::NotApplied);
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        assert_eq!(state(&b, t), TweakState::Applied);
        assert_eq!(
            b.get(Hive::HKCU, ADV, "TaskbarAl"),
            Some(RegValue::Dword(0))
        );

        revert(&b, &c, t.id.as_str(), &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            b.get(Hive::HKCU, ADV, "TaskbarAl"),
            Some(RegValue::Dword(1))
        );
        assert!(j.entries.is_empty());
    }

    #[test]
    fn revert_deletes_created_keys_but_keeps_preexisting_ones() {
        let c = catalog();
        let b = MockBackend::new();
        b.set(
            Hive::HKCU,
            r"Software\Classes\CLSID\{other}",
            "",
            RegValue::String("x".into()),
        );
        let keys_before = b.key_count();
        let t = c.tweak("interface.classic_context_menu").unwrap();
        let mut j = Journal::default();

        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        assert_eq!(state(&b, t), TweakState::Applied);
        match &j.entries[0].prior[0] {
            PriorState::Registry {
                created_root,
                value,
                ..
            } => {
                assert_eq!(
                    created_root.as_deref(),
                    Some(r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}")
                );
                assert_eq!(value, &None);
            }
            p => panic!("{p:?}"),
        }
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
        assert_eq!(b.key_count(), keys_before);
        assert!(b
            .reg_key_exists(Hive::HKCU, r"Software\Classes\CLSID\{other}")
            .unwrap());
        assert!(!b
            .reg_key_exists(
                Hive::HKCU,
                r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"
            )
            .unwrap());
    }

    #[test]
    fn reapplying_keeps_the_original_state() {
        let c = catalog();
        let b = MockBackend::new();
        b.set(Hive::HKCU, ADV, "TaskbarAl", RegValue::Dword(1));
        let t = c.tweak("interface.taskbar_left").unwrap();
        let mut j = Journal::default();
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        assert_eq!(j.entries.len(), 1);
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            b.get(Hive::HKCU, ADV, "TaskbarAl"),
            Some(RegValue::Dword(1))
        );
    }

    #[test]
    fn failed_action_rolls_back_the_whole_tweak() {
        let c = catalog();
        let b = MockBackend::new();
        // ads.start_recommendations: Explorer\Advanced first, then ContentDeliveryManager.
        *b.fail_writes_containing.borrow_mut() = Some("ContentDeliveryManager".into());
        let t = c.tweak("ads.start_recommendations").unwrap();
        let mut j = Journal::default();
        let mut saves = 0;
        let mut persist = |_: &Journal| {
            saves += 1;
            Ok(())
        };
        assert!(apply(&b, t, &mut j, &mut persist).is_err());
        assert_eq!(b.get(Hive::HKCU, ADV, "Start_IrisRecommendations"), None);
        assert!(
            !b.reg_key_exists(Hive::HKCU, "Software").unwrap(),
            "created keys must be pruned"
        );
        assert!(j.entries.is_empty());
        assert_eq!(
            saves, 2,
            "journal written before the change, then cleaned up"
        );
    }

    #[test]
    fn service_is_disabled_stopped_and_restored() {
        let c = catalog();
        let b = MockBackend::new().with_service("DiagTrack", StartType::Automatic, true);
        let t = c.tweak("privacy.diagtrack_service").unwrap();
        let mut j = Journal::default();
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            b.service_query("DiagTrack").unwrap().unwrap().startup,
            StartType::Disabled
        );
        assert!(!b.service_query("DiagTrack").unwrap().unwrap().running);
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
        let st = b.service_query("DiagTrack").unwrap().unwrap();
        assert_eq!((st.startup, st.running), (StartType::Automatic, true));
    }

    #[test]
    fn missing_service_is_unavailable() {
        let c = catalog();
        let b = MockBackend::new();
        let t = c.tweak("privacy.diagtrack_service").unwrap();
        assert!(matches!(state(&b, t), TweakState::Unavailable(_)));
        assert!(apply(&b, t, &mut Journal::default(), &mut no_persist()).is_err());
    }

    #[test]
    fn power_plan_and_sysparams() {
        let c = catalog();
        let b = MockBackend::new();
        let mut j = Journal::default();
        let plan = c.tweak("power.high_performance").unwrap();
        let anim = c.tweak("interface.reduce_animations").unwrap();
        apply(&b, plan, &mut j, &mut no_persist()).unwrap();
        apply(&b, anim, &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            *b.active_scheme.borrow(),
            "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c"
        );
        assert!(!b.sysparam_get(SysParam::WindowAnimation).unwrap());

        b.power_schemes.borrow_mut().clear();
        b.power_schemes
            .borrow_mut()
            .push("381b4222-f694-41f0-9685-ff5bb260df2e".into());
        revert(&b, &c, &plan.id, &mut j, &mut no_persist()).unwrap();
        revert(&b, &c, &anim.id, &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            *b.active_scheme.borrow(),
            "381b4222-f694-41f0-9685-ff5bb260df2e"
        );
        assert!(b.sysparam_get(SysParam::WindowAnimation).unwrap());
    }

    #[test]
    fn tampered_journal_is_refused() {
        let c = catalog();
        let b = MockBackend::new();
        let mut j = Journal::default();
        j.entries.push(JournalEntry {
            tweak_id: "interface.taskbar_left".into(),
            applied_at: 0,
            prior: vec![PriorState::Registry {
                hive: Hive::HKLM,
                path: r"SYSTEM\CurrentControlSet\Services\Evil".into(),
                name: "ImagePath".into(),
                value: Some(RegValue::String(r"C:\evil.exe".into())),
                created_root: None,
            }],
        });
        assert!(revert(&b, &c, "interface.taskbar_left", &mut j, &mut no_persist()).is_err());
        assert_eq!(
            b.get(
                Hive::HKLM,
                r"SYSTEM\CurrentControlSet\Services\Evil",
                "ImagePath"
            ),
            None
        );
    }

    #[test]
    fn run_revert_all_through_a_real_journal_file() {
        let c = catalog();
        let b = MockBackend::new().with_service("DiagTrack", StartType::Automatic, true);
        let dir = std::env::temp_dir().join(format!("customwin-engine-{}", std::process::id()));
        let path = dir.join("journal.json");
        let ids: Vec<String> = c.profile("minimal").unwrap().tweaks.clone();

        let applied = run(&b, &c, &path, &Operation::Apply(ids.clone())).unwrap();
        assert!(applied.iter().all(|r| r.error.is_none()), "{applied:?}");
        assert_eq!(Journal::load(&path).unwrap().entries.len(), ids.len());
        assert!(requires_admin(
            &c,
            &Journal::load(&path).unwrap(),
            &Operation::RevertAll
        ));

        let reverted = run(&b, &c, &path, &Operation::RevertAll).unwrap();
        assert!(reverted.iter().all(|r| r.error.is_none()), "{reverted:?}");
        assert!(Journal::load(&path).unwrap().entries.is_empty());
        assert_eq!(b.key_count(), 0, "every key created by the profile is gone");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
