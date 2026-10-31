//! Latest NVIDIA driver lookup (optional, online). This module only parses NVIDIA's
//! public responses and compares versions; the requests are made by
//! [`crate::windows::check_nvidia_drivers`].
//!
//! 1. `lookupValueSearch.aspx?TypeID=3` lists every GeForce product with its series
//!    (`psid`) and product (`pfid`) ids.
//! 2. `AjaxDriverService.php?func=DriverManualLookup` returns the newest driver for one
//!    product.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

pub const LOOKUP_HOST: &str = "www.nvidia.com";
pub const LOOKUP_PATH: &str = "/Download/API/lookupValueSearch.aspx?TypeID=3";
pub const DRIVER_HOST: &str = "gfwsl.geforce.com";

/// Windows 11 64-bit, Game Ready (WHQL, DCH) drivers.
pub fn driver_path(psid: u32, pfid: u32) -> String {
    format!(
        "/services_toolkit/services/com/nvidia/services/AjaxDriverService.php?func=DriverManualLookup\
         &psid={psid}&pfid={pfid}&osID=135&languageCode=1033&isWHQL=1&dch=1&sort1=0&numberOfResults=1"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    pub name: String,
    pub psid: u32,
    pub pfid: u32,
}

/// Parses the product list. A tiny scanner is enough for this flat, stable format.
pub fn parse_products(xml: &str) -> Vec<Product> {
    let mut out = Vec::new();
    for chunk in xml.split("<LookupValue ").skip(1) {
        let attr = chunk
            .split("ParentID=\"")
            .nth(1)
            .and_then(|r| r.split('"').next());
        let tag = |t: &str| {
            chunk
                .split(&format!("<{t}>"))
                .nth(1)
                .and_then(|r| r.split(&format!("</{t}>")).next())
        };
        if let (Some(psid), Some(name), Some(pfid)) = (attr, tag("Name"), tag("Value")) {
            if let (Ok(psid), Ok(pfid)) = (psid.parse(), pfid.parse()) {
                out.push(Product {
                    name: xml_unescape(name.trim()),
                    psid,
                    pfid,
                });
            }
        }
    }
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// `NVIDIA GeForce RTX 4070` and `GeForce RTX 4070` are the same card.
fn normalize(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    lower
        .strip_prefix("nvidia ")
        .unwrap_or(&lower)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Finds the product matching a GPU name as Windows reports it.
pub fn find_product<'a>(products: &'a [Product], gpu_name: &str) -> Option<&'a Product> {
    let wanted = normalize(gpu_name);
    products.iter().find(|p| normalize(&p.name) == wanted)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestDriver {
    pub version: String,
    /// `YYYY-MM-DD`.
    pub release_date: Option<String>,
    pub details_url: String,
}

#[derive(Deserialize)]
struct LookupResponse {
    #[serde(rename = "IDS", default)]
    ids: Vec<LookupId>,
}

#[derive(Deserialize)]
struct LookupId {
    #[serde(rename = "downloadInfo")]
    info: DownloadInfo,
}

#[derive(Deserialize)]
struct DownloadInfo {
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "ReleaseDateTime", default)]
    release: String,
    #[serde(rename = "DetailsURL", default)]
    details_url: String,
}

pub fn parse_latest(json: &str) -> Option<LatestDriver> {
    let r: LookupResponse = serde_json::from_str(json).ok()?;
    let info = r.ids.into_iter().next()?.info;
    // Only link to NVIDIA's own site.
    let details_url = if info.details_url.starts_with("https://www.nvidia.com/") {
        info.details_url
    } else {
        String::new()
    };
    Some(LatestDriver {
        version: info.version,
        release_date: parse_release_date(&info.release),
        details_url,
    })
}

/// `Wed Oct 21, 2026` → `2026-10-21`.
fn parse_release_date(s: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let parts: Vec<&str> = s.split([' ', ',']).filter(|p| !p.is_empty()).collect();
    let [_, month, day, year] = parts[..] else {
        return None;
    };
    let m = MONTHS.iter().position(|x| *x == month)? + 1;
    let d: u32 = day.parse().ok()?;
    let y: u32 = year.parse().ok()?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Compares dotted numeric versions: `560.94` < `621.01`.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |v: &str| -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a).cmp(&parse(b))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverCheck {
    pub gpu: String,
    pub installed: Option<String>,
    pub latest: LatestDriver,
    pub update_available: bool,
}

pub fn check(gpu: &str, installed: Option<&str>, latest: LatestDriver) -> DriverCheck {
    DriverCheck {
        gpu: gpu.to_string(),
        installed: installed.map(str::to_string),
        update_available: installed
            .is_none_or(|i| compare_versions(i, &latest.version) == Ordering::Less),
        latest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Excerpts of real responses (October 2026).
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?><LookupValueSearch><LookupValues><LookupValue ParentID="120"><Name>GeForce RTX 3060</Name><Value>942</Value></LookupValue><LookupValue ParentID="127"><Name>NVIDIA GeForce RTX 4070 Ti</Name><Value>1001</Value></LookupValue><LookupValue ParentID="127"><Name>NVIDIA GeForce RTX 4070</Name><Value>1015</Value></LookupValue><LookupValue ParentID="129"><Name>GeForce RTX 4070 Laptop GPU</Name><Value>1006</Value></LookupValue></LookupValues></LookupValueSearch>"#;
    const JSON: &str = r#"{ "Success" : "1", "IDS" : [{ "downloadInfo": { "Success" : "1", "ID" : "282412", "Name" : "GeForce%20Game%20Ready%20Driver", "Release" : "620", "Version" : "621.01", "IsBeta" : "0", "IsWHQL" : "1", "ReleaseDateTime" : "Wed Oct 21, 2026", "DetailsURL" : "https://www.nvidia.com/en-us/drivers/details/282412/", "DownloadURL" : "https://us.download.nvidia.com/Windows/621.01/621.01-desktop-win10-win11-64bit-international-dch-whql.exe" } }] }"#;

    #[test]
    fn finds_products_by_windows_name() {
        let p = parse_products(XML);
        assert_eq!(p.len(), 4);
        let rtx4070 = find_product(&p, "NVIDIA GeForce RTX 4070").unwrap();
        assert_eq!((rtx4070.psid, rtx4070.pfid), (127, 1015), "not the Ti");
        assert_eq!(
            find_product(&p, "NVIDIA GeForce RTX 3060").unwrap().pfid,
            942
        );
        assert_eq!(
            find_product(&p, "NVIDIA GeForce RTX 4070 Laptop GPU")
                .unwrap()
                .pfid,
            1006
        );
        assert!(find_product(&p, "AMD Radeon RX 7800 XT").is_none());
    }

    #[test]
    fn parses_latest_driver() {
        let d = parse_latest(JSON).unwrap();
        assert_eq!(d.version, "621.01");
        assert_eq!(d.release_date.as_deref(), Some("2026-10-21"));
        assert_eq!(
            d.details_url,
            "https://www.nvidia.com/en-us/drivers/details/282412/"
        );
        assert!(parse_latest(r#"{"Success":"0"}"#).is_none());
        let evil = JSON.replace("https://www.nvidia.com/", "https://evil.example/");
        assert_eq!(parse_latest(&evil).unwrap().details_url, "");
    }

    #[test]
    fn version_comparison() {
        assert_eq!(compare_versions("560.94", "621.01"), Ordering::Less);
        assert_eq!(compare_versions("621.01", "621.01"), Ordering::Equal);
        assert_eq!(compare_versions("1000.1", "999.99"), Ordering::Greater);
        let latest = parse_latest(JSON).unwrap();
        assert!(check("x", Some("560.94"), latest.clone()).update_available);
        assert!(!check("x", Some("621.01"), latest).update_available);
    }
}
