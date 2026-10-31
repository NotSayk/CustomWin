//! Data model of the tweak catalog: tweaks, the actions they perform, and profiles.
//!
//! Tweaks are declarative: an action only says *what* system setting should hold
//! *which* value. The engine reads the current state before changing anything, which is
//! what makes every tweak reversible.

use serde::{Deserialize, Serialize};

/// UI language. The catalog carries every string in both languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Fr,
    En,
}

impl Lang {
    /// `"fr-FR"`, `"fr"` → French; anything else → English.
    pub fn from_tag(tag: &str) -> Self {
        if tag.to_ascii_lowercase().starts_with("fr") {
            Lang::Fr
        } else {
            Lang::En
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Localized {
    pub fr: String,
    pub en: String,
}

impl Localized {
    pub fn get(&self, lang: Lang) -> &str {
        match lang {
            Lang::Fr => &self.fr,
            Lang::En => &self.en,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Hive {
    HKCU,
    HKLM,
}

/// A registry value. `{ dword = 0 }`, `{ qword = 1 }` or `{ string = "…" }` in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegValue {
    Dword(u32),
    Qword(u64),
    String(String),
    /// Any other registry type (binary, expandable string…), kept byte for byte so an
    /// original value can be restored exactly. Never used as a catalog target.
    Raw {
        reg_type: u32,
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartType {
    Boot,
    System,
    Automatic,
    Manual,
    Disabled,
}

/// Settings that must go through `SystemParametersInfo` to apply live
/// (writing their registry value alone only takes effect after signing out).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SysParam {
    /// "Animation effects" in Settings › Accessibility › Visual effects.
    ClientAreaAnimation,
    /// Minimize / maximize window animations.
    WindowAnimation,
    /// "Enhance pointer precision" (mouse acceleration).
    MouseAcceleration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Registry(RegistryAction),
    Service(ServiceAction),
    PowerPlan(PowerPlanAction),
    SystemParameter(SystemParameterAction),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAction {
    pub hive: Hive,
    /// Key path below the hive, with backslashes.
    pub path: String,
    /// Value name; empty string means the key's `(Default)` value.
    pub name: String,
    #[serde(flatten)]
    pub value: RegValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceAction {
    pub name: String,
    pub startup: StartType,
    /// Also stop the service right away (only meaningful with `disabled`).
    #[serde(default)]
    pub stop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerPlanAction {
    /// Power scheme GUID, lowercase, without braces.
    pub scheme: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemParameterAction {
    pub param: SysParam,
    pub value: bool,
}

impl Action {
    /// Whether this action needs an elevated process.
    pub fn requires_admin(&self) -> bool {
        match self {
            Action::Registry(r) => r.hive == Hive::HKLM,
            Action::Service(_) => true,
            Action::PowerPlan(_) | Action::SystemParameter(_) => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Privacy,
    Ads,
    Interface,
    Explorer,
    Gaming,
    Power,
    Updates,
    Expert,
}

/// How much can go wrong. `Expert` tweaks are hidden by default and never part of a
/// one-click profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Safe,
    Low,
    Medium,
    Expert,
}

/// What we actually know about the effect of a tweak. Lets the UI stay honest about
/// "optimizations" instead of promising FPS from placebo settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// Does exactly what it says (hides an ad, changes a menu). No performance claim.
    Functional,
    /// Measurable, reproducible performance or latency effect.
    Measured,
    /// Effect depends on the hardware, driver or game; can be neutral.
    Situational,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Restart {
    #[default]
    None,
    Explorer,
    SignOut,
    Reboot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tweak {
    pub id: String,
    pub category: Category,
    pub name: Localized,
    pub description: Localized,
    pub risk: Risk,
    pub evidence: Evidence,
    #[serde(default)]
    pub restart: Restart,
    /// Goals this tweak serves (`privacy`, `performance`, `gaming`, `clean`) and
    /// constraints (`desktop-only`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Warning shown next to the tweak.
    #[serde(default)]
    pub note: Option<Localized>,
    #[serde(rename = "action")]
    pub actions: Vec<Action>,
}

impl Tweak {
    pub fn requires_admin(&self) -> bool {
        self.actions.iter().any(Action::requires_admin)
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: Localized,
    pub description: Localized,
    pub tweaks: Vec<String>,
}
