use anyhow::{Result, anyhow};
use futures_util::{StreamExt, stream};
use reqwest::header::{RANGE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

mod catalog;

const PROBE_BYTES: u64 = 512 * 1024 - 1;
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const USER_AGENT_VALUE: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36";
const SCORE_TTL: Duration = Duration::from_secs(15 * 60);
/// Catalog hosts probed per stream in addition to the ones the API returned.
const MAX_CATALOG_PROBES: usize = 6;
/// Concurrent media probes per stream kind.
const PROBE_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Deserialize)]
pub struct PlayUrlData {
    pub dash: Dash,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dash {
    pub video: Vec<DashStream>,
    #[serde(default)]
    pub audio: Vec<DashStream>,
    pub dolby: Option<Dolby>,
    pub flac: Option<Flac>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dolby {
    #[serde(default)]
    pub audio: Option<Vec<DashStream>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Flac {
    pub audio: Option<DashStream>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DashStream {
    pub id: i64,
    #[serde(default)]
    pub bandwidth: i64,
    #[serde(default)]
    pub codecid: i64,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default, rename = "baseUrl")]
    pub base_url_camel: Option<String>,
    #[serde(default)]
    pub backup_url: Option<Vec<String>>,
    #[serde(default, rename = "backupUrl")]
    pub backup_url_camel: Option<Vec<String>>,
}

impl DashStream {
    fn primary_url(&self) -> Option<&str> {
        self.base_url.as_deref().or(self.base_url_camel.as_deref())
    }

    fn backup_urls(&self) -> impl Iterator<Item = &String> {
        self.backup_url
            .iter()
            .flatten()
            .chain(self.backup_url_camel.iter().flatten())
    }
}

#[derive(Debug, Clone)]
pub struct CdnCandidate {
    pub url: String,
    pub host: String,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct RankedStreams {
    pub video: Vec<CdnCandidate>,
    pub audio: Vec<CdnCandidate>,
}

#[derive(Clone, Copy)]
struct CachedScore {
    measured_at: Instant,
    probe: ProbeScore,
}

#[derive(Debug, Clone, Copy)]
struct ProbeScore {
    latency: Duration,
    throughput_bps: f64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct CdnHistory {
    attempts: u64,
    corruptions: u64,
    probe_samples: u64,
    probe_failures: u64,
    last_probe_ok: bool,
    latency_ms: f64,
    throughput_bps: f64,
    last_probed_at: i64,
    video_score: Option<f64>,
    video_speed_ratio: Option<f64>,
    video_bandwidth: Option<i64>,
    audio_score: Option<f64>,
    audio_speed_ratio: Option<f64>,
    audio_bandwidth: Option<i64>,
    catalog_reachable: Option<bool>,
    catalog_latency_ms: Option<f64>,
    catalog_probed_at: i64,
}

#[derive(Clone, Copy)]
enum StreamKind {
    Video,
    Audio,
}

static CDN_SCORES: OnceLock<Mutex<HashMap<String, CachedScore>>> = OnceLock::new();
static CDN_HISTORY: OnceLock<Mutex<HashMap<String, CdnHistory>>> = OnceLock::new();
static CDN_HISTORY_BASE: OnceLock<HashMap<String, CdnHistory>> = OnceLock::new();
struct HistoryWrite {
    values: HashMap<String, CdnHistory>,
    completed: Option<mpsc::SyncSender<bool>>,
}

static HISTORY_WRITER: OnceLock<mpsc::Sender<HistoryWrite>> = OnceLock::new();
static HISTORY_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scores() -> &'static Mutex<HashMap<String, CachedScore>> {
    CDN_SCORES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .host_str()
        .map(ToOwned::to_owned)
}

fn history_path() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push("bilibili-tui");
    path.push("cdn-history.json");
    Some(path)
}

fn history() -> &'static Mutex<HashMap<String, CdnHistory>> {
    CDN_HISTORY.get_or_init(|| {
        let values: HashMap<String, CdnHistory> = history_path()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let _ = CDN_HISTORY_BASE.set(values.clone());
        Mutex::new(values)
    })
}

fn write_history(
    previous: &HashMap<String, CdnHistory>,
    values: &HashMap<String, CdnHistory>,
) -> bool {
    let Some(path) = history_path() else {
        return false;
    };
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    let lock_path = path.with_extension("json.lock");
    let mut lock = None;
    for _ in 0..100 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => {
                lock = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&lock_path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(30));
                if stale {
                    let _ = fs::remove_file(&lock_path);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return false,
        }
    }
    let Some(_lock) = lock else { return false };
    let mut merged: HashMap<String, CdnHistory> = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for (host, value) in values {
        let old = previous.get(host).cloned().unwrap_or_default();
        let target = merged.entry(host.clone()).or_default();
        target.attempts = target
            .attempts
            .saturating_add(value.attempts.saturating_sub(old.attempts));
        target.corruptions = target
            .corruptions
            .saturating_add(value.corruptions.saturating_sub(old.corruptions));
        target.probe_samples = target
            .probe_samples
            .saturating_add(value.probe_samples.saturating_sub(old.probe_samples));
        target.probe_failures = target
            .probe_failures
            .saturating_add(value.probe_failures.saturating_sub(old.probe_failures));
        if value.last_probed_at >= target.last_probed_at {
            target.last_probe_ok = value.last_probe_ok;
            target.latency_ms = value.latency_ms;
            target.throughput_bps = value.throughput_bps;
            target.last_probed_at = value.last_probed_at;
            target.video_score = value.video_score;
            target.video_speed_ratio = value.video_speed_ratio;
            target.video_bandwidth = value.video_bandwidth;
            target.audio_score = value.audio_score;
            target.audio_speed_ratio = value.audio_speed_ratio;
            target.audio_bandwidth = value.audio_bandwidth;
        }
        if value.catalog_probed_at >= target.catalog_probed_at {
            target.catalog_reachable = value.catalog_reachable;
            target.catalog_latency_ms = value.catalog_latency_ms;
            target.catalog_probed_at = value.catalog_probed_at;
        }
    }
    let success = if let Ok(bytes) = serde_json::to_vec_pretty(&merged) {
        let sequence = HISTORY_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("json.{}.{sequence}.tmp", std::process::id()));
        if fs::write(&temporary, bytes).is_ok() {
            fs::rename(&temporary, &path).is_ok()
        } else {
            false
        }
    } else {
        false
    };
    let _ = fs::remove_file(lock_path);
    success
}

fn save_history(values: &HashMap<String, CdnHistory>) {
    let sender = HISTORY_WRITER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<HistoryWrite>();
        std::thread::Builder::new()
            .name("cdn-history-writer".into())
            .spawn(move || {
                let mut previous = CDN_HISTORY_BASE.get().cloned().unwrap_or_default();
                while let Ok(mut pending) = rx.recv() {
                    while pending.completed.is_none()
                        && let Ok(newer) = rx.try_recv()
                    {
                        pending = newer;
                    }
                    let written = write_history(&previous, &pending.values);
                    if written {
                        previous = pending.values;
                    }
                    if let Some(completed) = pending.completed {
                        let _ = completed.send(written);
                    }
                }
            })
            .expect("spawn CDN history writer");
        tx
    });
    let _ = sender.send(HistoryWrite {
        values: values.clone(),
        completed: None,
    });
}

fn save_history_durable(values: HashMap<String, CdnHistory>) {
    let Some(sender) = HISTORY_WRITER.get().cloned().or_else(|| {
        save_history(&values);
        HISTORY_WRITER.get().cloned()
    }) else {
        return;
    };
    for _ in 0..3 {
        let (completed_tx, completed_rx) = mpsc::sync_channel(0);
        if sender
            .send(HistoryWrite {
                values: values.clone(),
                completed: Some(completed_tx),
            })
            .is_err()
        {
            return;
        }
        if completed_rx.recv_timeout(Duration::from_secs(2)) == Ok(true) {
            return;
        }
    }
}

fn record_probe(host: &str, probe: ProbeScore) {
    if let Ok(mut values) = history().lock() {
        let entry = values.entry(host.to_string()).or_default();
        let latency_ms = probe.latency.as_secs_f64() * 1000.0;
        let alpha = if entry.probe_samples == 0 { 1.0 } else { 0.25 };
        entry.latency_ms = entry.latency_ms * (1.0 - alpha) + latency_ms * alpha;
        entry.throughput_bps = entry.throughput_bps * (1.0 - alpha) + probe.throughput_bps * alpha;
        entry.probe_samples = entry.probe_samples.saturating_add(1);
        entry.last_probe_ok = true;
        entry.last_probed_at = chrono::Utc::now().timestamp();
        save_history(&values);
    }
}

fn record_probe_failure(host: &str) {
    if let Ok(mut values) = history().lock() {
        let entry = values.entry(host.to_string()).or_default();
        entry.probe_failures = entry.probe_failures.saturating_add(1);
        entry.last_probe_ok = false;
        entry.last_probed_at = chrono::Utc::now().timestamp();
        save_history(&values);
    }
}

fn catalog_prior(host: &str) -> Option<f64> {
    let value = history().lock().ok()?.get(host).cloned()?;
    match value.catalog_reachable {
        Some(true) => value
            .catalog_latency_ms
            .map(|latency| latency_score(Duration::from_secs_f64(latency / 1000.0))),
        Some(false) => Some(0.0),
        None => None,
    }
}

fn record_rank(host: &str, kind: StreamKind, score: f64, speed_ratio: f64, bandwidth: i64) {
    if let Ok(mut values) = history().lock() {
        let entry = values.entry(host.to_string()).or_default();
        match kind {
            StreamKind::Video => {
                entry.video_score = Some(score);
                entry.video_speed_ratio = Some(speed_ratio);
                entry.video_bandwidth = Some(bandwidth);
            }
            StreamKind::Audio => {
                entry.audio_score = Some(score);
                entry.audio_speed_ratio = Some(speed_ratio);
                entry.audio_bandwidth = Some(bandwidth);
            }
        }
        save_history(&values);
    }
}

pub fn record_cdn_result(host: &str, corrupted: bool) {
    let snapshot = if let Ok(mut values) = history().lock() {
        let entry = values.entry(host.to_string()).or_default();
        entry.attempts = entry.attempts.saturating_add(1);
        if corrupted {
            entry.corruptions = entry.corruptions.saturating_add(1);
        }
        Some(values.clone())
    } else {
        None
    };
    // Playback outcomes are sparse and must survive an immediate application
    // exit, unlike high-frequency probe samples handled by the writer thread.
    if let Some(values) = snapshot {
        save_history_durable(values);
    }
}

fn reliability(host: &str) -> f64 {
    let value = history()
        .lock()
        .ok()
        .and_then(|values| values.get(host).cloned())
        .unwrap_or_default();
    1.0 - (value.corruptions as f64 + 1.0) / (value.attempts as f64 + 10.0)
}

fn cached_score(url: &str) -> Option<ProbeScore> {
    let host = host(url)?;
    if let Some(cached) = scores().lock().ok()?.get(&host).copied()
        && cached.measured_at.elapsed() < SCORE_TTL
    {
        return Some(cached.probe);
    }
    let value = history().lock().ok()?.get(&host).cloned()?;
    history_score(&value, chrono::Utc::now().timestamp())
}

/// Reuse a persisted probe only while it is recent and the latest probe of
/// the host succeeded.
fn history_score(value: &CdnHistory, now: i64) -> Option<ProbeScore> {
    let age = now - value.last_probed_at;
    (value.last_probe_ok && value.probe_samples > 0 && age >= 0 && age < SCORE_TTL.as_secs() as i64)
        .then(|| ProbeScore {
            latency: Duration::from_secs_f64(value.latency_ms / 1000.0),
            throughput_bps: value.throughput_bps,
        })
}

fn failed_recently(value: &CdnHistory, now: i64) -> bool {
    let age = now - value.last_probed_at;
    !value.last_probe_ok && value.probe_failures > 0 && age >= 0 && age < SCORE_TTL.as_secs() as i64
}

/// Pick a few catalog hosts worth probing: skip hosts the API already gave
/// and hosts that just failed, prefer the detected region and hosts with a
/// good probe history, then fill remaining slots with unexplored hosts.
fn select_catalog_hosts<'a>(
    catalog_hosts: &'a [String],
    known_hosts: &[String],
    region: Option<catalog::Region>,
    history: &HashMap<String, CdnHistory>,
    now: i64,
) -> Vec<&'a str> {
    let mut candidates: Vec<(&str, Option<&CdnHistory>)> = catalog_hosts
        .iter()
        .map(String::as_str)
        .filter(|host| catalog::is_trusted_media_host(host))
        .filter(|host| !known_hosts.iter().any(|known| known == host))
        .map(|host| (host, history.get(host)))
        .filter(|(_, value)| !value.is_some_and(|value| failed_recently(value, now)))
        .collect();
    candidates.sort_by(|(a_host, a), (b_host, b)| {
        let rank = |host: &str, value: Option<&CdnHistory>| {
            let region_mismatch = region_factor(region, host) < 1.0;
            let known_good = value.is_some_and(|value| value.last_probe_ok);
            let latency = value
                .filter(|value| value.last_probe_ok)
                .map_or(f64::INFINITY, |value| value.latency_ms);
            (region_mismatch, !known_good, latency)
        };
        let (a_region, a_unknown, a_latency) = rank(a_host, *a);
        let (b_region, b_unknown, b_latency) = rank(b_host, *b);
        a_region
            .cmp(&b_region)
            .then(a_unknown.cmp(&b_unknown))
            .then(a_latency.total_cmp(&b_latency))
    });
    candidates
        .into_iter()
        .take(MAX_CATALOG_PROBES)
        .map(|(host, _)| host)
        .collect()
}

async fn probe(client: &reqwest::Client, url: String) -> (String, Option<ProbeScore>) {
    if let Some(score) = cached_score(&url) {
        return (url, Some(score));
    }

    let started = Instant::now();
    let result = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut response = client
            .get(&url)
            .header(RANGE, format!("bytes=0-{PROBE_BYTES}"))
            .header(REFERER, "https://www.bilibili.com/")
            .header(USER_AGENT, USER_AGENT_VALUE)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("HTTP {}", response.status()));
        }
        let mut received = 0usize;
        let mut first_byte = None;
        while let Some(chunk) = response.chunk().await? {
            first_byte.get_or_insert_with(|| started.elapsed());
            received += chunk.len();
            if received > PROBE_BYTES as usize {
                break;
            }
        }
        if received == 0 {
            return Err(anyhow!("empty CDN response"));
        }
        let elapsed = started.elapsed();
        Ok::<_, anyhow::Error>(ProbeScore {
            latency: first_byte.unwrap_or(elapsed),
            throughput_bps: received as f64 * 8.0 / elapsed.as_secs_f64(),
        })
    })
    .await;

    let score = result.ok().and_then(Result::ok);
    if let Some(host) = host(&url) {
        if let Some(score) = score {
            if let Ok(mut cache) = scores().lock() {
                cache.insert(
                    host.clone(),
                    CachedScore {
                        measured_at: Instant::now(),
                        probe: score,
                    },
                );
            }
            record_probe(&host, score);
        } else {
            if let Ok(mut cache) = scores().lock() {
                cache.remove(&host);
            }
            record_probe_failure(&host);
        }
    }
    (url, score)
}

async fn rank_urls(
    stream: &DashStream,
    kind: StreamKind,
    region: Option<catalog::Region>,
    catalog_hosts: &[String],
) -> Result<Vec<CdnCandidate>> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(800))
        .build()?;
    let mut urls = Vec::with_capacity(
        1 + stream.backup_url.as_ref().map_or(0, Vec::len)
            + stream.backup_url_camel.as_ref().map_or(0, Vec::len),
    );
    let primary = stream
        .primary_url()
        .ok_or_else(|| anyhow!("CDN 流缺少主地址"))?;
    urls.push(primary.to_string());
    urls.extend(stream.backup_urls().cloned());
    // Bilibili UPOS signatures belong to the media path/query and can be used
    // with another UPOS edge. Preserve every signed component and replace only
    // the host so a few trusted catalog nodes participate in media probing.
    let known_hosts: Vec<String> = urls.iter().filter_map(|url| host(url)).collect();
    let selected = {
        let history = history()
            .lock()
            .map(|values| values.clone())
            .unwrap_or_default();
        select_catalog_hosts(
            catalog_hosts,
            &known_hosts,
            region,
            &history,
            chrono::Utc::now().timestamp(),
        )
    };
    if let Ok(template) = reqwest::Url::parse(primary) {
        for catalog_host in selected {
            let mut rewritten = template.clone();
            if rewritten.set_host(Some(catalog_host)).is_ok() {
                urls.push(rewritten.to_string());
            }
        }
    }
    urls.sort_by_key(|url| host(url));
    urls.dedup_by(|a, b| host(a) == host(b));

    let results: Vec<_> = stream::iter(urls.into_iter().map(|url| probe(&client, url)))
        .buffer_unordered(PROBE_CONCURRENCY)
        .collect()
        .await;
    let all_ranked = results
        .into_iter()
        .filter_map(|(url, probe)| {
            let probe = probe?;
            let host = host(&url)?;
            let latency = latency_score(probe.latency);
            let ratio = if probe.throughput_bps == 0.0 || stream.bandwidth <= 0 {
                1.0
            } else {
                probe.throughput_bps / stream.bandwidth as f64
            };
            let speed = speed_score(ratio);
            let region_factor = region_factor(region, &host);
            let base_score = (reliability(&host) * 0.55 + latency * 0.35 + speed.min(1.0) * 0.10)
                * region_factor;
            let score = catalog_prior(&host)
                .map(|prior| base_score * 0.95 + prior * 0.05)
                .unwrap_or(base_score);
            record_rank(&host, kind, score, ratio, stream.bandwidth);
            Some(CdnCandidate { url, host, score })
        })
        .collect::<Vec<_>>();
    let mut ranked = all_ranked;
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    (!ranked.is_empty())
        .then_some(ranked)
        .ok_or_else(|| anyhow!("没有可用的 CDN 节点"))
}

fn region_factor(region: Option<catalog::Region>, host: &str) -> f64 {
    match region {
        Some(catalog::Region::MainlandChina) if catalog::is_overseas_host(host) => 0.92,
        Some(catalog::Region::Overseas) if !catalog::is_overseas_host(host) => 0.92,
        _ => 1.0,
    }
}

fn latency_score(latency: Duration) -> f64 {
    1.0 / (1.0 + latency.as_secs_f64() / 0.2)
}

fn speed_score(ratio: f64) -> f64 {
    if ratio < 1.0 {
        ratio.max(0.0).powi(2) * 0.6
    } else if ratio <= 1.5 {
        0.6 + (ratio - 1.0) * 0.7
    } else {
        0.95 + (1.0 - (-(ratio - 1.5)).exp()) * 0.05
    }
}

pub async fn rank_streams(
    data: &PlayUrlData,
    quality: crate::storage::VideoQuality,
) -> Result<RankedStreams> {
    let video = select_video_stream(data, quality)?;
    let mut audio = data.dash.audio.iter().collect::<Vec<_>>();
    if let Some(dolby) = &data.dash.dolby {
        audio.extend(dolby.audio.iter().flatten());
    }
    if let Some(flac) = &data.dash.flac
        && let Some(stream) = &flac.audio
    {
        audio.push(stream);
    }
    let audio = audio
        .into_iter()
        .max_by_key(|stream| stream.bandwidth)
        .ok_or_else(|| anyhow!("播放地址没有音频流"))?;
    let region_client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(800))
        .build()?;
    let (region, catalog_hosts) = catalog::regional_hosts(&region_client).await;
    let (video, audio) = tokio::join!(
        rank_urls(video, StreamKind::Video, region, &catalog_hosts),
        rank_urls(audio, StreamKind::Audio, region, &catalog_hosts)
    );
    Ok(RankedStreams {
        video: video?,
        audio: audio?,
    })
}

fn select_video_stream(
    data: &PlayUrlData,
    quality: crate::storage::VideoQuality,
) -> Result<&DashStream> {
    data.dash
        .video
        .iter()
        .filter(|stream| stream.id <= quality.qn())
        .max_by_key(|stream| (stream.id, stream.bandwidth))
        .ok_or_else(|| anyhow!("播放地址没有符合画质上限的视频流"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playurl_accepts_camel_and_snake_case_urls() {
        let value = serde_json::json!({
            "dash": {
                "video": [{"id": 80, "bandwidth": 1, "baseUrl": "https://a/v", "backupUrl": ["https://b/v"]}],
                "audio": [{"id": 30280, "bandwidth": 2, "base_url": "https://a/a", "backup_url": ["https://b/a"]}],
                "dolby": null,
                "flac": null
            }
        });
        let data: PlayUrlData = serde_json::from_value(value).unwrap();
        assert_eq!(
            data.dash.video[0].backup_url_camel.as_ref().map(Vec::len),
            Some(1)
        );
        assert_eq!(data.dash.audio[0].base_url.as_deref(), Some("https://a/a"));
    }

    #[test]
    fn video_stream_selection_respects_quality_cap() {
        let data: PlayUrlData = serde_json::from_value(serde_json::json!({
            "dash": {
                "video": [
                    {"id": 120, "bandwidth": 300},
                    {"id": 80, "bandwidth": 200},
                    {"id": 64, "bandwidth": 100}
                ],
                "audio": [],
                "dolby": null,
                "flac": null
            }
        }))
        .unwrap();

        assert_eq!(
            select_video_stream(&data, crate::storage::VideoQuality::Best)
                .unwrap()
                .id,
            120
        );
        assert_eq!(
            select_video_stream(&data, crate::storage::VideoQuality::Q1080p)
                .unwrap()
                .id,
            80
        );
        assert_eq!(
            select_video_stream(&data, crate::storage::VideoQuality::Q720p)
                .unwrap()
                .id,
            64
        );
        assert!(select_video_stream(&data, crate::storage::VideoQuality::Q360p).is_err());
    }

    #[test]
    fn speed_penalizes_below_bitrate_and_flattens_above_one_point_five() {
        assert!(speed_score(0.5) < speed_score(0.9));
        assert!(speed_score(0.9) < speed_score(1.0));
        assert!(speed_score(1.5) - speed_score(1.0) > speed_score(3.0) - speed_score(1.5));
        assert!(speed_score(10.0) <= 1.0);
    }

    #[test]
    fn latency_rewards_low_time_to_first_byte() {
        assert!(
            latency_score(Duration::from_millis(20)) > latency_score(Duration::from_millis(200))
        );
    }

    #[test]
    fn unknown_region_does_not_bias_cdn_hosts() {
        assert_eq!(region_factor(None, "upos-hz-mirrorakam.akamaized.net"), 1.0);
        assert_eq!(region_factor(None, "upos-sz-mirrorali.bilivideo.com"), 1.0);
        assert!(
            region_factor(
                Some(catalog::Region::Overseas),
                "upos-sz-mirrorali.bilivideo.com"
            ) < 1.0
        );
    }

    #[test]
    fn ranking_database_accepts_legacy_history_records() {
        let value: CdnHistory = serde_json::from_value(serde_json::json!({
            "attempts": 10,
            "corruptions": 1
        }))
        .unwrap();
        assert_eq!(value.attempts, 10);
        assert_eq!(value.corruptions, 1);
        assert_eq!(value.probe_samples, 0);
        assert!(value.video_score.is_none());
        assert_eq!(value.catalog_reachable, None);
    }

    fn probed(ok: bool, latency_ms: f64, at: i64) -> CdnHistory {
        CdnHistory {
            probe_samples: 1,
            probe_failures: u64::from(!ok),
            last_probe_ok: ok,
            latency_ms,
            last_probed_at: at,
            ..CdnHistory::default()
        }
    }

    #[test]
    fn host_that_just_failed_is_not_scored_from_history() {
        let now = 1_000_000;
        assert!(history_score(&probed(true, 40.0, now - 10), now).is_some());
        assert!(history_score(&probed(false, 40.0, now - 10), now).is_none());
        let expired = now - SCORE_TTL.as_secs() as i64;
        assert!(history_score(&probed(true, 40.0, expired), now).is_none());
    }

    #[test]
    fn catalog_selection_is_capped_trusted_and_skips_failed_hosts() {
        let now = 1_000_000;
        let mut catalog_hosts: Vec<String> = (0..20)
            .map(|index| format!("upos-sz-mirror{index:02}.bilivideo.com"))
            .collect();
        catalog_hosts.push("upos-evil.akamaized.net".to_string());
        let history = HashMap::from([
            (
                "upos-sz-mirror05.bilivideo.com".to_string(),
                probed(true, 30.0, now - 60),
            ),
            (
                "upos-sz-mirror00.bilivideo.com".to_string(),
                probed(false, 0.0, now - 60),
            ),
        ]);
        let known = vec!["upos-sz-mirror01.bilivideo.com".to_string()];

        let selected = select_catalog_hosts(&catalog_hosts, &known, None, &history, now);

        assert_eq!(selected.len(), MAX_CATALOG_PROBES);
        assert_eq!(selected[0], "upos-sz-mirror05.bilivideo.com");
        assert!(!selected.contains(&"upos-sz-mirror00.bilivideo.com"));
        assert!(!selected.contains(&"upos-sz-mirror01.bilivideo.com"));
        assert!(!selected.contains(&"upos-evil.akamaized.net"));
    }

    #[test]
    fn catalog_latency_is_a_small_ranking_prior() {
        let fast = latency_score(Duration::from_millis(20));
        let slow = latency_score(Duration::from_millis(500));
        assert!(fast > slow);
        let base = 0.8;
        assert!(base * 0.95 + fast * 0.05 > base * 0.95 + slow * 0.05);
    }
}
