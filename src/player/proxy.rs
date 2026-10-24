use crate::api::cdn::{CdnCandidate, PlayUrlData, RankedStreams, record_cdn_result};
use anyhow::{Result, anyhow};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE, REFERER, USER_AGENT};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio_util::sync::CancellationToken;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36";

/// Fetches a new playurl response for the stream a proxy serves.
pub type Refresher =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<PlayUrlData>> + Send>> + Send + Sync>;

/// Shortest time between two playurl refreshes of one proxy.
const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

struct ProxyState {
    client: reqwest::Client,
    /// Replaced as a whole when expired signatures are refreshed; indices
    /// keep naming the same host across refreshes.
    streams: std::sync::RwLock<Arc<RankedStreams>>,
    refresher: Option<Refresher>,
    /// When the last refresh started; held while a refresh runs.
    last_refresh: Mutex<Option<std::time::Instant>>,
    video_index: AtomicUsize,
    audio_index: AtomicUsize,
    prefixes: Mutex<HashMap<usize, Arc<CachedPrefix>>>,
    audio_cache: RwLock<Option<Arc<CachedPrefix>>>,
    /// Cancels the running audio prefetch when a newer one starts.
    audio_prefetch: std::sync::Mutex<Option<CancellationToken>>,
    cancellation: CancellationToken,
}

/// Largest audio track kept in memory; longer tracks stream from the CDN.
const MAX_AUDIO_CACHE: u64 = 64 * 1024 * 1024;

struct CachedPrefix {
    bytes: Vec<u8>,
    total: u64,
}

impl ProxyState {
    fn streams(&self) -> Arc<RankedStreams> {
        match self.streams.read() {
            Ok(streams) => streams.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn video(&self, index: usize) -> Option<CdnCandidate> {
        self.streams().video.get(index).cloned()
    }

    fn audio(&self, index: usize) -> Option<CdnCandidate> {
        self.streams().audio.get(index).cloned()
    }

    fn video_len(&self) -> usize {
        self.streams().video.len()
    }

    /// Replace expired signatures with ones from a new playurl response.
    /// Returns whether the candidates may now succeed: a refresh that
    /// another request finished moments ago counts as success.
    async fn refresh(&self) -> bool {
        let Some(refresher) = &self.refresher else {
            return false;
        };
        let mut last = self.last_refresh.lock().await;
        if last.is_some_and(|at| at.elapsed() < REFRESH_INTERVAL) {
            return true;
        }
        *last = Some(std::time::Instant::now());
        let Ok(data) = refresher().await else {
            return false;
        };
        let Ok(refreshed) = crate::api::cdn::refreshed_streams(&self.streams(), &data) else {
            return false;
        };
        match self.streams.write() {
            Ok(mut streams) => *streams = Arc::new(refreshed),
            Err(poisoned) => *poisoned.into_inner() = Arc::new(refreshed),
        }
        true
    }
}

pub struct MediaProxy {
    state: Arc<ProxyState>,
    pub video_url: String,
    /// `None` when the stream has no separate audio track.
    pub audio_url: Option<String>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl MediaProxy {
    pub async fn start(streams: RankedStreams, refresher: Option<Refresher>) -> Result<Self> {
        if streams.video.is_empty() {
            return Err(anyhow!("CDN 候选地址为空"));
        }
        let has_audio = !streams.audio.is_empty();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let state = Arc::new(ProxyState {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(3))
                .read_timeout(std::time::Duration::from_secs(15))
                .build()?,
            streams: std::sync::RwLock::new(Arc::new(streams)),
            refresher,
            last_refresh: Mutex::new(None),
            video_index: AtomicUsize::new(0),
            audio_index: AtomicUsize::new(0),
            prefixes: Mutex::new(HashMap::new()),
            audio_cache: RwLock::new(None),
            audio_prefetch: std::sync::Mutex::new(None),
            cancellation: CancellationToken::new(),
        });
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let server_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((socket, _)) = accepted else { break };
                        let state = server_state.clone();
                        tokio::spawn(async move {
                            tokio::select! {
                                _ = state.cancellation.cancelled() => {}
                                result = serve(socket, state.clone()) => { let _ = result; }
                            }
                        });
                    }
                }
            }
        });
        prefetch_backup(state.clone());
        if has_audio {
            prefetch_audio(state.clone());
        }
        Ok(Self {
            state,
            video_url: format!("http://{address}/video?generation=0"),
            audio_url: has_audio.then(|| format!("http://{address}/audio")),
            shutdown: Some(shutdown_tx),
        })
    }

    pub fn next_video_cdn(&self) -> Option<(usize, String)> {
        let next = self.state.video_index.load(Ordering::Acquire) + 1;
        self.video_cdn(next)
    }

    pub fn video_cdn(&self, index: usize) -> Option<(usize, String)> {
        (index < self.state.video_len()).then(|| {
            let base = self.video_url.split('?').next().unwrap_or(&self.video_url);
            (index, format!("{base}?generation={index}"))
        })
    }

    pub fn commit_video_cdn(&mut self, next: usize) -> bool {
        let current = self.state.video_index.load(Ordering::Acquire);
        if next <= current || next >= self.state.video_len() {
            return false;
        }
        self.state.video_index.store(next, Ordering::Release);
        self.video_url = format!(
            "{}?generation={next}",
            self.video_url.split('?').next().unwrap()
        );
        prefetch_backup(self.state.clone());
        true
    }

    pub fn record_current_corruption(&self) {
        let index = self.state.video_index.load(Ordering::Acquire);
        if let Some(candidate) = self.state.video(index) {
            record_cdn_result(&candidate.host, true);
        }
    }

    #[cfg(test)]
    pub fn switch_video_cdn_for_test(&mut self) -> Option<String> {
        let (next, url) = self.next_video_cdn()?;
        self.state.video_index.store(next, Ordering::Release);
        self.video_url = url.clone();
        Some(url)
    }

    pub fn record_success(&self) {
        let index = self.state.video_index.load(Ordering::Relaxed);
        if let Some(candidate) = self.state.video(index) {
            record_cdn_result(&candidate.host, false);
        }
    }
}

/// Download the current audio track into memory. Starting a new prefetch
/// (after an audio failover) cancels the previous one, so the track is never
/// downloaded twice at once and an older download cannot undo the failover.
fn prefetch_audio(state: Arc<ProxyState>) {
    let token = state.cancellation.child_token();
    if let Ok(mut running) = state.audio_prefetch.lock()
        && let Some(previous) = running.replace(token.clone())
    {
        previous.cancel();
    }
    tokio::spawn(async move {
        let download = async {
            let start = state.audio_index.load(Ordering::Acquire);
            let audio = state.streams().audio.clone();
            for (index, candidate) in audio.iter().enumerate().skip(start) {
                let Ok(mut response) = state
                    .client
                    .get(&candidate.url)
                    .header(RANGE, "bytes=0-")
                    .header(REFERER, "https://www.bilibili.com/")
                    .header(USER_AGENT, UA)
                    .send()
                    .await
                else {
                    continue;
                };
                let Some(total) = cacheable_audio_length(&response) else {
                    continue;
                };
                let mut bytes = Vec::with_capacity(total as usize);
                while let Ok(Some(chunk)) = response.chunk().await {
                    if bytes.len().saturating_add(chunk.len()) > total as usize {
                        bytes.clear();
                        break;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                if bytes.len() as u64 == total {
                    return Some((index, CachedPrefix { bytes, total }));
                }
            }
            None
        };
        tokio::select! {
            _ = token.cancelled() => {}
            cached = download => {
                if let Some((index, cached)) = cached
                    && !token.is_cancelled()
                {
                    state.audio_index.store(index, Ordering::Release);
                    *state.audio_cache.write().await = Some(Arc::new(cached));
                }
            }
        }
    });
}

fn cacheable_audio_length(response: &reqwest::Response) -> Option<u64> {
    cacheable_audio_length_parts(
        response.status(),
        response.headers(),
        response.content_length(),
    )
}

fn cacheable_audio_length_parts(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    content_length: Option<u64>,
) -> Option<u64> {
    let total = match status {
        reqwest::StatusCode::PARTIAL_CONTENT => headers
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_range)
            .map(|(_, _, total)| total),
        reqwest::StatusCode::OK => content_length,
        _ => None,
    }?;
    (total > 0 && total <= MAX_AUDIO_CACHE).then_some(total)
}

impl Drop for MediaProxy {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.state.cancellation.cancel();
    }
}

fn prefetch_backup(state: Arc<ProxyState>) {
    let next = state.video_index.load(Ordering::Relaxed) + 1;
    let Some(candidate) = state.video(next) else {
        return;
    };
    tokio::spawn(async move {
        let prefetch = async {
            let Ok(mut response) = state
                .client
                .get(candidate.url)
                .header(RANGE, "bytes=0-1048575")
                .header(REFERER, "https://www.bilibili.com/")
                .header(USER_AGENT, UA)
                .send()
                .await
                .and_then(|response| response.error_for_status())
            else {
                return;
            };
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                return;
            }
            let Some((range_start, range_end, total)) = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
            else {
                return;
            };
            if range_start != 0 || range_end < range_start || total <= range_end {
                return;
            }
            let mut bytes = Vec::with_capacity(1024 * 1024);
            while let Ok(Some(chunk)) = response.chunk().await {
                let remaining = 1024 * 1024usize - bytes.len();
                bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                if bytes.len() >= 1024 * 1024 {
                    break;
                }
            }
            if !bytes.is_empty() {
                state
                    .prefixes
                    .lock()
                    .await
                    .insert(next, Arc::new(CachedPrefix { bytes, total }));
            }
        };
        tokio::select! {
            _ = state.cancellation.cancelled() => {}
            _ = prefetch => {}
        }
    });
}

fn parse_range(value: &str) -> Option<(usize, Option<usize>)> {
    let value = value.strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    Some((
        start.parse().ok()?,
        (!end.is_empty()).then(|| end.parse().ok()).flatten(),
    ))
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

fn generation(path: &str) -> Option<usize> {
    path.split_once('?')?.1.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "generation").then(|| value.parse().ok()).flatten()
    })
}

async fn serve(mut socket: TcpStream, state: Arc<ProxyState>) -> Result<()> {
    let mut request = Vec::with_capacity(4096);
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|value| value == b"\r\n\r\n") && request.len() < 32 * 1024 {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
    }
    if !request.windows(4).any(|value| value == b"\r\n\r\n") {
        socket
            .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }
    let request = String::from_utf8_lossy(&request);
    let mut lines = request.lines();
    let first = lines.next().ok_or_else(|| anyhow!("empty proxy request"))?;
    let mut request_parts = first.split_whitespace();
    let method = request_parts.next().unwrap_or("");
    let path = request_parts.next().unwrap_or("/");
    let route = path.split('?').next().unwrap_or(path);
    if !matches!(method, "GET" | "HEAD") {
        socket
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }
    let range = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range").then(|| value.trim())
    });
    let video_index = generation(path)
        .unwrap_or(0)
        .max(state.video_index.load(Ordering::Acquire));
    let candidate = match route {
        "/video" => state.video(video_index),
        "/audio" => state.audio(state.audio_index.load(Ordering::Acquire)),
        _ => None,
    };
    let Some(candidate) = candidate else {
        socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    };
    if let Some((start, requested_end)) = range.and_then(parse_range) {
        // Clone the cached prefix out of its lock: socket writes can stall for
        // as long as mpv takes to read, and must not block prefetch writers.
        let (cached, content_type) = match route {
            "/audio" => (state.audio_cache.read().await.clone(), "audio/mp4"),
            "/video" => (
                state.prefixes.lock().await.get(&video_index).cloned(),
                "video/mp4",
            ),
            _ => (None, ""),
        };
        if let Some(cached) = cached
            && let Some((head, body)) = cached_range(&cached, start, requested_end, content_type)
        {
            socket.write_all(head.as_bytes()).await?;
            if method == "GET" {
                socket.write_all(&cached.bytes[body]).await?;
            }
            return Ok(());
        }
    }
    let send = |candidate: &CdnCandidate| {
        let mut request = state
            .client
            .get(&candidate.url)
            .header(REFERER, "https://www.bilibili.com/")
            .header(USER_AGENT, UA);
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        request.send()
    };
    let mut response = match send(&candidate).await {
        Ok(response) if upstream_outcome(response.status()) != UpstreamOutcome::Failover => {
            response
        }
        first => {
            let mut recovered = None;
            // Only transport errors and server errors say something about the
            // CDN; 403/404 also happen on healthy hosts (expired signatures).
            let blame_host = match &first {
                Err(_) => true,
                Ok(response) => response.status().is_server_error(),
            };
            if route == "/audio" {
                let start = state.audio_index.load(Ordering::Acquire) + 1;
                let audio = state.streams().audio.clone();
                for (index, candidate) in audio.iter().enumerate().skip(start) {
                    if let Ok(response) = send(candidate).await
                        && upstream_outcome(response.status()) == UpstreamOutcome::Serve
                    {
                        state.audio_index.store(index, Ordering::Release);
                        prefetch_audio(state.clone());
                        recovered = Some(response);
                        break;
                    }
                }
            } else if route == "/video" {
                if blame_host {
                    record_cdn_result(&candidate.host, true);
                }
                let video = state.streams().video.clone();
                for (index, candidate) in video.iter().enumerate().skip(video_index + 1) {
                    if let Ok(response) = send(candidate).await
                        && upstream_outcome(response.status()) == UpstreamOutcome::Serve
                    {
                        state.video_index.store(index, Ordering::Release);
                        prefetch_backup(state.clone());
                        recovered = Some(response);
                        break;
                    }
                }
            }
            // Every host refusing the request means the signatures expired,
            // typically after a long pause: fetch new ones and retry the
            // host that was serving.
            if recovered.is_none() && !blame_host && state.refresh().await {
                let retry = match route {
                    "/video" => state.video(state.video_index.load(Ordering::Acquire)),
                    _ => state.audio(state.audio_index.load(Ordering::Acquire)),
                };
                if let Some(retry) = retry
                    && let Ok(response) = send(&retry).await
                    && upstream_outcome(response.status()) != UpstreamOutcome::Failover
                {
                    recovered = Some(response);
                }
            }
            match recovered {
                Some(response) => response,
                None => {
                    // Answer mpv instead of resetting the connection, so it
                    // reports an HTTP error rather than retrying blindly.
                    let status = match &first {
                        Ok(response) => response.status().as_u16(),
                        Err(_) => 502,
                    };
                    let _ = socket.write_all(empty_response(status).as_bytes()).await;
                    return Err(match first {
                        Err(error) => {
                            anyhow!(error.without_url()).context("CDN proxy upstream request")
                        }
                        Ok(response) => anyhow!("CDN proxy upstream HTTP {}", response.status()),
                    });
                }
            }
        }
    };
    let status = response.status();
    let reason = status.canonical_reason().unwrap_or("OK");
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nConnection: close\r\n",
        status.as_u16(),
        reason
    );
    for name in [CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE] {
        if let Some(value) = response
            .headers()
            .get(&name)
            .and_then(|value| value.to_str().ok())
        {
            head.push_str(name.as_str());
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
    }
    head.push_str("Accept-Ranges: bytes\r\n\r\n");
    socket.write_all(head.as_bytes()).await?;
    if method == "GET" {
        while let Some(chunk) = response.chunk().await? {
            socket.write_all(&chunk).await?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamOutcome {
    /// Media bytes to stream back.
    Serve,
    /// A valid answer to this request, such as 416 for a range past the end.
    Relay,
    /// The candidate cannot serve the request; try the next one.
    Failover,
}

fn upstream_outcome(status: reqwest::StatusCode) -> UpstreamOutcome {
    match status {
        reqwest::StatusCode::OK | reqwest::StatusCode::PARTIAL_CONTENT => UpstreamOutcome::Serve,
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE => UpstreamOutcome::Relay,
        _ => UpstreamOutcome::Failover,
    }
}

fn empty_response(status: u16) -> String {
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Bad Gateway");
    format!("HTTP/1.1 {status} {reason}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
}

/// Response head and body range for a request served from a cached prefix,
/// or `None` when the requested range is not fully cached.
fn cached_range(
    cached: &CachedPrefix,
    start: usize,
    requested_end: Option<usize>,
    content_type: &str,
) -> Option<(String, std::ops::RangeInclusive<usize>)> {
    let end = requested_end.unwrap_or_else(|| cached.bytes.len().saturating_sub(1));
    (end < cached.bytes.len() && start <= end).then(|| {
        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nAccept-Ranges: bytes\r\n\r\n",
            end - start + 1,
            cached.total
        );
        (head, start..=end)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_valid_byte_ranges() {
        assert_eq!(parse_range("bytes=10-20"), Some((10, Some(20))));
        assert_eq!(parse_range("bytes=10-"), Some((10, None)));
        assert_eq!(parse_range("items=10-20"), None);
        assert_eq!(parse_range("bytes=-20"), None);
    }

    #[test]
    fn validates_content_range_and_generation() {
        assert_eq!(parse_content_range("bytes 0-99/1000"), Some((0, 99, 1000)));
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(generation("/video?generation=3"), Some(3));
        assert_eq!(generation("/video?x=1&generation=4"), Some(4));
    }

    #[test]
    fn accepts_bounded_audio_when_origin_ignores_range() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(
            cacheable_audio_length_parts(reqwest::StatusCode::OK, &headers, Some(1024)),
            Some(1024)
        );
        assert_eq!(
            cacheable_audio_length_parts(
                reqwest::StatusCode::OK,
                &headers,
                Some(257 * 1024 * 1024)
            ),
            None
        );
        assert_eq!(
            cacheable_audio_length_parts(
                reqwest::StatusCode::OK,
                &headers,
                Some(MAX_AUDIO_CACHE + 1)
            ),
            None
        );
    }

    #[test]
    fn range_past_the_end_is_relayed_instead_of_failing_over() {
        assert_eq!(
            upstream_outcome(reqwest::StatusCode::RANGE_NOT_SATISFIABLE),
            UpstreamOutcome::Relay
        );
        assert_eq!(
            upstream_outcome(reqwest::StatusCode::PARTIAL_CONTENT),
            UpstreamOutcome::Serve
        );
        assert_eq!(
            upstream_outcome(reqwest::StatusCode::FORBIDDEN),
            UpstreamOutcome::Failover
        );
        assert_eq!(
            empty_response(502),
            "HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn cached_prefix_serves_only_fully_cached_ranges() {
        let cached = CachedPrefix {
            bytes: vec![0; 100],
            total: 1000,
        };
        let (head, body) = cached_range(&cached, 10, Some(19), "video/mp4").unwrap();
        assert_eq!(body, 10..=19);
        assert!(head.contains("Content-Length: 10\r\n"));
        assert!(head.contains("Content-Range: bytes 10-19/1000\r\n"));
        assert_eq!(
            cached_range(&cached, 0, None, "video/mp4").unwrap().1,
            0..=99
        );
        assert!(cached_range(&cached, 50, Some(100), "video/mp4").is_none());
        assert!(cached_range(&cached, 20, Some(10), "video/mp4").is_none());
    }

    fn empty_streams() -> RankedStreams {
        RankedStreams {
            video: Vec::new(),
            audio: Vec::new(),
            identity: crate::api::cdn::StreamIdentity::Durl { quality: 80 },
        }
    }

    /// An upstream that serves only URLs carrying `sig=new`, like a CDN
    /// after the old signatures expired.
    async fn expiring_upstream() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 4096];
                    let read = socket.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let response = if request.lines().next().unwrap_or("").contains("sig=new") {
                        "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfresh"
                    } else {
                        "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    };
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        address
    }

    #[tokio::test]
    async fn expired_signatures_are_refreshed_and_the_request_retried() {
        let upstream = expiring_upstream().await;
        let old = format!("http://{upstream}/v.flv?sig=old");
        let streams = RankedStreams {
            video: vec![CdnCandidate {
                url: old,
                host: "127.0.0.1".to_string(),
                score: 1.0,
            }],
            ..empty_streams()
        };
        let refreshes = Arc::new(AtomicUsize::new(0));
        let counter = refreshes.clone();
        let refresher: Refresher = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let data = serde_json::json!({
                "quality": 80,
                "durl": [{"order": 1, "url": format!("http://{upstream}/v.flv?sig=new")}]
            });
            Box::pin(async move { Ok(serde_json::from_value(data)?) })
        });
        let proxy = MediaProxy::start(streams, Some(refresher)).await.unwrap();
        assert!(proxy.audio_url.is_none());

        let body = reqwest::get(&proxy.video_url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();

        assert_eq!(body, "fresh");
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn new_audio_prefetch_cancels_the_running_one() {
        let state = Arc::new(ProxyState {
            client: reqwest::Client::new(),
            streams: std::sync::RwLock::new(Arc::new(empty_streams())),
            refresher: None,
            last_refresh: Mutex::new(None),
            video_index: AtomicUsize::new(0),
            audio_index: AtomicUsize::new(0),
            prefixes: Mutex::new(HashMap::new()),
            audio_cache: RwLock::new(None),
            audio_prefetch: std::sync::Mutex::new(None),
            cancellation: CancellationToken::new(),
        });
        prefetch_audio(state.clone());
        let first = state.audio_prefetch.lock().unwrap().clone().unwrap();
        prefetch_audio(state.clone());
        assert!(first.is_cancelled());
        let second = state.audio_prefetch.lock().unwrap().clone().unwrap();
        assert!(!second.is_cancelled());
    }
}
