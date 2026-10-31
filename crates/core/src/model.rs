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
    Appx(AppxAction),
    NetAdapter(NetAdapterAction),
    Nvidia(NvidiaAction),
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

/// Removes a preinstalled Store app for the current user. Undoing it reinstalls the
/// app from the Microsoft Store, which needs internet: only partially reversible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppxAction {
    /// Package name, e.g. `Microsoft.BingNews`.
    pub package: String,
    /// Microsoft Store product id used to reinstall it, e.g. `9WZDNCRFHVFW`.
    pub store_id: String,
}

/// Which network adapters a [`NetAdapterAction`] applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Ethernet,
    Wifi,
    All,
}

/// Sets a value in the driver key of every physical network adapter of a kind
/// (`HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e972-…}\NNNN`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetAdapterAction {
    pub adapters: AdapterKind,
    /// Value name, e.g. the standard advanced-property keyword `*EEE`.
    pub name: String,
    #[serde(flatten)]
    pub value: RegValue,
    /// Only touch adapters whose driver already exposes this value (advanced
    /// properties); otherwise every adapter of the kind gets it.
    #[serde(default)]
    pub only_if_present: bool,
}

/// A DWORD setting of the NVIDIA driver's global profile, through NVAPI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NvidiaAction {
    /// NVAPI setting id (`NvApiDriverSettings.h`).
    pub setting: u32,
    pub value: u32,
}

impl Action {
    /// Whether this action needs an elevated process.
    pub fn requires_admin(&self) -> bool {
        match self {
            Action::Registry(r) => r.hive == Hive::HKLM,
            Action::Service(_) | Action::NetAdapter(_) | Action::Nvidia(_) => true,
            Action::PowerPlan(_) | Action::SystemParameter(_) | Action::Appx(_) => false,
        }
    }

    /// Whether the action changes the signed-in user's own settings (as opposed to
    /// machine-wide ones). Matters when the elevated helper runs as another account.
    pub fn per_user(&self) -> bool {
        match self {
            Action::Registry(r) => r.hive == Hive::HKCU,
            Action::SystemParameter(_) | Action::Appx(_) => true,
            Action::Service(_)
            | Action::PowerPlan(_)
            | Action::NetAdapter(_)
            | Action::Nvidia(_) => false,
        }
    }

    /// Whether CustomWin can undo this action without help from outside the PC.
    pub fn fully_reversible(&self) -> bool {
        !matches!(self, Action::Appx(_))
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
    Apps,
    Network,
    Nvidia,
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

    pub fn fully_reversible(&self) -> bool {
        self.actions.iter().all(Action::fully_reversible)
    }

    pub fn per_user(&self) -> bool {
        self.actions.iter().any(Action::per_user)
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
