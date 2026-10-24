use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const REFRESH_SECS: i64 = 24 * 60 * 60;
/// Upper bound on hosts kept from third-party catalogs.
const MAX_EXTRACTED_HOSTS: usize = 64;
/// Bilibili-owned media domains. Shared CDN domains such as `akamaized.net`
/// are trusted only for the built-in hosts, because anyone can own a subdomain.
const TRUSTED_SUFFIXES: [&str; 2] = [".bilivideo.com", ".bilibilivideo.com"];
const SOURCES: [&str; 2] = [
    "https://gist.githubusercontent.com/maguowei/20a39ed03b43b87704c47c517064e3d7/raw/bilibili.snd.sgmodule",
    "https://raw.githubusercontent.com/BiliUniverse/Redirect/main/src/function/database.mjs",
];
// PiliPlus keeps this list as its user-selectable Bilibili CDN catalog.  Keep
// the complete set locally so ranking is not dependent on a third-party
// catalog fetch succeeding.
const PILIPLUS_DOMESTIC: [&str; 14] = [
    "upos-sz-mirrorali.bilivideo.com",
    "upos-sz-mirroralib.bilivideo.com",
    "upos-sz-mirroralio1.bilivideo.com",
    "upos-sz-mirrorcos.bilivideo.com",
    "upos-sz-mirrorcosb.bilivideo.com",
    "upos-sz-mirrorcoso1.bilivideo.com",
    "upos-sz-mirrorhw.bilivideo.com",
    "upos-sz-mirrorhwb.bilivideo.com",
    "upos-sz-mirrorhwo1.bilivideo.com",
    "upos-sz-mirror08c.bilivideo.com",
    "upos-sz-mirror08h.bilivideo.com",
    "upos-sz-mirror08ct.bilivideo.com",
    "upos-tf-all-hw.bilivideo.com",
    "upos-tf-all-tx.bilivideo.com",
];
const PILIPLUS_OVERSEAS: [&str; 5] = [
    "upos-hz-mirrorakam.akamaized.net",
    "upos-sz-mirroraliov.bilivideo.com",
    "upos-sz-mirrorcosov.bilivideo.com",
    "upos-sz-mirrorhwov.bilivideo.com",
    "cn-hk-eq-bcache-01.bilivideo.com",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Region {
    MainlandChina,
    Overseas,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct CatalogCache {
    updated_at: i64,
    region_updated_at: i64,
    region: Option<Region>,
    hosts: Vec<String>,
}

fn cache_path() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push("bilibili-tui");
    path.push("cdn-catalog.json");
    Some(path)
}

fn load_cache() -> CatalogCache {
    cache_path()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &CatalogCache) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(cache) {
        let temporary = path.with_extension("json.tmp");
        if fs::write(&temporary, bytes).is_ok() {
            let _ = fs::rename(temporary, path);
        }
    }
}

fn extract_hosts(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || character == '.' || character == '-')
    })
    .filter(|token| {
        (token.starts_with("upos-") || token.starts_with("proxy-"))
            && ["bilivideo.com", "bilibilivideo.com", "akamaized.net"]
                .iter()
                .any(|suffix| {
                    token
                        .strip_suffix(suffix)
                        .is_some_and(|prefix| prefix.ends_with('.'))
                })
    })
    .map(str::to_ascii_lowercase)
}

/// Whether a host may receive a signed media URL rewritten from another edge.
pub(super) fn is_trusted_media_host(host: &str) -> bool {
    PILIPLUS_DOMESTIC.contains(&host)
        || PILIPLUS_OVERSEAS.contains(&host)
        || TRUSTED_SUFFIXES.iter().any(|suffix| {
            host.strip_suffix(suffix)
                .is_some_and(|prefix| !prefix.is_empty())
        })
}

/// Built-in hosts plus a capped set of trusted hosts from the latest catalog.
fn merge_catalog(extracted: impl IntoIterator<Item = String>) -> Vec<String> {
    let extracted: BTreeSet<String> = extracted
        .into_iter()
        .filter(|host| is_trusted_media_host(host))
        .take(MAX_EXTRACTED_HOSTS)
        .collect();
    let mut hosts: BTreeSet<String> = PILIPLUS_DOMESTIC
        .into_iter()
        .chain(PILIPLUS_OVERSEAS)
        .map(str::to_string)
        .collect();
    hosts.extend(extracted);
    hosts.into_iter().collect()
}

pub(super) fn is_overseas_host(host: &str) -> bool {
    PILIPLUS_OVERSEAS.contains(&host)
}

async fn detect_region(client: &Client) -> Option<Region> {
    let value: serde_json::Value = tokio::time::timeout(Duration::from_secs(5), async {
        client
            .get("https://api.bilibili.com/x/web-interface/zone?jsonp=jsonp")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    })
    .await
    .ok()?
    .ok()?;
    let country_code = value.pointer("/data/country_code")?.as_i64()?;
    Some(if country_code == 86 {
        Region::MainlandChina
    } else {
        Region::Overseas
    })
}

async fn fetch_catalog(client: &Client, source: &str) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut response = client.get(source).send().await?.error_for_status()?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len().saturating_add(chunk.len()) > 1024 * 1024 {
                return Ok::<_, reqwest::Error>(None);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8(body).ok())
    })
    .await
    .ok()?
    .ok()?
}

#[allow(dead_code)]
pub(super) async fn regional_hosts(client: &Client) -> (Option<Region>, Vec<String>) {
    let now = chrono::Utc::now().timestamp();
    let mut cache = load_cache();
    if now - cache.updated_at >= REFRESH_SECS || cache.hosts.is_empty() {
        let mut extracted = Vec::new();
        let mut fetched = false;
        for source in SOURCES {
            if let Some(text) = fetch_catalog(client, source).await {
                fetched = true;
                extracted.extend(extract_hosts(&text));
            }
        }
        // Replace rather than accumulate, so hosts dropped upstream are pruned.
        // Keep the previous catalog when every source is unreachable.
        if fetched {
            cache.hosts = extracted;
        }
        cache.updated_at = now;
    }
    cache.hosts = merge_catalog(std::mem::take(&mut cache.hosts));
    if (now - cache.region_updated_at >= REFRESH_SECS || cache.region.is_none())
        && let Some(region) = detect_region(client).await
    {
        cache.region = Some(region);
        cache.region_updated_at = now;
    }
    let region = cache.region;
    save_cache(&cache);
    // Return the complete catalog. The caller ranks same-region hosts higher,
    // while retaining cross-region nodes as useful failure fallbacks.
    (region, cache.hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_classifies_cdn_hosts() {
        let text =
            "Host.A=upos-sz-mirrorali.bilivideo.com, https://upos-hz-mirrorakam.akamaized.net/path";
        let hosts = extract_hosts(text).collect::<Vec<_>>();
        assert_eq!(hosts.len(), 2);
        assert!(!is_overseas_host(&hosts[0]));
        assert!(is_overseas_host(&hosts[1]));
        assert!(is_overseas_host("upos-sz-mirrorhwov.bilivideo.com"));
        assert!(is_overseas_host("cn-hk-eq-bcache-01.bilivideo.com"));
        assert_eq!(extract_hosts("proxy-evilakamaized.net").count(), 0);
    }

    #[test]
    fn third_party_catalog_is_trusted_and_capped() {
        let extracted = (0..200)
            .map(|index| format!("upos-x{index:03}.bilivideo.com"))
            .chain(["upos-evil.akamaized.net".to_string()]);
        let hosts = merge_catalog(extracted);
        let builtin = PILIPLUS_DOMESTIC.len() + PILIPLUS_OVERSEAS.len();
        assert_eq!(hosts.len(), builtin + MAX_EXTRACTED_HOSTS);
        assert!(!hosts.iter().any(|host| host == "upos-evil.akamaized.net"));
        assert!(
            hosts
                .iter()
                .any(|host| host == "upos-hz-mirrorakam.akamaized.net")
        );
        assert!(!is_trusted_media_host("bilivideo.com"));
        assert!(!is_trusted_media_host("evilbilivideo.com"));
    }

    #[test]
    fn missing_region_remains_neutral() {
        let cache = CatalogCache::default();
        assert_eq!(cache.region, None);
    }
}
