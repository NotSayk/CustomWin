//! Machine analysis: what we know about the PC, what looks improvable, and which catalog
//! tweaks fit the user's answers to the questionnaire.
//!
//! Everything here is pure (no system calls) so it can be unit-tested; the data is
//! collected by [`crate::windows::collect_system_info`].

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::engine::TweakState;
use crate::model::{Evidence, Localized, Risk, Tweak};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub memory_bytes: u64,
    pub gpus: Vec<GpuInfo>,
    pub power: PowerInfo,
    pub features: Features,
    pub network: Vec<NetAdapter>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OsInfo {
    pub name: String,
    pub display_version: String,
    pub build: u32,
    pub ubr: u32,
    pub edition: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub name: String,
    pub logical_cores: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    #[default]
    Other,
}

impl GpuVendor {
    pub fn from_pci_id(id: u32) -> Self {
        match id {
            0x10de => GpuVendor::Nvidia,
            0x1002 | 0x1022 => GpuVendor::Amd,
            0x8086 => GpuVendor::Intel,
            _ => GpuVendor::Other,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: GpuVendor,
    pub vram_bytes: u64,
    /// Windows driver version, e.g. `32.0.15.6094`.
    pub driver_version: Option<String>,
    /// Vendor-facing version, e.g. `560.94` for NVIDIA.
    pub driver_display_version: Option<String>,
    /// `YYYY-MM-DD`.
    pub driver_date: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerInfo {
    pub active_scheme: String,
    pub active_scheme_name: String,
    pub has_battery: bool,
    pub on_battery: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Features {
    /// `None` when Windows uses its default (on).
    pub game_mode: Option<bool>,
    /// `None` when not configured.
    pub hags: Option<bool>,
    pub memory_integrity: Option<bool>,
    pub background_recording: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetKind {
    Ethernet,
    Wifi,
    #[default]
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NetAdapter {
    pub name: String,
    pub description: String,
    pub kind: NetKind,
    pub connected: bool,
    pub link_speed_bps: u64,
}

/// Turns the Windows driver version of an NVIDIA driver into the version NVIDIA
/// publishes: the last five digits of the last two fields. `32.0.15.6094` → `560.94`.
pub fn nvidia_display_version(windows_version: &str) -> Option<String> {
    let parts: Vec<&str> = windows_version.split('.').collect();
    if parts.len() != 4
        || !parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let digits = format!("{}{:0>4}", parts[2], parts[3]);
    let last5 = &digits[digits.len().checked_sub(5)?..];
    Some(format!("{}.{}", &last5[..3], &last5[3..]))
}

/// Registry `DriverDate` is `M-D-YYYY`; returns `YYYY-MM-DD`.
pub fn normalize_driver_date(raw: &str) -> Option<String> {
    let p: Vec<u32> = raw
        .split('-')
        .map(|x| x.trim().parse().ok())
        .collect::<Option<_>>()?;
    match p[..] {
        [m @ 1..=12, d @ 1..=31, y @ 1990..=2100] => Some(format!("{y:04}-{m:02}-{d:02}")),
        _ => None,
    }
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` date (proleptic Gregorian).
pub fn days_from_iso(date: &str) -> Option<i64> {
    let p: Vec<i64> = date
        .split('-')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let [y, m, d] = p[..] else { return None };
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Suggestion,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Insight {
    pub id: String,
    pub severity: Severity,
    pub title: Localized,
    pub detail: Localized,
    /// Catalog tweak that fixes it, if any.
    pub tweak: Option<String>,
}

fn l(fr: impl Into<String>, en: impl Into<String>) -> Localized {
    Localized {
        fr: fr.into(),
        en: en.into(),
    }
}

const POWER_SAVER: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";

/// Observations about the machine that go beyond the catalog's on/off tweaks.
/// `today` is days since the Unix epoch.
pub fn insights(info: &SystemInfo, today: i64) -> Vec<Insight> {
    let mut out = Vec::new();

    for gpu in info.gpus.iter().filter(|g| g.vendor == GpuVendor::Nvidia) {
        let Some(age) = gpu
            .driver_date
            .as_deref()
            .and_then(days_from_iso)
            .map(|d| today - d)
        else {
            continue;
        };
        if age > 180 {
            let v = gpu.driver_display_version.clone().unwrap_or_default();
            out.push(Insight {
                id: "gpu.driver_old".into(),
                severity: Severity::Warning,
                title: l(format!("Pilote NVIDIA ancien ({v})"), format!("Old NVIDIA driver ({v})")),
                detail: l(
                    format!("Ton pilote pour {} a {} mois. Les nouveaux pilotes corrigent des bugs et améliorent les jeux récents : mets-le à jour via l'application NVIDIA.", gpu.name, age / 30),
                    format!("Your {} driver is {} months old. Newer drivers fix bugs and improve recent games: update it through the NVIDIA app.", gpu.name, age / 30),
                ),
                tweak: None,
            });
        }
    }

    if !info.power.has_battery && info.power.active_scheme.eq_ignore_ascii_case(POWER_SAVER) {
        out.push(Insight {
            id: "power.saver_on_desktop".into(),
            severity: Severity::Warning,
            title: l(
                "Mode économie d'énergie sur un PC fixe",
                "Power saver on a desktop PC",
            ),
            detail: l(
                "Ce plan bride le processeur alors que ton PC est branché en permanence.",
                "This plan throttles the CPU even though your PC is always plugged in.",
            ),
            tweak: Some("power.high_performance".into()),
        });
    }
    if info.power.on_battery {
        out.push(Insight {
            id: "power.on_battery".into(),
            severity: Severity::Info,
            title: l("PC sur batterie", "PC running on battery"),
            detail: l(
                "Les performances sont réduites sur batterie : branche le chargeur pour jouer.",
                "Performance is reduced on battery: plug in the charger to play.",
            ),
            tweak: None,
        });
    }

    for a in info
        .network
        .iter()
        .filter(|a| a.connected && a.kind == NetKind::Ethernet)
    {
        if a.link_speed_bps > 0 && a.link_speed_bps <= 100_000_000 {
            out.push(Insight {
                id: "network.ethernet_100m".into(),
                severity: Severity::Warning,
                title: l(format!("Ethernet limité à 100 Mb/s ({})", a.name), format!("Ethernet capped at 100 Mb/s ({})", a.name)),
                detail: l(
                    "La carte négocie 100 Mb/s au lieu de 1 Gb/s ou plus : c'est presque toujours un câble abîmé ou de catégorie trop ancienne (Cat5), ou un port de box limité.",
                    "The adapter negotiates 100 Mb/s instead of 1 Gb/s or more: almost always a damaged or too-old (Cat5) cable, or a limited router port.",
                ),
                tweak: None,
            });
        }
    }
    let wired = info
        .network
        .iter()
        .any(|a| a.connected && a.kind == NetKind::Ethernet);
    let wifi = info
        .network
        .iter()
        .any(|a| a.connected && a.kind == NetKind::Wifi);
    if wifi && !wired {
        out.push(Insight {
            id: "network.wifi_only".into(),
            severity: Severity::Info,
            title: l("Connecté en Wi-Fi", "Connected over Wi-Fi"),
            detail: l(
                "Pour jouer en ligne, un câble Ethernet donne une latence plus basse et plus stable que le Wi-Fi.",
                "For online gaming, an Ethernet cable gives lower and steadier latency than Wi-Fi.",
            ),
            tweak: None,
        });
    }

    if info.features.background_recording == Some(true) {
        out.push(Insight {
            id: "gaming.background_recording".into(),
            severity: Severity::Suggestion,
            title: l("Enregistrement en arrière-plan actif", "Background recording is on"),
            detail: l(
                "La Game Bar enregistre en permanence pendant que tu joues, ce qui occupe l'encodeur vidéo.",
                "The Game Bar records constantly while you play, which keeps the video encoder busy.",
            ),
            tweak: Some("gaming.game_dvr_off".into()),
        });
    }

    if info.memory_bytes > 0 && info.memory_bytes < 12 * (1 << 30) {
        out.push(Insight {
            id: "hardware.low_ram".into(),
            severity: Severity::Info,
            title: l("Peu de mémoire vive", "Low on RAM"),
            detail: l(
                format!("{} Go de RAM : les jeux récents en demandent 16. C'est l'amélioration matérielle la plus rentable.", info.memory_bytes >> 30),
                format!("{} GB of RAM: recent games ask for 16. It is the most cost-effective hardware upgrade.", info.memory_bytes >> 30),
            ),
            tweak: None,
        });
    }

    out.sort_by_key(|i| std::cmp::Reverse(i.severity));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Goal {
    Gaming,
    Performance,
    Privacy,
    Clean,
}

impl Goal {
    fn tag(self) -> &'static str {
        match self {
            Goal::Gaming => "gaming",
            Goal::Performance => "performance",
            Goal::Privacy => "privacy",
            Goal::Clean => "clean",
        }
    }
}

/// Answers to the questionnaire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answers {
    pub goals: Vec<Goal>,
    /// Let expert tweaks appear in the recommendations.
    #[serde(default)]
    pub include_expert: bool,
    /// Overrides battery detection.
    #[serde(default)]
    pub is_laptop: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recommendation {
    pub tweak_id: String,
    pub goals: Vec<Goal>,
}

/// Catalog tweaks that serve at least one of the user's goals and are not applied yet,
/// best first: proven effect, then lower risk.
pub fn recommend(
    catalog: &Catalog,
    answers: &Answers,
    info: &SystemInfo,
    state_of: impl Fn(&Tweak) -> TweakState,
) -> Vec<Recommendation> {
    let laptop = answers.is_laptop.unwrap_or(info.power.has_battery);
    let mut recs: Vec<(&Tweak, Recommendation)> = catalog
        .tweaks
        .iter()
        .filter(|t| answers.include_expert || t.risk != Risk::Expert)
        .filter(|t| !(laptop && t.has_tag("desktop-only")))
        .filter_map(|t| {
            let goals: Vec<Goal> = answers
                .goals
                .iter()
                .copied()
                .filter(|g| t.has_tag(g.tag()))
                .collect();
            if goals.is_empty() {
                return None;
            }
            match state_of(t) {
                TweakState::NotApplied | TweakState::Partial => Some((
                    t,
                    Recommendation {
                        tweak_id: t.id.clone(),
                        goals,
                    },
                )),
                TweakState::Applied | TweakState::Unavailable(_) => None,
            }
        })
        .collect();
    let evidence_rank = |e: Evidence| match e {
        Evidence::Measured => 0,
        Evidence::Situational => 1,
        Evidence::Functional => 2,
    };
    recs.sort_by_key(|(t, r)| {
        (
            std::cmp::Reverse(r.goals.len()),
            evidence_rank(t.evidence),
            t.risk,
        )
    });
    recs.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_versions() {
        assert_eq!(
            nvidia_display_version("32.0.15.6094").as_deref(),
            Some("560.94")
        );
        assert_eq!(
            nvidia_display_version("31.0.15.5222").as_deref(),
            Some("552.22")
        );
        assert_eq!(
            nvidia_display_version("32.0.15.7602").as_deref(),
            Some("576.02")
        );
        assert_eq!(
            nvidia_display_version("30.0.14.0620").as_deref(),
            Some("406.20")
        );
        assert_eq!(nvidia_display_version("garbage"), None);
    }

    #[test]
    fn dates() {
        assert_eq!(
            normalize_driver_date("7-9-2024").as_deref(),
            Some("2024-07-09")
        );
        assert_eq!(normalize_driver_date("13-1-2024"), None);
        assert_eq!(days_from_iso("1970-01-01"), Some(0));
        assert_eq!(days_from_iso("2000-03-01"), Some(11_017));
        assert_eq!(
            days_from_iso("2026-10-31").unwrap() - days_from_iso("2026-01-01").unwrap(),
            303
        );
    }

    fn machine() -> SystemInfo {
        SystemInfo {
            memory_bytes: 8 << 30,
            gpus: vec![GpuInfo {
                name: "NVIDIA GeForce RTX 3070".into(),
                vendor: GpuVendor::Nvidia,
                driver_display_version: Some("552.22".into()),
                driver_date: Some("2024-04-01".into()),
                ..Default::default()
            }],
            power: PowerInfo {
                active_scheme: POWER_SAVER.into(),
                ..Default::default()
            },
            network: vec![NetAdapter {
                name: "Ethernet".into(),
                kind: NetKind::Ethernet,
                connected: true,
                link_speed_bps: 100_000_000,
                ..Default::default()
            }],
            features: Features {
                background_recording: Some(true),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn insights_spot_common_problems() {
        let ids: Vec<String> = insights(&machine(), days_from_iso("2026-10-31").unwrap())
            .into_iter()
            .map(|i| i.id)
            .collect();
        for id in [
            "gpu.driver_old",
            "power.saver_on_desktop",
            "network.ethernet_100m",
            "gaming.background_recording",
            "hardware.low_ram",
        ] {
            assert!(ids.contains(&id.to_string()), "missing {id} in {ids:?}");
        }
        assert!(!ids.contains(&"network.wifi_only".to_string()));
        let fresh = insights(&machine(), days_from_iso("2024-05-01").unwrap());
        assert!(!fresh.iter().any(|i| i.id == "gpu.driver_old"));
    }

    #[test]
    fn insight_tweaks_exist_in_catalog() {
        let c = Catalog::embedded().unwrap();
        for i in insights(&machine(), 30_000) {
            if let Some(t) = &i.tweak {
                assert!(c.tweak(t).is_some(), "{t}");
            }
        }
    }

    #[test]
    fn recommendations_follow_goals_and_constraints() {
        let c = Catalog::embedded().unwrap();
        let mut info = SystemInfo::default();
        let answers = Answers {
            goals: vec![Goal::Gaming],
            ..Default::default()
        };

        let desktop = recommend(&c, &answers, &info, |_| TweakState::NotApplied);
        let ids: Vec<&str> = desktop.iter().map(|r| r.tweak_id.as_str()).collect();
        assert!(ids.contains(&"gaming.game_dvr_off"));
        assert!(ids.contains(&"power.high_performance"));
        assert!(!ids.contains(&"expert.vbs_off"), "expert tweaks are opt-in");
        assert!(
            !ids.contains(&"privacy.advertising_id"),
            "not a gaming tweak"
        );
        assert_eq!(
            ids[0], "gaming.game_dvr_off",
            "measured + 2 goals ranks first"
        );

        info.power.has_battery = true;
        let laptop = recommend(&c, &answers, &info, |_| TweakState::NotApplied);
        assert!(!laptop
            .iter()
            .any(|r| r.tweak_id == "power.high_performance"));

        let none = recommend(&c, &answers, &info, |_| TweakState::Applied);
        assert!(none.is_empty());

        let expert = Answers {
            include_expert: true,
            ..answers
        };
        assert!(recommend(&c, &expert, &info, |_| TweakState::NotApplied)
            .iter()
            .any(|r| r.tweak_id == "expert.vbs_off"));
    }
}
