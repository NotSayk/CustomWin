//! Abstraction over the system calls the engine needs.
//!
//! The real implementation lives in [`crate::windows`]; [`mock::MockBackend`] is an
//! in-memory stand-in that lets the engine be tested on any OS.

use crate::model::{Hive, RegValue, StartType, SysParam};
use crate::Error;

pub trait SystemBackend {
    fn reg_key_exists(&self, hive: Hive, path: &str) -> Result<bool, Error>;
    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>, Error>;
    /// Writes a value, creating the key and its missing parents.
    fn reg_write(&self, hive: Hive, path: &str, name: &str, value: &RegValue) -> Result<(), Error>;
    /// Deletes a value. A missing value is not an error.
    fn reg_delete_value(&self, hive: Hive, path: &str, name: &str) -> Result<(), Error>;
    /// Deletes a key only if it has no values and no subkeys. Returns whether it was deleted.
    fn reg_delete_key_if_empty(&self, hive: Hive, path: &str) -> Result<bool, Error>;

    /// `None` if the service does not exist.
    fn service_query(&self, name: &str) -> Result<Option<ServiceState>, Error>;
    fn service_set_startup(&self, name: &str, startup: StartType) -> Result<(), Error>;
    fn service_stop(&self, name: &str) -> Result<(), Error>;
    fn service_start(&self, name: &str) -> Result<(), Error>;

    /// Active power scheme GUID, lowercase.
    fn power_active_scheme(&self) -> Result<String, Error>;
    fn power_set_active_scheme(&self, guid: &str) -> Result<(), Error>;

    fn sysparam_get(&self, param: SysParam) -> Result<bool, Error>;
    fn sysparam_set(&self, param: SysParam, value: bool) -> Result<(), Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceState {
    pub startup: StartType,
    pub running: bool,
}

/// Splits `A\B\C` into its ancestors-or-self: `A`, `A\B`, `A\B\C`.
pub(crate) fn key_prefixes(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('\\')
        .map(move |(i, _)| &path[..i])
        .chain(std::iter::once(path))
}

pub mod mock {
    //! In-memory backend for tests. Keys and value names are case-insensitive, like Windows.

    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    use super::*;

    #[derive(Default)]
    pub struct MockBackend {
        keys: RefCell<BTreeSet<(Hive, String)>>,
        values: RefCell<BTreeMap<(Hive, String, String), RegValue>>,
        pub services: RefCell<HashMap<String, ServiceState>>,
        pub power_schemes: RefCell<Vec<String>>,
        pub active_scheme: RefCell<String>,
        pub sysparams: RefCell<HashMap<SysParam, bool>>,
        /// Any registry write whose path contains this string fails.
        pub fail_writes_containing: RefCell<Option<String>>,
    }

    fn norm(s: &str) -> String {
        s.to_ascii_lowercase()
    }

    impl MockBackend {
        pub fn new() -> Self {
            let m = MockBackend::default();
            *m.active_scheme.borrow_mut() = "381b4222-f694-41f0-9685-ff5bb260df2e".into();
            m.power_schemes.borrow_mut().extend([
                "381b4222-f694-41f0-9685-ff5bb260df2e".to_string(),
                "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c".to_string(),
            ]);
            for p in [
                SysParam::ClientAreaAnimation,
                SysParam::WindowAnimation,
                SysParam::MouseAcceleration,
            ] {
                m.sysparams.borrow_mut().insert(p, true);
            }
            m
        }

        pub fn with_service(self, name: &str, startup: StartType, running: bool) -> Self {
            self.services
                .borrow_mut()
                .insert(norm(name), ServiceState { startup, running });
            self
        }

        pub fn set(&self, hive: Hive, path: &str, name: &str, value: RegValue) {
            self.reg_write(hive, path, name, &value).unwrap();
        }

        pub fn get(&self, hive: Hive, path: &str, name: &str) -> Option<RegValue> {
            self.reg_read(hive, path, name).unwrap()
        }

        pub fn key_count(&self) -> usize {
            self.keys.borrow().len()
        }
    }

    impl SystemBackend for MockBackend {
        fn reg_key_exists(&self, hive: Hive, path: &str) -> Result<bool, Error> {
            Ok(self.keys.borrow().contains(&(hive, norm(path))))
        }

        fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>, Error> {
            Ok(self
                .values
                .borrow()
                .get(&(hive, norm(path), norm(name)))
                .cloned())
        }

        fn reg_write(
            &self,
            hive: Hive,
            path: &str,
            name: &str,
            value: &RegValue,
        ) -> Result<(), Error> {
            if let Some(f) = self.fail_writes_containing.borrow().as_deref() {
                if path.contains(f) {
                    return Err(Error::System(format!("simulated failure writing {path}")));
                }
            }
            let mut keys = self.keys.borrow_mut();
            for prefix in key_prefixes(path) {
                keys.insert((hive, norm(prefix)));
            }
            self.values
                .borrow_mut()
                .insert((hive, norm(path), norm(name)), value.clone());
            Ok(())
        }

        fn reg_delete_value(&self, hive: Hive, path: &str, name: &str) -> Result<(), Error> {
            self.values
                .borrow_mut()
                .remove(&(hive, norm(path), norm(name)));
            Ok(())
        }

        fn reg_delete_key_if_empty(&self, hive: Hive, path: &str) -> Result<bool, Error> {
            let p = norm(path);
            let child_prefix = format!("{p}\\");
            let has_values = self
                .values
                .borrow()
                .keys()
                .any(|(h, k, _)| *h == hive && *k == p);
            let has_children = self
                .keys
                .borrow()
                .iter()
                .any(|(h, k)| *h == hive && k.starts_with(&child_prefix));
            if has_values || has_children {
                return Ok(false);
            }
            Ok(self.keys.borrow_mut().remove(&(hive, p)))
        }

        fn service_query(&self, name: &str) -> Result<Option<ServiceState>, Error> {
            Ok(self.services.borrow().get(&norm(name)).copied())
        }

        fn service_set_startup(&self, name: &str, startup: StartType) -> Result<(), Error> {
            match self.services.borrow_mut().get_mut(&norm(name)) {
                Some(s) => {
                    s.startup = startup;
                    Ok(())
                }
                None => Err(Error::Unavailable(format!("service {name} not found"))),
            }
        }

        fn service_stop(&self, name: &str) -> Result<(), Error> {
            if let Some(s) = self.services.borrow_mut().get_mut(&norm(name)) {
                s.running = false;
            }
            Ok(())
        }

        fn service_start(&self, name: &str) -> Result<(), Error> {
            if let Some(s) = self.services.borrow_mut().get_mut(&norm(name)) {
                s.running = true;
            }
            Ok(())
        }

        fn power_active_scheme(&self) -> Result<String, Error> {
            Ok(self.active_scheme.borrow().clone())
        }

        fn power_set_active_scheme(&self, guid: &str) -> Result<(), Error> {
            if !self.power_schemes.borrow().iter().any(|s| s == guid) {
                return Err(Error::Unavailable(format!(
                    "power scheme {guid} does not exist"
                )));
            }
            *self.active_scheme.borrow_mut() = guid.to_string();
            Ok(())
        }

        fn sysparam_get(&self, param: SysParam) -> Result<bool, Error> {
            Ok(self.sysparams.borrow().get(&param).copied().unwrap_or(true))
        }

        fn sysparam_set(&self, param: SysParam, value: bool) -> Result<(), Error> {
            self.sysparams.borrow_mut().insert(param, value);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockBackend;
    use super::*;

    #[test]
    fn prefixes() {
        let v: Vec<_> = key_prefixes(r"A\B\C").collect();
        assert_eq!(v, ["A", r"A\B", r"A\B\C"]);
        assert_eq!(key_prefixes("A").collect::<Vec<_>>(), ["A"]);
    }

    #[test]
    fn mock_is_case_insensitive_and_prunes_only_empty_keys() {
        let m = MockBackend::new();
        m.set(Hive::HKCU, r"Software\X\Y", "V", RegValue::Dword(1));
        assert_eq!(
            m.get(Hive::HKCU, r"software\x\y", "v"),
            Some(RegValue::Dword(1))
        );
        assert!(!m
            .reg_delete_key_if_empty(Hive::HKCU, r"Software\X")
            .unwrap());
        m.reg_delete_value(Hive::HKCU, r"Software\X\Y", "V")
            .unwrap();
        assert!(m
            .reg_delete_key_if_empty(Hive::HKCU, r"Software\X\Y")
            .unwrap());
        assert!(m
            .reg_delete_key_if_empty(Hive::HKCU, r"Software\X")
            .unwrap());
    }
}
