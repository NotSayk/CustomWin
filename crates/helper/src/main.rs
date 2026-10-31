//! `customwin-helper` — the only part of CustomWin that runs with administrator rights.
//!
//! The UI starts it through UAC with `--request <file>`; it runs only catalog tweaks
//! (never arbitrary commands), and writes `<file>.response.json` back. It also works as a
//! small command-line tool for power users and scripts:
//!
//! ```text
//! customwin-helper list
//! customwin-helper status
//! customwin-helper catalog-json
//! customwin-helper apply <id>... | profile <id> | revert <id>... | revert-all
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use customwin_core::engine::touches_user_settings;
use customwin_core::journal::Journal;
use customwin_core::{Catalog, HelperRequest, HelperResponse, Operation};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let catalog = match Catalog::embedded() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match args.first().map(String::as_str) {
        Some("--request") if args.len() == 2 => serve_request(&catalog, Path::new(&args[1])),
        Some("list") => {
            for t in &catalog.tweaks {
                let admin = if t.requires_admin() { " [admin]" } else { "" };
                println!("{:<36} {:?}{admin}  {}", t.id, t.risk, t.name.en);
            }
            ExitCode::SUCCESS
        }
        Some("status") => status(&catalog),
        Some("catalog-json") => {
            println!(
                "{}",
                serde_json::to_string_pretty(&catalog.view()).expect("catalog serializes")
            );
            ExitCode::SUCCESS
        }
        Some("apply") => cli(&catalog, Operation::Apply(args[1..].to_vec())),
        Some("revert") => cli(&catalog, Operation::Revert(args[1..].to_vec())),
        Some("revert-all") => cli(&catalog, Operation::RevertAll),
        Some("profile") if args.len() == 2 => match catalog.profile(&args[1]) {
            Some(p) => cli(&catalog, Operation::Apply(p.tweaks.clone())),
            None => {
                eprintln!("unknown profile `{}`", args[1]);
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("usage: customwin-helper list | status | apply <id>... | profile <id> | revert <id>... | revert-all");
            ExitCode::from(2)
        }
    }
}

/// Response path for a request file: `x.json` → `x.json.response.json`.
fn response_path(request: &Path) -> PathBuf {
    let mut p = request.as_os_str().to_owned();
    p.push(".response.json");
    PathBuf::from(p)
}

fn serve_request(catalog: &Catalog, request_path: &Path) -> ExitCode {
    let response = match std::fs::read(request_path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice::<HelperRequest>(&b).map_err(|e| e.to_string()))
    {
        Ok(req) => handle(catalog, &req),
        Err(e) => HelperResponse {
            error: Some(format!("bad request: {e}")),
            ..Default::default()
        },
    };
    let ok = response.error.is_none();
    match serde_json::to_vec(&response)
        .map(|json| std::fs::write(response_path(request_path), json))
    {
        Ok(Ok(())) if ok => ExitCode::SUCCESS,
        Ok(Ok(())) => ExitCode::from(1),
        _ => ExitCode::from(3),
    }
}

fn handle(catalog: &Catalog, req: &HelperRequest) -> HelperResponse {
    // The journal path comes from an unelevated process: only accept CustomWin's own
    // journal file name. Its content is validated against the catalog before replay.
    let is_journal = req
        .journal_path
        .file_name()
        .is_some_and(|n| n == "journal.json")
        && req
            .journal_path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "CustomWin");
    if !is_journal {
        return HelperResponse {
            error: Some("refused journal path".into()),
            ..Default::default()
        };
    }
    // Elevated with another administrator's credentials, this process sees that
    // account's HKCU: per-user tweaks would land on the wrong account.
    if let (Some(requester), Some(me)) = (&req.user_sid, platform::current_sid()) {
        let journal = Journal::load(&req.journal_path).unwrap_or_default();
        if *requester != me && touches_user_settings(catalog, &journal, &req.operation) {
            return HelperResponse {
                error: Some(
                    "elevated as a different account: per-user settings would go to that \
                     account. Run CustomWin from an administrator account."
                        .into(),
                ),
                ..Default::default()
            };
        }
    }
    platform::run(
        catalog,
        &req.operation,
        &req.journal_path,
        req.restore_point,
    )
}

fn cli(catalog: &Catalog, op: Operation) -> ExitCode {
    let response = platform::run(catalog, &op, &platform::journal_path(), false);
    for w in &response.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(e) = &response.error {
        eprintln!("error: {e}");
        return ExitCode::from(1);
    }
    let mut failed = false;
    for r in &response.results {
        match &r.error {
            None => println!("ok      {}", r.tweak_id),
            Some(e) => {
                failed = true;
                println!("FAILED  {}: {e}", r.tweak_id);
            }
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn status(catalog: &Catalog) -> ExitCode {
    for (id, state) in platform::states(catalog) {
        println!("{id:<36} {state}");
    }
    ExitCode::SUCCESS
}

#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use customwin_core::engine::{self, requires_admin};
    use customwin_core::journal::Journal;
    use customwin_core::windows::{create_restore_point, is_elevated, WindowsBackend};
    use customwin_core::{Catalog, HelperResponse, Operation};

    pub fn journal_path() -> PathBuf {
        customwin_core::windows::journal_path()
    }

    pub fn current_sid() -> Option<String> {
        customwin_core::windows::current_user_sid()
    }

    pub fn run(
        catalog: &Catalog,
        op: &Operation,
        journal: &Path,
        restore_point: bool,
    ) -> HelperResponse {
        let mut response = HelperResponse::default();
        let needs_admin = Journal::load(journal).map_or(true, |j| requires_admin(catalog, &j, op));
        if needs_admin && !is_elevated() {
            response.error =
                Some("administrator rights required: run from an elevated terminal".into());
            return response;
        }
        if restore_point && matches!(op, Operation::Apply(_)) {
            if let Err(e) = create_restore_point("CustomWin") {
                response.warnings.push(format!("no restore point: {e}"));
            }
        }
        match engine::run(&WindowsBackend, catalog, journal, op) {
            Ok(results) => response.results = results,
            Err(e) => response.error = Some(e.to_string()),
        }
        response
    }

    pub fn states(catalog: &Catalog) -> Vec<(String, String)> {
        catalog
            .tweaks
            .iter()
            .map(|t| {
                (
                    t.id.clone(),
                    format!("{:?}", engine::state(&WindowsBackend, t)),
                )
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod platform {
    //! Lets the workspace build and test on other OSes; nothing here touches the system.
    use std::path::{Path, PathBuf};

    use customwin_core::{Catalog, HelperResponse, Operation};

    pub fn journal_path() -> PathBuf {
        std::env::temp_dir().join("CustomWin").join("journal.json")
    }

    pub fn current_sid() -> Option<String> {
        None
    }

    pub fn run(_: &Catalog, _: &Operation, _: &Path, _: bool) -> HelperResponse {
        HelperResponse {
            error: Some("CustomWin only runs on Windows".into()),
            ..Default::default()
        }
    }

    pub fn states(_: &Catalog) -> Vec<(String, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_path_appends_suffix() {
        assert_eq!(
            response_path(Path::new(r"C:\t\req.json")),
            PathBuf::from(r"C:\t\req.json.response.json")
        );
    }

    #[cfg(windows)]
    #[test]
    fn per_user_tweaks_are_refused_for_another_account() {
        let c = Catalog::embedded().unwrap();
        let dir = std::env::temp_dir()
            .join(format!("sid-{}", std::process::id()))
            .join("CustomWin");
        let req = |ids: &[&str]| HelperRequest {
            operation: Operation::Apply(ids.iter().map(|s| s.to_string()).collect()),
            journal_path: dir.join("journal.json"),
            restore_point: false,
            user_sid: Some("S-1-5-21-1-2-3-9999".into()),
        };
        let refused = handle(&c, &req(&["ads.lock_screen"]));
        assert!(refused.error.unwrap().contains("different account"));
        // Machine-wide only: the SID check lets it through (it may still fail later).
        let machine = handle(&c, &req(&["privacy.diagtrack_service"]));
        assert!(!machine
            .error
            .unwrap_or_default()
            .contains("different account"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn foreign_journal_paths_are_refused() {
        let c = Catalog::embedded().unwrap();
        let req = |p: &str| HelperRequest {
            operation: Operation::RevertAll,
            journal_path: p.into(),
            restore_point: false,
            user_sid: None,
        };
        for bad in [
            "/etc/passwd",
            "/x/CustomWin/other.json",
            "/x/Other/journal.json",
        ] {
            assert_eq!(
                handle(&c, &req(bad)).error.as_deref(),
                Some("refused journal path"),
                "{bad}"
            );
        }
    }
}
