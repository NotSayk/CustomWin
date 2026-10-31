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
use crate::model::{Action, AdapterKind, Hive, NetAdapterAction, Tweak};
use crate::Error;

/// Driver keys of the network adapter class.
pub const NET_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}";

/// Instance subkeys (`0000`, `0001`…) of the physical adapters a [`NetAdapterAction`]
/// targets. `NCF_PHYSICAL` (0x4) in `Characteristics` rules out virtual adapters (VPN,
/// Hyper-V, loopback); `*IfType` tells Ethernet (6) from Wi-Fi (71).
pub fn net_targets(b: &dyn SystemBackend, n: &NetAdapterAction) -> Result<Vec<String>, Error> {
    let dword = |path: &str, name: &str| -> Result<Option<u32>, Error> {
        Ok(match b.reg_read(Hive::HKLM, path, name)? {
            Some(crate::model::RegValue::Dword(v)) => Some(v),
            _ => None,
        })
    };
    let mut out = Vec::new();
    for instance in b.reg_subkeys(Hive::HKLM, NET_CLASS)? {
        if !is_instance(&instance) {
            continue;
        }
        let path = format!(r"{NET_CLASS}\{instance}");
        let physical = dword(&path, "Characteristics")?.is_some_and(|c| c & 0x4 != 0);
        let kind_ok = matches!(
            (n.adapters, dword(&path, "*IfType")?),
            (AdapterKind::All, Some(6 | 71))
                | (AdapterKind::Ethernet, Some(6))
                | (AdapterKind::Wifi, Some(71))
        );
        if physical
            && kind_ok
            && (!n.only_if_present || b.reg_read(Hive::HKLM, &path, &n.name)?.is_some())
        {
            out.push(instance);
        }
    }
    if out.is_empty() {
        return Err(Error::Unavailable("no matching network adapter".into()));
    }
    Ok(out)
}

/// `0000`–`9999`: a network adapter instance key, nothing else.
fn is_instance(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|c| c.is_ascii_digit())
}

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
        Action::Appx(a) => !b.appx_installed(&a.package)?,
        Action::NetAdapter(n) => {
            let mut all = true;
            for instance in net_targets(b, n)? {
                let path = format!(r"{NET_CLASS}\{instance}");
                all &= b.reg_read(Hive::HKLM, &path, &n.name)?.as_ref() == Some(&n.value);
            }
            all
        }
        Action::Nvidia(nv) => b.nvidia_get(nv.setting)?.value == Some(nv.value),
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
        Action::Appx(a) => PriorState::Appx {
            package: a.package.clone(),
            store_id: a.store_id.clone(),
            was_installed: b.appx_installed(&a.package)?,
        },
        Action::NetAdapter(n) => {
            let mut values = Vec::new();
            for instance in net_targets(b, n)? {
                let path = format!(r"{NET_CLASS}\{instance}");
                let v = b.reg_read(Hive::HKLM, &path, &n.name)?;
                values.push((instance, v));
            }
            PriorState::NetAdapter {
                name: n.name.clone(),
                values,
            }
        }
        Action::Nvidia(nv) => {
            let current = b.nvidia_get(nv.setting)?;
            PriorState::Nvidia {
                setting: nv.setting,
                value: current.value.filter(|_| current.user_set),
            }
        }
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
        Action::Appx(a) => {
            if b.appx_installed(&a.package)? {
                b.appx_remove(&a.package)?;
            }
            Ok(())
        }
        Action::NetAdapter(n) => {
            for instance in net_targets(b, n)? {
                b.reg_write(
                    Hive::HKLM,
                    &format!(r"{NET_CLASS}\{instance}"),
                    &n.name,
                    &n.value,
                )?;
            }
            Ok(())
        }
        Action::Nvidia(nv) => b.nvidia_set(nv.setting, nv.value),
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
        PriorState::Appx {
            package,
            store_id,
            was_installed,
        } => {
            if *was_installed && !b.appx_installed(package)? {
                b.appx_install(store_id)?;
            }
            Ok(())
        }
        PriorState::NetAdapter { name, values } => {
            for (instance, value) in values {
                let path = format!(r"{NET_CLASS}\{instance}");
                match value {
                    Some(v) => b.reg_write(Hive::HKLM, &path, name, v)?,
                    None => b.reg_delete_value(Hive::HKLM, &path, name)?,
                }
            }
            Ok(())
        }
        PriorState::Nvidia { setting, value } => match value {
            Some(v) => b.nvidia_set(*setting, *v),
            None => b.nvidia_reset(*setting),
        },
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
            (
                PriorState::Appx {
                    package, store_id, ..
                },
                Action::Appx(a),
            ) => *package == a.package && *store_id == a.store_id,
            (PriorState::NetAdapter { name, values }, Action::NetAdapter(n)) => {
                name.eq_ignore_ascii_case(&n.name) && values.iter().all(|(i, _)| is_instance(i))
            }
            (PriorState::Nvidia { setting, .. }, Action::Nvidia(nv)) => *setting == nv.setting,
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

/// Whether running `op` changes the signed-in user's own settings.
pub fn touches_user_settings(catalog: &Catalog, journal: &Journal, op: &Operation) -> bool {
    let per_user = |id: &String| catalog.tweak(id).is_some_and(Tweak::per_user);
    match op {
        Operation::Apply(ids) | Operation::Revert(ids) => ids.iter().any(per_user),
        Operation::RevertAll => journal.entries.iter().any(|e| per_user(&e.tweak_id)),
    }
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
    fn appx_is_removed_and_reinstalled_from_the_store() {
        let c = catalog();
        let b = MockBackend::new();
        b.appx.borrow_mut().insert("Microsoft.BingNews".into());
        b.store
            .borrow_mut()
            .insert("9WZDNCRFHVFW".into(), "Microsoft.BingNews".into());
        let t = c.tweak("apps.news").unwrap();
        assert!(!t.fully_reversible());
        let mut j = Journal::default();

        assert_eq!(state(&b, t), TweakState::NotApplied);
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        assert!(!b.appx_installed("Microsoft.BingNews").unwrap());
        assert_eq!(state(&b, t), TweakState::Applied);
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
        assert!(b.appx_installed("Microsoft.BingNews").unwrap());
    }

    #[test]
    fn absent_app_needs_no_reinstall() {
        let c = catalog();
        let b = MockBackend::new(); // store is empty: an install attempt would fail
        let t = c.tweak("apps.news").unwrap();
        let mut j = Journal::default();
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
    }

    #[test]
    fn net_adapter_targets_only_matching_physical_adapters() {
        let c = catalog();
        let b = MockBackend::new();
        b.add_adapter("0001", 6, true); // Ethernet, exposes *EEE
        b.add_adapter("0002", 6, true); // Ethernet without EEE support
        b.add_adapter("0003", 71, true); // Wi-Fi
        b.add_adapter("0004", 6, false); // virtual (Hyper-V…)
        let eth = |i: &str| format!(r"{NET_CLASS}\{i}");
        b.set(
            Hive::HKLM,
            &eth("0001"),
            "*EEE",
            RegValue::String("1".into()),
        );
        b.set(
            Hive::HKLM,
            &eth("0004"),
            "*EEE",
            RegValue::String("1".into()),
        );
        b.set(
            Hive::HKLM,
            NET_CLASS,
            "Class",
            RegValue::String("Net".into()),
        );
        b.set(
            Hive::HKLM,
            &format!(r"{NET_CLASS}\Properties"),
            "x",
            RegValue::Dword(1),
        );

        let mut j = Journal::default();
        let eee = c.tweak("network.eee_off").unwrap();
        apply(&b, eee, &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            b.get(Hive::HKLM, &eth("0001"), "*EEE"),
            Some(RegValue::String("0".into()))
        );
        assert_eq!(b.get(Hive::HKLM, &eth("0002"), "*EEE"), None);
        assert_eq!(
            b.get(Hive::HKLM, &eth("0004"), "*EEE"),
            Some(RegValue::String("1".into()))
        );

        let pnp = c.tweak("network.adapter_power_off_disabled").unwrap();
        apply(&b, pnp, &mut j, &mut no_persist()).unwrap();
        for i in ["0001", "0002", "0003"] {
            assert_eq!(
                b.get(Hive::HKLM, &eth(i), "PnPCapabilities"),
                Some(RegValue::Dword(24))
            );
        }
        assert_eq!(b.get(Hive::HKLM, &eth("0004"), "PnPCapabilities"), None);

        revert(&b, &c, &pnp.id, &mut j, &mut no_persist()).unwrap();
        revert(&b, &c, &eee.id, &mut j, &mut no_persist()).unwrap();
        assert_eq!(
            b.get(Hive::HKLM, &eth("0001"), "*EEE"),
            Some(RegValue::String("1".into()))
        );
        assert_eq!(b.get(Hive::HKLM, &eth("0002"), "PnPCapabilities"), None);

        let green = c.tweak("network.green_ethernet_off").unwrap();
        assert!(
            matches!(state(&b, green), TweakState::Unavailable(_)),
            "no Realtek adapter"
        );
    }

    #[test]
    fn nvidia_setting_restores_user_value_or_default() {
        let c = catalog();
        let b = MockBackend::new();
        let t = c.tweak("nvidia.low_latency").unwrap();
        assert!(
            matches!(state(&b, t), TweakState::Unavailable(_)),
            "no NVIDIA GPU"
        );

        *b.nvidia.borrow_mut() = Some(Default::default());
        let mut j = Journal::default();
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        assert_eq!(state(&b, t), TweakState::Applied);
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
        assert!(
            b.nvidia.borrow().as_ref().unwrap().is_empty(),
            "back to driver default"
        );

        b.nvidia
            .borrow_mut()
            .as_mut()
            .unwrap()
            .insert(0x007BA09E, 3);
        apply(&b, t, &mut j, &mut no_persist()).unwrap();
        revert(&b, &c, &t.id, &mut j, &mut no_persist()).unwrap();
        assert_eq!(b.nvidia.borrow().as_ref().unwrap()[&0x007BA09E], 3);
    }

    #[test]
    fn tampered_net_adapter_journal_is_refused() {
        let c = catalog();
        let b = MockBackend::new();
        let mut j = Journal::default();
        j.entries.push(JournalEntry {
            tweak_id: "network.eee_off".into(),
            applied_at: 0,
            prior: vec![PriorState::NetAdapter {
                name: "*EEE".into(),
                values: vec![(
                    r"..\..\Services\Evil".into(),
                    Some(RegValue::String("x".into())),
                )],
            }],
        });
        assert!(revert(&b, &c, "network.eee_off", &mut j, &mut no_persist()).is_err());
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
