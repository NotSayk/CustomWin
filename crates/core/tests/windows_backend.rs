//! Exercises the real Windows backend against the registry, inside a scratch key under
//! HKCU so nothing outside it is touched. Runs on Windows (and under Wine in CI-less
//! environments).
#![cfg(windows)]

use customwin_core::backend::SystemBackend;
use customwin_core::engine::{self, TweakState};
use customwin_core::journal::Journal;
use customwin_core::model::*;
use customwin_core::windows::{collect_system_info, WindowsBackend};

fn scratch(test: &str) -> String {
    format!(r"Software\CustomWinTest\{test}-{}", std::process::id())
}

fn tweak(id: &str, path: &str, value: RegValue) -> Tweak {
    Tweak {
        id: id.into(),
        category: Category::Interface,
        name: Localized {
            fr: "t".into(),
            en: "t".into(),
        },
        description: Localized {
            fr: "t".into(),
            en: "t".into(),
        },
        risk: Risk::Safe,
        evidence: Evidence::Functional,
        restart: Restart::None,
        tags: vec![],
        note: None,
        actions: vec![Action::Registry(RegistryAction {
            hive: Hive::HKCU,
            path: path.into(),
            name: "V".into(),
            value,
        })],
    }
}

#[test]
fn registry_roundtrip_all_types() {
    let b = WindowsBackend;
    let root = scratch("types");
    let path = format!(r"{root}\A\B");
    assert!(!b.reg_key_exists(Hive::HKCU, &root).unwrap());

    for v in [
        RegValue::Dword(7),
        RegValue::Qword(1 << 40),
        RegValue::String("héllo".into()),
        RegValue::String(String::new()),
        RegValue::Raw {
            reg_type: 3,
            data: vec![1, 2, 3],
        },
    ] {
        b.reg_write(Hive::HKCU, &path, "V", &v).unwrap();
        assert_eq!(b.reg_read(Hive::HKCU, &path, "V").unwrap(), Some(v));
    }
    b.reg_write(Hive::HKCU, &path, "", &RegValue::String("default".into()))
        .unwrap();
    assert_eq!(
        b.reg_read(Hive::HKCU, &path, "").unwrap(),
        Some(RegValue::String("default".into()))
    );

    b.reg_delete_value(Hive::HKCU, &path, "V").unwrap();
    b.reg_delete_value(Hive::HKCU, &path, "V").unwrap(); // missing is fine
    assert_eq!(b.reg_read(Hive::HKCU, &path, "V").unwrap(), None);
    assert!(
        !b.reg_delete_key_if_empty(Hive::HKCU, &path).unwrap(),
        "still holds (Default)"
    );
    b.reg_delete_value(Hive::HKCU, &path, "").unwrap();
    assert!(b.reg_delete_key_if_empty(Hive::HKCU, &path).unwrap());
    assert!(b
        .reg_delete_key_if_empty(Hive::HKCU, &format!(r"{root}\A"))
        .unwrap());
    assert!(b.reg_delete_key_if_empty(Hive::HKCU, &root).unwrap());
    assert!(!b.reg_key_exists(Hive::HKCU, &root).unwrap());
}

#[test]
fn engine_apply_revert_on_real_registry() {
    let b = WindowsBackend;
    let root = scratch("engine");
    let t = tweak(
        "interface.test",
        &format!(r"{root}\Deep\Key"),
        RegValue::Dword(0),
    );
    let mut j = Journal::default();
    let mut persist = |_: &Journal| Ok(());

    assert_eq!(engine::state(&b, &t), TweakState::NotApplied);
    engine::apply(&b, &t, &mut j, &mut persist).unwrap();
    assert_eq!(engine::state(&b, &t), TweakState::Applied);

    let catalog = customwin_core::Catalog {
        tweaks: vec![t.clone()],
        profiles: vec![],
    };
    engine::revert(&b, &catalog, &t.id, &mut j, &mut persist).unwrap();
    assert!(
        !b.reg_key_exists(Hive::HKCU, &root).unwrap(),
        "created keys are pruned"
    );
}

#[test]
fn reading_system_state_does_not_fail() {
    let b = WindowsBackend;
    // Values and probes vary per machine (and Wine stubs some); only check they answer.
    let _ = b.power_active_scheme();
    for p in [
        SysParam::ClientAreaAnimation,
        SysParam::WindowAnimation,
        SysParam::MouseAcceleration,
    ] {
        let _ = b.sysparam_get(p);
    }
    assert_eq!(b.service_query("CustomWinNoSuchService").unwrap(), None);
    let info = collect_system_info();
    assert!(info.cpu.logical_cores > 0);
}
