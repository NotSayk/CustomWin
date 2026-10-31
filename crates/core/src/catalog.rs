//! The tweak catalog, embedded in the binary at compile time.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::{Action, Profile, Risk, Tweak};
use crate::Error;

const SOURCES: &[(&str, &str)] = &[
    ("privacy.toml", include_str!("../catalog/privacy.toml")),
    ("ads.toml", include_str!("../catalog/ads.toml")),
    ("interface.toml", include_str!("../catalog/interface.toml")),
    ("gaming.toml", include_str!("../catalog/gaming.toml")),
    ("hardware.toml", include_str!("../catalog/hardware.toml")),
    ("apps.toml", include_str!("../catalog/apps.toml")),
    ("expert.toml", include_str!("../catalog/expert.toml")),
    ("profiles.toml", include_str!("../catalog/profiles.toml")),
];

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    tweak: Vec<Tweak>,
    #[serde(default)]
    profile: Vec<Profile>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub tweaks: Vec<Tweak>,
    pub profiles: Vec<Profile>,
}

impl Catalog {
    /// Parses and validates the catalog embedded in the binary.
    pub fn embedded() -> Result<Self, Error> {
        Self::from_sources(SOURCES)
    }

    pub fn from_sources(sources: &[(&str, &str)]) -> Result<Self, Error> {
        let mut tweaks = Vec::new();
        let mut profiles = Vec::new();
        for (file, text) in sources {
            let parsed: File =
                toml::from_str(text).map_err(|e| Error::Catalog(format!("{file}: {e}")))?;
            tweaks.extend(parsed.tweak);
            profiles.extend(parsed.profile);
        }
        let catalog = Catalog { tweaks, profiles };
        catalog.validate()?;
        Ok(catalog)
    }

    /// What the UI receives: the catalog plus derived fields.
    pub fn view(&self) -> CatalogView<'_> {
        CatalogView {
            tweaks: self
                .tweaks
                .iter()
                .map(|t| TweakView {
                    tweak: t,
                    requires_admin: t.requires_admin(),
                    fully_reversible: t.fully_reversible(),
                })
                .collect(),
            profiles: &self.profiles,
        }
    }

    pub fn tweak(&self, id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|t| t.id == id)
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    fn validate(&self) -> Result<(), Error> {
        let bad = |msg: String| Err(Error::Catalog(msg));
        let mut ids = HashSet::new();
        for t in &self.tweaks {
            if !ids.insert(t.id.as_str()) {
                return bad(format!("duplicate tweak id `{}`", t.id));
            }
            if t.actions.is_empty() {
                return bad(format!("tweak `{}` has no action", t.id));
            }
            for a in &t.actions {
                match a {
                    Action::Registry(r) => {
                        if r.path.is_empty() || r.path.starts_with('\\') || r.path.ends_with('\\') {
                            return bad(format!(
                                "tweak `{}`: malformed registry path `{}`",
                                t.id, r.path
                            ));
                        }
                        if matches!(r.value, crate::model::RegValue::Raw { .. }) {
                            return bad(format!(
                                "tweak `{}`: raw values are not allowed as targets",
                                t.id
                            ));
                        }
                    }
                    Action::Service(s) if s.name.is_empty() => {
                        return bad(format!("tweak `{}`: empty service name", t.id));
                    }
                    Action::PowerPlan(p) if !is_guid(&p.scheme) => {
                        return bad(format!(
                            "tweak `{}`: `{}` is not a lowercase GUID",
                            t.id, p.scheme
                        ));
                    }
                    // These strings end up in Store and PowerShell calls: keep them to a
                    // strict alphabet so they can never be read as anything else.
                    Action::Appx(a)
                        if !is_package_name(&a.package) || !is_store_id(&a.store_id) =>
                    {
                        return bad(format!(
                            "tweak `{}`: malformed app package or store id",
                            t.id
                        ));
                    }
                    Action::NetAdapter(n)
                        if n.name.is_empty()
                            || n.name.contains('\\')
                            || matches!(n.value, crate::model::RegValue::Raw { .. }) =>
                    {
                        return bad(format!("tweak `{}`: malformed adapter value", t.id));
                    }
                    _ => {}
                }
            }
        }
        let mut profile_ids = HashSet::new();
        for p in &self.profiles {
            if !profile_ids.insert(p.id.as_str()) {
                return bad(format!("duplicate profile id `{}`", p.id));
            }
            for id in &p.tweaks {
                match self.tweak(id) {
                    None => return bad(format!("profile `{}` lists unknown tweak `{id}`", p.id)),
                    Some(t) if !t.fully_reversible() => {
                        return bad(format!(
                            "profile `{}` must not contain partially reversible tweak `{id}`",
                            p.id
                        ))
                    }
                    Some(t) if t.risk == Risk::Expert => {
                        return bad(format!(
                            "profile `{}` must not contain expert tweak `{id}`",
                            p.id
                        ))
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct TweakView<'a> {
    #[serde(flatten)]
    pub tweak: &'a Tweak,
    pub requires_admin: bool,
    pub fully_reversible: bool,
}

#[derive(Debug, Serialize)]
pub struct CatalogView<'a> {
    pub tweaks: Vec<TweakView<'a>>,
    pub profiles: &'a [Profile],
}

/// `Microsoft.BingNews`, `Clipchamp.Clipchamp`, `MSTeams`.
pub(crate) fn is_package_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

/// `9WZDNCRFHVFW`, `XP8BT8DW290MPQ`.
pub(crate) fn is_store_id(s: &str) -> bool {
    (12..=14).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// `8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c`
pub(crate) fn is_guid(s: &str) -> bool {
    let groups: Vec<&str> = s.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, len)| {
            g.len() == len
                && g.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_valid() {
        let c = Catalog::embedded().expect("catalog must parse and validate");
        assert!(c.tweaks.len() >= 25, "only {} tweaks", c.tweaks.len());
        for id in ["minimal", "gaming", "privacy"] {
            assert!(c.profile(id).is_some(), "missing profile {id}");
        }
    }

    #[test]
    fn every_string_is_translated() {
        let c = Catalog::embedded().unwrap();
        for t in &c.tweaks {
            for l in [Some(&t.name), Some(&t.description), t.note.as_ref()]
                .into_iter()
                .flatten()
            {
                assert!(
                    !l.fr.trim().is_empty() && !l.en.trim().is_empty(),
                    "{}",
                    t.id
                );
            }
            assert!(
                t.id.starts_with(&format!("{}.", prefix(t))),
                "id `{}` does not match its category",
                t.id
            );
        }
    }

    fn prefix(t: &Tweak) -> &'static str {
        use crate::model::Category::*;
        match t.category {
            Privacy => "privacy",
            Ads => "ads",
            Interface => "interface",
            Explorer => "explorer",
            Gaming => "gaming",
            Power => "power",
            Updates => "updates",
            Apps => "apps",
            Network => "network",
            Nvidia => "nvidia",
            Expert => "expert",
        }
    }

    #[test]
    fn registry_value_syntax() {
        let c = Catalog::embedded().unwrap();
        let menu = c.tweak("interface.classic_context_menu").unwrap();
        match &menu.actions[0] {
            Action::Registry(r) => {
                assert_eq!(r.name, "");
                assert_eq!(r.value, crate::model::RegValue::String(String::new()));
            }
            other => panic!("unexpected action {other:?}"),
        }
        assert!(c
            .tweak("privacy.diagtrack_service")
            .unwrap()
            .requires_admin());
        assert!(!c.tweak("ads.lock_screen").unwrap().requires_admin());
    }

    #[test]
    fn rejects_expert_tweak_in_profile() {
        let src = r#"
            [[tweak]]
            id = "expert.x"
            category = "expert"
            risk = "expert"
            evidence = "situational"
            name = { fr = "x", en = "x" }
            description = { fr = "x", en = "x" }
            [[tweak.action]]
            kind = "power_plan"
            scheme = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c"

            [[profile]]
            id = "p"
            name = { fr = "p", en = "p" }
            description = { fr = "p", en = "p" }
            tweaks = ["expert.x"]
        "#;
        assert!(Catalog::from_sources(&[("t.toml", src)]).is_err());
    }

    #[test]
    fn package_and_store_id_checks() {
        assert!(is_package_name("Microsoft.BingNews"));
        assert!(!is_package_name("Microsoft.Bing'; rm"));
        assert!(is_package_name("MSTeams"));
        assert!(!is_package_name("") && !is_package_name(".hidden"));
        assert!(is_store_id("9WZDNCRFHVFW") && is_store_id("XP8BT8DW290MPQ"));
        assert!(!is_store_id("9wzdncrfhvfw") && !is_store_id("9WZ"));
    }

    #[test]
    fn guid_check() {
        assert!(is_guid("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c"));
        assert!(!is_guid("{8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c}"));
        assert!(!is_guid("8C5E7FDA-E8BF-4A96-9A85-A6E23A8C635C"));
    }
}
