use crate::api::client::ApiClient;
use crate::api::danmaku::VideoDanmaku;
use crate::api::live_danmaku_hub::LiveDanmakuHub;
use crate::api::live_ws::LiveMessage;
use crate::domain::playback::{PlayOrder, PlaybackEvent, PlaylistItem};
use crate::storage::{Credentials, DanmakuConfig, VideoQuality};
use anyhow::Result;
use std::collections::VecDeque;
use std::io::Write as StdWrite;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixStream as MpvIpcStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{
    ClientOptions as MpvIpcClientOptions, NamedPipeClient as MpvIpcStream,
};
use tokio::process::Command;
use tokio::time::{Instant, interval_at, timeout};

mod proxy;

static LIVE_SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static MPV_IPC_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const LIVE_DANMAKU_SCRIPT: &str = include_str!("live_danmaku.lua");

/// yt-dlp's Bilibili extractor stores the stream `qn` in `quality`, so the cap
/// is applied to the same identifier as the DASH path. Height alone would let
/// 1080P+ / 1080P60 through a 1080P cap and mis-cap portrait videos. Formats
/// without a `quality` value are excluded rather than risk exceeding the cap.
fn ytdl_format(quality: VideoQuality) -> String {
    match quality {
        VideoQuality::Best => "bestvideo+bestaudio/best".to_string(),
        _ => {
            let qn = quality.qn();
            format!("bestvideo[quality<={qn}]+bestaudio/best[quality<={qn}]")
        }
    }
}

fn configure_vod_buffering(cmd: &mut Command) {
    // Separated DASH streams need normal buffering. Explicitly restore these
    // values so VOD playback is not affected by mpv's low-latency profile.
    cmd.arg("--audio-buffer=0.2");
    cmd.arg("--vd-lavc-threads=0");
    cmd.arg("--cache=yes");
    cmd.arg("--cache-pause=yes");
    cmd.arg("--demuxer-lavf-o=");
    cmd.arg("--demuxer-lavf-probe-info=auto");
    cmd.arg("--demuxer-lavf-analyzeduration=0");
    cmd.arg("--video-latency-hacks=no");
    cmd.arg("--stream-buffer-size=128KiB");
}

/// Play a video using mpv with yt-dlp and report watch progress
/// This function spawns mpv in a background task to avoid blocking the TUI
#[allow(clippy::too_many_arguments)]
pub async fn play_video(
    api_client: Arc<ApiClient>,
    bvid: &str,
    aid: i64,
    cid: i64,
    duration: i64,
    page_num: Option<i32>,
    credentials: Option<&Credentials>,
    danmaku_config: DanmakuConfig,
    video_quality: VideoQuality,
    playback_event_tx: Sender<PlaybackEvent>,
    session_id: u64,
) -> Result<()> {
    let webpage_url = match page_num {
        Some(p) if p > 1 => format!("https://www.bilibili.com/video/{}?p={}", bvid, p),
        _ => format!("https://www.bilibili.com/video/{}", bvid),
    };

    // Report watch start
    let _ = crate::api::heartbeat::report_watch_start(&api_client, aid, cid, bvid, duration).await;

    let start_ts = chrono::Utc::now().timestamp();

    let mut media_proxy = match api_client.get_play_url(bvid, cid, video_quality).await {
        Ok(play_url) => match crate::api::cdn::rank_streams(&play_url, video_quality).await {
            Ok(streams) => proxy::MediaProxy::start(streams).await.ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };

    let danmaku = api_client.get_video_danmaku(cid).await.unwrap_or_default();
    let ipc_path = vod_mpv_ipc_path(cid);
    remove_stale_mpv_ipc(&ipc_path);
    let danmaku_script_path = create_live_danmaku_script()?;
    let mut session_files = SessionFiles::new(vec![ipc_path.clone(), danmaku_script_path.clone()]);

    let mut cmd = Command::new("mpv");
    cmd.kill_on_drop(true);

    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());

    // Cookies are only needed by yt-dlp; the proxy path passes `--ytdl=no`.
    if let (Some(creds), None) = (credentials, &media_proxy) {
        let cookie_path = crate::storage::export_cookies_for_ytdlp(creds)?;
        cmd.arg(format!(
            "--ytdl-raw-options=cookies={}",
            cookie_path.display()
        ));
        session_files.push(cookie_path);
    }

    cmd.arg("--force-window=immediate");
    configure_vod_buffering(&mut cmd);
    // The TUI owns VOD danmaku rendering through the same OSD script used by
    // live playback. Do not pass CID to global MPV scripts, which would add a
    // second ASS subtitle track.
    cmd.arg(format!("--referrer={webpage_url}"));
    cmd.arg(format!("--http-header-fields=Referer: {webpage_url}"));
    cmd.arg(format!("--input-ipc-server={}", ipc_path.display()));
    cmd.arg(format!("--script={}", danmaku_script_path.display()));
    cmd.arg("--script-opts-append=double_video_fps=no");
    cmd.arg("--msg-level=ffmpeg=error,vd=warn");
    if let Some(proxy) = &media_proxy {
        cmd.arg("--ytdl=no");
        cmd.arg(format!("--audio-file={}", proxy.audio_url));
        cmd.arg(&proxy.video_url);
    } else {
        cmd.arg(format!("--ytdl-format={}", ytdl_format(video_quality)));
        cmd.arg(&webpage_url);
    }

    let mut child = cmd.spawn()?;
    let stderr = child
        .stderr
        .take()
        .map(BufReader::new)
        .map(|reader| reader.lines());

    // Clone bvid for the background task (needs 'static lifetime)
    let bvid = bvid.to_string();

    // Spawn a background task to handle heartbeat and cleanup
    // This prevents blocking the TUI
    tokio::spawn(async move {
        let start_time = Instant::now();
        let mut played_time: i64 = 0;
        let mut heartbeat_interval = interval_at(
            Instant::now() + Duration::from_secs(15),
            Duration::from_secs(15),
        );
        let mut position_interval = interval_at(
            Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        let mut stderr = stderr;
        let mut decode_errors = VecDeque::new();
        let mut last_switch = Instant::now() - Duration::from_secs(10);
        let mut exit_error = None;
        let mut current_cdn_corrupted = false;
        let mut next_danmaku = 0usize;
        let mut last_danmaku_position = 0.0;
        let mut danmaku_interval = tokio::time::interval(Duration::from_millis(20));
        let mut danmaku_ready = false;

        loop {
            tokio::select! {
                _ = heartbeat_interval.tick() => {
                    let real_played_time = start_time.elapsed().as_secs() as i64;

                    let _ = crate::api::heartbeat::report_heartbeat(
                        &api_client,
                        aid,
                        cid,
                        &bvid,
                        played_time,
                        played_time,
                        real_played_time,
                        start_ts,
                        0, // play_type: 0 = playing
                    ).await;
                }
                _ = position_interval.tick() => {
                    if let Some(position) = mpv_time_pos(&ipc_path).await {
                        played_time = position.max(0.0) as i64;
                    }
                }
                // Keep polling after the last comment so seeking back resumes danmaku.
                _ = danmaku_interval.tick(), if !danmaku.is_empty() => {
                    if let Some(position) = mpv_time_pos(&ipc_path).await {
                        if !danmaku_ready {
                            danmaku_ready = send_live_danmaku_config(
                                &ipc_path,
                                &danmaku_script_path,
                                &danmaku_config,
                            ).await.is_ok();
                            if !danmaku_ready {
                                continue;
                            }
                        }
                        let due = due_danmaku_range(
                            &danmaku,
                            next_danmaku,
                            last_danmaku_position,
                            position,
                        );
                        next_danmaku = due.end;
                        last_danmaku_position = position;
                        if !due.is_empty() {
                            let _ = send_video_danmaku_batch(
                                &ipc_path,
                                &danmaku_script_path,
                                &danmaku[due],
                            )
                            .await;
                        }
                    }
                }
                result = child.wait() => {
                    let real_played_time = start_time.elapsed().as_secs() as i64;

                    let _ = crate::api::heartbeat::report_heartbeat(
                        &api_client,
                        aid,
                        cid,
                        &bvid,
                        played_time,
                        played_time,
                        real_played_time,
                        start_ts,
                        4, // play_type: 4 = end
                    ).await;

                    if result.as_ref().is_ok_and(|status| status.success())
                        && !current_cdn_corrupted
                        && let Some(proxy) = &media_proxy
                    {
                        proxy.record_success();
                    }
                    if !result.as_ref().is_ok_and(|status| status.success()) {
                        exit_error = Some(match result {
                            Ok(status) => format!("MPV exited with {status}"),
                            Err(error) => format!("failed waiting for MPV: {error}"),
                        });
                    }
                    break;
                }
                line = async {
                    match &mut stderr {
                        Some(lines) => lines.next_line().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let Ok(Some(line)) = line else { stderr = None; continue };
                    if is_corrupt_video_log(&line) {
                        let now = Instant::now();
                        decode_errors.push_back(now);
                        while decode_errors.front().is_some_and(|seen| now.duration_since(*seen) > Duration::from_secs(3)) {
                            decode_errors.pop_front();
                        }
                    }
                    if decode_errors.len() >= 3 && last_switch.elapsed() > Duration::from_secs(5) {
                        decode_errors.clear();
                        if let Some(proxy) = &mut media_proxy {
                            if !current_cdn_corrupted {
                                proxy.record_current_corruption();
                                current_cdn_corrupted = true;
                            }
                            if let Some((next, video_url)) = proxy.next_video_cdn() {
                            let position = mpv_time_pos(&ipc_path).await.unwrap_or(0.0);
                            let previous_url = proxy.video_url.clone();
                            let switched = replace_mpv_stream(
                                &ipc_path,
                                &video_url,
                                &proxy.audio_url,
                                position,
                            ).await.is_ok() || mpv_path(&ipc_path).await.as_deref() == Some(video_url.as_str());
                            if switched {
                                proxy.commit_video_cdn(next);
                                current_cdn_corrupted = false;
                                last_switch = Instant::now();
                            } else {
                                let _ = replace_mpv_stream(
                                    &ipc_path,
                                    &previous_url,
                                    &proxy.audio_url,
                                    position,
                                ).await;
                            }
                            }
                        }
                    }
                }
            }
        }

        drop(session_files);
        let event = match exit_error {
            Some(error) => PlaybackEvent::Failed { session_id, error },
            None => PlaybackEvent::Finished {
                session_id,
                bvid: Some(bvid),
            },
        };
        let _ = playback_event_tx.send(event);
    });

    Ok(())
}

/// A position change larger than this between two polls is treated as a seek.
const DANMAKU_SEEK_THRESHOLD: f64 = 1.0;

/// Comments to send for `position`, given the cursor `next` and the position
/// of the previous poll. After a seek in either direction the cursor jumps to
/// `position`, so comments that were skipped over are not replayed in bulk.
fn due_danmaku_range(
    danmaku: &[crate::api::danmaku::VideoDanmaku],
    next: usize,
    last_position: f64,
    position: f64,
) -> std::ops::Range<usize> {
    let seeked =
        position + 0.25 < last_position || position > last_position + DANMAKU_SEEK_THRESHOLD;
    let start = if seeked {
        danmaku.partition_point(|message| message.time < position)
    } else {
        next.min(danmaku.len())
    };
    let end = start + danmaku[start..].partition_point(|message| message.time <= position + 0.02);
    start..end
}

async fn send_video_danmaku_batch(
    ipc_path: &std::path::Path,
    script_path: &std::path::Path,
    messages: &[VideoDanmaku],
) -> Result<()> {
    if messages.is_empty() {
        return Ok(());
    }
    let payload = serde_json::to_string(
        &messages
            .iter()
            .map(|message| {
                serde_json::json!({
                    "text": message.text,
                    "color": message.color,
                })
            })
            .collect::<Vec<_>>(),
    )?;
    let script_name = mpv_script_name(script_path);
    mpv_ipc(
        ipc_path,
        serde_json::json!(["script-message-to", script_name, "danmaku-batch", payload]),
    )
    .await
    .map(|_| ())
}

fn is_corrupt_video_log(line: &str) -> bool {
    [
        "Invalid NAL unit size",
        "Error splitting the input into NAL units",
        "Error while decoding frame",
    ]
    .iter()
    .any(|needle| line.contains(needle))
}

fn create_live_danmaku_script() -> Result<std::path::PathBuf> {
    let script_path = std::env::temp_dir().join(format!(
        "bilibili-tui-danmaku-{}-{}.lua",
        std::process::id(),
        MPV_IPC_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&script_path)?;
    StdWrite::write_all(&mut file, LIVE_DANMAKU_SCRIPT.as_bytes())?;
    Ok(script_path)
}

fn live_danmaku_value(message: &LiveMessage) -> Option<serde_json::Value> {
    let LiveMessage::Danmaku { content, color, .. } = message else {
        return None;
    };

    Some(serde_json::json!({
        "text": content,
        "color": color,
    }))
}

#[cfg(test)]
fn live_danmaku_payload(message: &LiveMessage) -> Option<String> {
    serde_json::to_string(&live_danmaku_value(message)?).ok()
}

async fn send_live_danmaku_batch(
    ipc_path: &std::path::Path,
    script_path: &std::path::Path,
    messages: &[LiveMessage],
) -> Result<()> {
    let payloads: Vec<_> = messages.iter().filter_map(live_danmaku_value).collect();
    if payloads.is_empty() {
        return Ok(());
    }
    let payload = serde_json::to_string(&payloads)?;
    let script_name = mpv_script_name(script_path);
    mpv_ipc(
        ipc_path,
        serde_json::json!(["script-message-to", &script_name, "danmaku-batch", payload]),
    )
    .await
    .map(|_| ())
}

async fn send_live_danmaku_config(
    ipc_path: &std::path::Path,
    script_path: &std::path::Path,
    config: &DanmakuConfig,
) -> Result<()> {
    let payload = serde_json::to_string(config)?;
    let script_name = mpv_script_name(script_path);
    mpv_ipc(
        ipc_path,
        serde_json::json!(["script-message-to", &script_name, "danmaku-config", payload]),
    )
    .await
    .map(|_| ())
}

fn mpv_script_name(script_path: &std::path::Path) -> String {
    let script_stem = script_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("live_danmaku");
    // MPV normalizes script filenames to identifiers before using them as
    // `script-message-to` targets (for example, '-' and '.' become '_').
    script_stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn mpv_ipc_path(prefix: &str, suffix: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::path::PathBuf::from(format!(
            r"\\.\pipe\{prefix}-{}-{suffix}",
            std::process::id()
        ))
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join(format!("{prefix}-{}-{suffix}.sock", std::process::id()))
    }
}

/// Temporary files owned by one player session. They are removed when the
/// guard drops, including when the session task is dropped at app exit or an
/// early return aborts setup.
struct SessionFiles(Vec<std::path::PathBuf>);

impl SessionFiles {
    fn new(paths: Vec<std::path::PathBuf>) -> Self {
        Self(paths)
    }

    fn push(&mut self, path: std::path::PathBuf) {
        self.0.push(path);
    }
}

impl Drop for SessionFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Each VOD session gets its own socket so replaying the same CID while an
/// earlier mpv is still open cannot unlink or hijack that player's socket.
fn vod_mpv_ipc_path(cid: i64) -> std::path::PathBuf {
    let sequence = MPV_IPC_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    mpv_ipc_path("bilibili-tui-mpv", &format!("{cid}-{sequence}"))
}

fn remove_stale_mpv_ipc(path: &std::path::Path) {
    #[cfg(not(windows))]
    let _ = std::fs::remove_file(path);

    #[cfg(windows)]
    let _ = path;
}

async fn connect_mpv_ipc(path: &std::path::Path) -> Result<MpvIpcStream> {
    #[cfg(unix)]
    {
        Ok(MpvIpcStream::connect(path).await?)
    }
    #[cfg(windows)]
    {
        Ok(MpvIpcClientOptions::new().open(path)?)
    }
}

async fn mpv_ipc(path: &std::path::Path, command: serde_json::Value) -> Result<serde_json::Value> {
    timeout(Duration::from_secs(2), async {
        let mut stream = connect_mpv_ipc(path).await?;
        let request_id = MPV_IPC_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "command": command,
            "request_id": request_id,
        }))?;
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                anyhow::bail!("MPV IPC closed before command response");
            }
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if value.get("request_id").and_then(|value| value.as_u64()) != Some(request_id) {
                continue;
            }
            ensure_mpv_success(&value)?;
            return Ok(value);
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("MPV IPC timed out"))?
}

fn ensure_mpv_success(value: &serde_json::Value) -> Result<()> {
    match value.get("error").and_then(|value| value.as_str()) {
        Some("success") => Ok(()),
        Some(error) => Err(anyhow::anyhow!("MPV IPC error: {error}")),
        None => Err(anyhow::anyhow!("MPV IPC response has no status")),
    }
}

/// File lifecycle events from the mpv IPC stream. `entry` is mpv's
/// `playlist_entry_id`, absent on old mpv versions.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MpvFileEvent {
    Started { entry: Option<i64> },
    Loaded,
    Ended { entry: Option<i64>, reason: String },
}

fn mpv_file_event(value: &serde_json::Value) -> Option<MpvFileEvent> {
    let entry = value
        .get("playlist_entry_id")
        .and_then(serde_json::Value::as_i64);
    match value.get("event")?.as_str()? {
        "start-file" => Some(MpvFileEvent::Started { entry }),
        "file-loaded" => Some(MpvFileEvent::Loaded),
        "end-file" => Some(MpvFileEvent::Ended {
            entry,
            reason: value
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
        }),
        _ => None,
    }
}

/// Whether an `end-file` belongs to the file currently playing. Events for an
/// entry replaced since then are stale. Without ids every event is accepted.
fn is_current_entry(current: Option<i64>, ended: Option<i64>) -> bool {
    match (current, ended) {
        (Some(current), Some(ended)) => current == ended,
        _ => true,
    }
}

/// Tracks one `loadfile` until the new file has loaded or failed.
#[derive(Default)]
struct LoadProgress {
    command_ok: bool,
    started: bool,
    entry: Option<i64>,
    loaded: bool,
}

impl LoadProgress {
    /// Apply one file event. The replaced file ends before the new one
    /// starts, so only an `end-file: error` after `start-file` is a failure.
    fn apply(&mut self, event: MpvFileEvent) -> Result<()> {
        match event {
            MpvFileEvent::Started { entry } => {
                self.started = true;
                self.entry = entry;
            }
            MpvFileEvent::Loaded => self.loaded = true,
            MpvFileEvent::Ended { entry, reason }
                if reason == "error" && self.started && is_current_entry(self.entry, entry) =>
            {
                anyhow::bail!("MPV failed to load the stream")
            }
            MpvFileEvent::Ended { .. } => {}
        }
        Ok(())
    }

    fn done(&self) -> bool {
        self.command_ok && self.loaded
    }
}

/// Replace the current file and wait until it has loaded. Returns the new
/// `playlist_entry_id` when mpv reports one.
async fn loadfile_and_wait(
    path: &std::path::Path,
    video_url: &str,
    audio_url: &str,
    position: f64,
) -> Result<Option<i64>> {
    timeout(Duration::from_secs(10), async {
        if audio_url.contains(',') {
            anyhow::bail!("audio CDN URL cannot be represented as an MPV option list");
        }
        let mut stream = connect_mpv_ipc(path).await?;
        let request_id = 1u64;
        let load_options = format!("audio-files={audio_url},start={position}");
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "command": [
                "loadfile",
                video_url,
                "replace",
                -1,
                load_options
            ],
            "request_id": request_id,
        }))?;
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        let mut lines = BufReader::new(stream).lines();
        let mut progress = LoadProgress::default();
        while !progress.done() {
            let line = lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow::anyhow!("MPV IPC closed"))?;
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if value.get("request_id").and_then(|value| value.as_u64()) == Some(request_id) {
                ensure_mpv_success(&value)?;
                progress.command_ok = true;
            }
            if let Some(event) = mpv_file_event(&value) {
                progress.apply(event)?;
            }
        }
        Ok(progress.entry)
    })
    .await
    .map_err(|_| anyhow::anyhow!("MPV file load timed out"))?
}

async fn mpv_time_pos(path: &std::path::Path) -> Option<f64> {
    mpv_ipc(path, serde_json::json!(["get_property", "time-pos"]))
        .await
        .ok()?
        .get("data")?
        .as_f64()
}

async fn load_live_and_wait(path: &std::path::Path, url: &str) -> Result<()> {
    timeout(Duration::from_secs(10), async {
        let mut stream = connect_mpv_ipc(path).await?;
        let request_id = 2u64;
        let mut request = serde_json::to_vec(&serde_json::json!({
            "command": ["loadfile", url, "replace"],
            "request_id": request_id,
        }))?;
        request.push(b'\n');
        stream.write_all(&request).await?;
        let mut lines = BufReader::new(stream).lines();
        let mut command_ok = false;
        let mut loaded = false;
        loop {
            let line = lines
                .next_line()
                .await?
                .ok_or_else(|| anyhow::anyhow!("MPV IPC closed while loading live stream"))?;
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if value.get("request_id").and_then(|value| value.as_u64()) == Some(request_id) {
                ensure_mpv_success(&value)?;
                command_ok = true;
            }
            match value.get("event").and_then(|value| value.as_str()) {
                Some("file-loaded") => loaded = true,
                Some("end-file")
                    if value.get("reason").and_then(|value| value.as_str()) == Some("error") =>
                {
                    anyhow::bail!("MPV failed to load live stream")
                }
                _ => {}
            }
            if command_ok && loaded {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("MPV live stream load timed out"))?
}

async fn mpv_path(path: &std::path::Path) -> Option<String> {
    mpv_ipc(path, serde_json::json!(["get_property", "path"]))
        .await
        .ok()?
        .get("data")?
        .as_str()
        .map(str::to_owned)
}

async fn replace_mpv_stream(
    path: &std::path::Path,
    video_url: &str,
    audio_url: &str,
    position: f64,
) -> Result<Option<i64>> {
    loadfile_and_wait(path, video_url, audio_url, position).await
}

/// Try the remaining video CDNs in order. `current_entry` follows the file
/// mpv ends up playing, so `end-file` events of replaced files are ignored.
async fn switch_to_working_video_cdn(
    prepared: &mut PreparedPlaylistItem,
    ipc_path: &std::path::Path,
    position: f64,
    current_entry: &mut Option<i64>,
) -> bool {
    let previous = prepared.proxy.video_url.clone();
    let mut candidate = prepared.proxy.next_video_cdn();
    while let Some((index, url)) = candidate {
        match replace_mpv_stream(ipc_path, &url, &prepared.proxy.audio_url, position).await {
            Ok(entry) => {
                *current_entry = entry;
                return prepared.proxy.commit_video_cdn(index);
            }
            Err(_) if mpv_path(ipc_path).await.as_deref() == Some(url.as_str()) => {
                *current_entry = None;
                return prepared.proxy.commit_video_cdn(index);
            }
            Err(_) => candidate = prepared.proxy.video_cdn(index + 1),
        }
    }
    *current_entry = replace_mpv_stream(ipc_path, &previous, &prepared.proxy.audio_url, position)
        .await
        .ok()
        .flatten();
    false
}

/// Start one mpv process with multiple Bilibili URLs. mpv owns automatic
/// advancement, so window/fullscreen/volume state is preserved between items.
#[allow(clippy::too_many_arguments)]
pub async fn play_playlist(
    api_client: Arc<ApiClient>,
    items: Vec<PlaylistItem>,
    order: PlayOrder,
    start_index: usize,
    _credentials: Option<&Credentials>,
    video_quality: VideoQuality,
    playback_event_tx: Sender<PlaybackEvent>,
    session_id: u64,
) -> Result<()> {
    let (items, requested_start) = ordered_playlist(items, order, start_index)?;
    let (first, skipped) =
        prepare_next_playlist_item(&api_client, &items, requested_start, video_quality).await;
    let Some((start_index, first)) = first else {
        anyhow::bail!("播放列表没有可播放项目: {}", skipped.join("; "));
    };
    log_skipped_playlist_items(&skipped);

    let mut cmd = Command::new("mpv");
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());
    cmd.arg("--idle=yes");
    cmd.arg("--force-window=immediate");
    cmd.arg("--msg-level=ffmpeg=error,vd=warn");
    cmd.arg("--ytdl=no");
    cmd.arg("--script-opts-append=double_video_fps=yes");
    let ipc_path = mpv_ipc_path("bilibili-tui-playlist", &session_id.to_string());
    remove_stale_mpv_ipc(&ipc_path);
    cmd.arg(format!("--input-ipc-server={}", ipc_path.display()));

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return Err(error.into()),
    };
    let stderr = child
        .stderr
        .take()
        .map(BufReader::new)
        .map(|reader| reader.lines());
    tokio::spawn(async move {
        let result = run_playlist(
            &mut child,
            stderr,
            &ipc_path,
            api_client,
            items,
            start_index,
            first,
            video_quality,
            playback_event_tx.clone(),
            session_id,
        )
        .await;
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let _ = tokio::fs::remove_file(&ipc_path).await;
        let event = match result {
            Ok(()) => PlaybackEvent::Finished {
                session_id,
                bvid: None,
            },
            Err(error) => PlaybackEvent::Failed {
                session_id,
                error: crate::api::redact::redact_url_queries(&format!("播放列表失败: {error:#}")),
            },
        };
        let _ = playback_event_tx.send(event);
    });
    Ok(())
}

struct PreparedPlaylistItem {
    cid: i64,
    duration: i64,
    proxy: proxy::MediaProxy,
}

async fn prepare_playlist_item(
    api_client: &ApiClient,
    item: &PlaylistItem,
    video_quality: VideoQuality,
) -> Result<PreparedPlaylistItem> {
    let (cid, duration) = match (item.cid, item.duration) {
        (Some(cid), Some(duration)) => (cid, duration),
        _ => {
            let info = api_client.get_video_info(&item.bvid).await?;
            (
                item.cid.unwrap_or(info.cid),
                item.duration.or(info.duration).unwrap_or_default(),
            )
        }
    };
    let play_url = api_client
        .get_play_url(&item.bvid, cid, video_quality)
        .await?;
    let streams = crate::api::cdn::rank_streams(&play_url, video_quality).await?;
    let proxy = proxy::MediaProxy::start(streams).await?;
    Ok(PreparedPlaylistItem {
        cid,
        duration,
        proxy,
    })
}

async fn prepare_next_playlist_item(
    api_client: &ApiClient,
    items: &[PlaylistItem],
    start: usize,
    video_quality: VideoQuality,
) -> (Option<(usize, PreparedPlaylistItem)>, Vec<String>) {
    let mut failures = Vec::new();
    for (index, item) in items.iter().enumerate().skip(start) {
        if let Some(prepared) = accept_prepared_result(
            item,
            prepare_playlist_item(api_client, item, video_quality).await,
            &mut failures,
        ) {
            return (Some((index, prepared)), failures);
        }
    }
    (None, failures)
}

fn accept_prepared_result<T>(
    item: &PlaylistItem,
    result: Result<T>,
    failures: &mut Vec<String>,
) -> Option<T> {
    match result {
        Ok(prepared) => Some(prepared),
        Err(error) => {
            failures.push(format!("{}: {error:#}", item.bvid));
            None
        }
    }
}

fn log_skipped_playlist_items(failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    write_playback_diagnostic(&format!("playlist skipped items: {}", failures.join("; ")));
}

fn write_playback_diagnostic(message: &str) {
    let message = crate::api::redact::redact_url_queries(message);
    let Some(mut dir) = dirs::config_dir() else {
        return;
    };
    dir.push("bilibili-tui");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("playback.log");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut log) = options.open(path) {
        use std::io::Write;
        let _ = writeln!(
            log,
            "[{}] {message}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_playlist(
    child: &mut tokio::process::Child,
    stderr: Option<tokio::io::Lines<BufReader<tokio::process::ChildStderr>>>,
    ipc_path: &std::path::Path,
    api_client: Arc<ApiClient>,
    items: Vec<PlaylistItem>,
    mut index: usize,
    mut prepared: PreparedPlaylistItem,
    video_quality: VideoQuality,
    tx: Sender<PlaybackEvent>,
    session_id: u64,
) -> Result<()> {
    wait_for_ipc(ipc_path, child).await?;
    let (end_tx, mut end_rx) = tokio::sync::mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let event_path = ipc_path.to_owned();
    let event_task =
        tokio::spawn(async move { observe_end_files(&event_path, end_tx, ready_tx).await });
    timeout(Duration::from_secs(2), ready_rx)
        .await
        .map_err(|_| anyhow::anyhow!("MPV event observer timed out"))?
        .map_err(|_| anyhow::anyhow!("MPV event observer failed to start"))?;
    let mut stderr = stderr;
    let mut decode_errors = VecDeque::new();
    let mut last_switch = Instant::now() - Duration::from_secs(10);
    let mut corrupted = false;
    let mut played_time = 0i64;
    let mut item_started = Instant::now();
    let mut start_ts = chrono::Utc::now().timestamp();
    let mut played_any = false;
    let mut current_entry = start_playlist_media(ipc_path, &items[index], &prepared).await?;
    let _ = tx.send(PlaybackEvent::ItemChanged {
        session_id,
        index,
        bvid: items[index].bvid.clone(),
    });
    let _ = crate::api::heartbeat::report_watch_start(
        &api_client,
        items[index].aid,
        prepared.cid,
        &items[index].bvid,
        prepared.duration,
    )
    .await;
    let mut heartbeat = interval_at(
        Instant::now() + Duration::from_secs(15),
        Duration::from_secs(15),
    );
    let mut position = interval_at(
        Instant::now() + Duration::from_secs(1),
        Duration::from_secs(1),
    );

    let result = loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                report_playlist_heartbeat(&api_client, &items[index], &prepared, played_time, item_started, start_ts, 0).await;
            }
            _ = position.tick() => {
                if let Some(value) = mpv_time_pos(ipc_path).await { played_time = value.max(0.0) as i64; }
            }
            status = child.wait() => {
                report_playlist_heartbeat(
                    &api_client,
                    &items[index],
                    &prepared,
                    played_time,
                    item_started,
                    start_ts,
                    4,
                ).await;
                if !corrupted && status.as_ref().is_ok_and(|status| status.success()) {
                    prepared.proxy.record_success();
                }
                break match status {
                    Ok(status) if status.success() => Ok(()),
                    Ok(status) => Err(anyhow::anyhow!("MPV exited with {status}")),
                    Err(error) => Err(error.into()),
                };
            }
            ended = end_rx.recv() => {
                let Some((entry, reason)) = ended else { break Err(anyhow::anyhow!("MPV IPC event stream closed")); };
                if !is_current_entry(current_entry, entry) {
                    continue;
                }
                if reason == "error" {
                    if !corrupted {
                        prepared.proxy.record_current_corruption();
                        corrupted = true;
                    }
                    let current_pos = mpv_time_pos(ipc_path).await.unwrap_or(played_time as f64);
                    if switch_to_working_video_cdn(&mut prepared, ipc_path, current_pos, &mut current_entry).await {
                        corrupted = false;
                        last_switch = Instant::now();
                        continue;
                    }
                    write_playback_diagnostic(&format!("playlist exhausted CDN candidates for {}", items[index].bvid));
                } else if reason != "eof" {
                    continue;
                }
                report_playlist_heartbeat(&api_client, &items[index], &prepared, played_time, item_started, start_ts, 4).await;
                if reason == "eof" {
                    played_any = true;
                    if !corrupted { prepared.proxy.record_success(); }
                }
                let (next, skipped) = prepare_next_playlist_item(
                    &api_client,
                    &items,
                    index + 1,
                    video_quality,
                ).await;
                log_skipped_playlist_items(&skipped);
                let Some((next_index, next_prepared)) = next else {
                    let _ = mpv_ipc(ipc_path, serde_json::json!(["quit"])).await;
                    break match child.wait().await {
                        Ok(status) if status.success() && played_any => Ok(()),
                        Ok(status) if status.success() => Err(anyhow::anyhow!("播放列表所有项目均无法播放")),
                        Ok(status) => Err(anyhow::anyhow!("MPV exited with {status}")),
                        Err(error) => Err(error.into()),
                    };
                };
                index = next_index;
                prepared = next_prepared;
                current_entry = start_playlist_media(ipc_path, &items[index], &prepared).await?;
                played_time = 0;
                corrupted = false;
                item_started = Instant::now();
                start_ts = chrono::Utc::now().timestamp();
                let _ = tx.send(PlaybackEvent::ItemChanged { session_id, index, bvid: items[index].bvid.clone() });
                let _ = crate::api::heartbeat::report_watch_start(
                    &api_client, items[index].aid, prepared.cid, &items[index].bvid, prepared.duration,
                ).await;
            }
            line = async {
                match &mut stderr {
                    Some(lines) => lines.next_line().await,
                    None => std::future::pending().await,
                }
            } => {
                let Ok(Some(line)) = line else { stderr = None; continue };
                if is_corrupt_video_log(&line) {
                    let now = Instant::now();
                    decode_errors.push_back(now);
                    while decode_errors.front().is_some_and(|seen| now.duration_since(*seen) > Duration::from_secs(3)) { decode_errors.pop_front(); }
                }
                if decode_errors.len() >= 3 && last_switch.elapsed() > Duration::from_secs(5) {
                    decode_errors.clear();
                    if !corrupted { prepared.proxy.record_current_corruption(); corrupted = true; }
                    let current_pos = mpv_time_pos(ipc_path).await.unwrap_or(played_time as f64);
                    if switch_to_working_video_cdn(&mut prepared, ipc_path, current_pos, &mut current_entry).await {
                        corrupted = false;
                        last_switch = Instant::now();
                    }
                }
            }
        }
    };
    event_task.abort();
    result
}

async fn wait_for_ipc(path: &std::path::Path, child: &mut tokio::process::Child) -> Result<()> {
    let mut delay = Duration::from_millis(25);
    loop {
        if connect_mpv_ipc(path).await.is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("MPV exited before IPC was ready: {status}");
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

async fn start_playlist_media(
    path: &std::path::Path,
    item: &PlaylistItem,
    prepared: &PreparedPlaylistItem,
) -> Result<Option<i64>> {
    let page = match item.page {
        Some(page) if page > 1 => format!("https://www.bilibili.com/video/{}?p={page}", item.bvid),
        _ => format!("https://www.bilibili.com/video/{}", item.bvid),
    };
    let _ = mpv_ipc(path, serde_json::json!(["set_property", "referrer", page])).await;
    // Append so options the user set in mpv.conf survive; only `cid` changes.
    let _ = mpv_ipc(path, playlist_cid_option(prepared.cid)).await;
    loadfile_and_wait(
        path,
        &prepared.proxy.video_url,
        &prepared.proxy.audio_url,
        0.0,
    )
    .await
}

fn playlist_cid_option(cid: i64) -> serde_json::Value {
    serde_json::json!(["change-list", "script-opts", "append", format!("cid={cid}")])
}

async fn report_playlist_heartbeat(
    api_client: &ApiClient,
    item: &PlaylistItem,
    prepared: &PreparedPlaylistItem,
    played: i64,
    started: Instant,
    start_ts: i64,
    play_type: i32,
) {
    let _ = crate::api::heartbeat::report_heartbeat(
        api_client,
        item.aid,
        prepared.cid,
        &item.bvid,
        played,
        played,
        started.elapsed().as_secs() as i64,
        start_ts,
        play_type,
    )
    .await;
}

async fn observe_end_files(
    path: &std::path::Path,
    tx: tokio::sync::mpsc::UnboundedSender<(Option<i64>, String)>,
    ready: tokio::sync::oneshot::Sender<()>,
) -> Result<()> {
    let mut stream = connect_mpv_ipc(path).await?;
    let mut request =
        serde_json::to_vec(&serde_json::json!({"command": ["request_log_messages", "no"]}))?;
    request.push(b'\n');
    stream.write_all(&request).await?;
    let _ = ready.send(());
    let mut lines = BufReader::new(stream).lines();
    while let Some(line) = lines.next_line().await? {
        let value: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(MpvFileEvent::Ended { entry, reason }) = mpv_file_event(&value) {
            let _ = tx.send((entry, reason));
        }
    }
    Ok(())
}

fn ordered_playlist(
    mut items: Vec<PlaylistItem>,
    order: PlayOrder,
    mut start_index: usize,
) -> Result<(Vec<PlaylistItem>, usize)> {
    if items.is_empty() {
        anyhow::bail!("播放列表为空");
    }
    if start_index >= items.len() {
        anyhow::bail!("播放起点超出列表范围");
    }
    match order {
        PlayOrder::Forward => {}
        PlayOrder::Reverse => {
            start_index = items.len() - 1 - start_index;
            items.reverse();
        }
        PlayOrder::Shuffle => {
            items.swap(0, start_index);
            let mut state = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64
                ^ items.len() as u64;
            for index in (2..items.len()).rev() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                items.swap(index, 1 + state as usize % index);
            }
            start_index = 0;
        }
    }
    Ok((items, start_index))
}

/// Play an authenticated bangumi episode, falling back to yt-dlp when the
/// direct PGC stream cannot be proxied.
pub async fn play_bangumi_episode(
    api_client: Arc<ApiClient>,
    ep_id: i64,
    credentials: Option<&Credentials>,
    video_quality: VideoQuality,
    playback_event_tx: Sender<PlaybackEvent>,
    session_id: u64,
) -> Result<()> {
    let video_url = format!("https://www.bilibili.com/bangumi/play/ep{}", ep_id);
    let media_proxy = match api_client.get_bangumi_play_url(ep_id, video_quality).await {
        Ok(play_url) => match crate::api::cdn::rank_streams(&play_url, video_quality).await {
            Ok(streams) => proxy::MediaProxy::start(streams).await.ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };

    let mut cmd = Command::new("mpv");
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    // Cookies are only needed by yt-dlp; the proxy path passes `--ytdl=no`.
    let mut session_files = SessionFiles::new(Vec::new());
    if let (Some(creds), None) = (credentials, &media_proxy) {
        let cookie_path = crate::storage::export_cookies_for_ytdlp(creds)?;
        cmd.arg(format!(
            "--ytdl-raw-options=cookies={}",
            cookie_path.display()
        ));
        session_files.push(cookie_path);
    }

    cmd.arg("--force-window=immediate");
    cmd.arg("--script-opts-append=double_video_fps=yes");
    cmd.arg(format!("--referrer={video_url}"));
    cmd.arg(format!("--http-header-fields=Referer: {video_url}"));
    if let Some(proxy) = &media_proxy {
        cmd.arg("--ytdl=no");
        cmd.arg(format!("--audio-file={}", proxy.audio_url));
        cmd.arg(&proxy.video_url);
    } else {
        cmd.arg(format!("--ytdl-format={}", ytdl_format(video_quality)));
        cmd.arg(&video_url);
    }

    let mut child = cmd.spawn()?;

    tokio::spawn(async move {
        let status = child.wait().await;

        drop(session_files);
        drop(media_proxy);
        let event = match status {
            Ok(status) if status.success() => PlaybackEvent::Finished {
                session_id,
                bvid: None,
            },
            Ok(status) => PlaybackEvent::Failed {
                session_id,
                error: format!("番剧播放器退出: {status}"),
            },
            Err(error) => PlaybackEvent::Failed {
                session_id,
                error: crate::api::redact::redact_url_queries(&format!("番剧播放器失败: {error}")),
            },
        };
        let _ = playback_event_tx.send(event);
    });

    Ok(())
}

/// Play a live stream using mpv
/// This function spawns mpv in a background task to avoid blocking the TUI
pub async fn play_live(
    api_client: Arc<ApiClient>,
    room_id: i64,
    danmaku_hub: Option<Arc<LiveDanmakuHub>>,
    mut danmaku_config_rx: tokio::sync::watch::Receiver<DanmakuConfig>,
    video_quality: VideoQuality,
) -> Result<()> {
    let mut urls = api_client
        .get_live_stream_urls(room_id, video_quality)
        .await?;
    let first_url = urls
        .first()
        .ok_or_else(|| anyhow::anyhow!("直播播放地址为空"))?;
    let sequence = LIVE_SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let ipc_path = mpv_ipc_path("bilibili-tui-live", &format!("{room_id}-{sequence}"));
    remove_stale_mpv_ipc(&ipc_path);
    let danmaku_script_path = create_live_danmaku_script()?;
    let mut child = match spawn_live_mpv(&ipc_path, &danmaku_script_path) {
        Ok(child) => child,
        Err(error) => {
            let _ = tokio::fs::remove_file(&danmaku_script_path).await;
            return Err(error);
        }
    };
    if let Err(error) = wait_for_ipc(&ipc_path, &mut child).await {
        shutdown_live_child(&ipc_path, &mut child, Duration::from_secs(1)).await;
        let _ = tokio::fs::remove_file(&ipc_path).await;
        let _ = tokio::fs::remove_file(&danmaku_script_path).await;
        return Err(error);
    }
    let initial_danmaku_config = danmaku_config_rx.borrow_and_update().clone();
    if let Err(error) =
        send_live_danmaku_config(&ipc_path, &danmaku_script_path, &initial_danmaku_config).await
    {
        write_live_diagnostic(room_id, &format!("live danmaku config IPC error: {error}"));
    }

    let (end_tx, mut end_rx) = tokio::sync::mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let observer_path = ipc_path.clone();
    let observer = tokio::spawn(async move {
        let _ = observe_end_files(&observer_path, end_tx, ready_tx).await;
    });
    if !matches!(timeout(Duration::from_secs(2), ready_rx).await, Ok(Ok(()))) {
        observer.abort();
        shutdown_live_child(&ipc_path, &mut child, Duration::from_secs(1)).await;
        let _ = tokio::fs::remove_file(&ipc_path).await;
        let _ = tokio::fs::remove_file(&danmaku_script_path).await;
        anyhow::bail!("直播 MPV 事件监听启动超时");
    }
    if let Err(error) = load_live_and_wait(&ipc_path, first_url).await {
        observer.abort();
        shutdown_live_child(&ipc_path, &mut child, Duration::from_secs(1)).await;
        let _ = tokio::fs::remove_file(&ipc_path).await;
        let _ = tokio::fs::remove_file(&danmaku_script_path).await;
        return Err(error);
    }

    let mut danmaku_rx = danmaku_hub.as_ref().map(|hub| hub.subscribe());
    tokio::spawn(async move {
        // Keep the shared connection alive even if the application switches to
        // another room while this MPV instance is still playing.
        let _danmaku_hub = danmaku_hub;
        let mut next_url = 1usize;
        let mut consecutive_failures = 0usize;
        let mut loaded_at = Instant::now();
        let mut danmaku_config_open = true;
        let mut danmaku_flush = tokio::time::interval(Duration::from_millis(16));
        let mut pending_danmaku = Vec::new();
        'playback: loop {
            tokio::select! {
                status = child.wait() => {
                    write_live_diagnostic(room_id, &format!("MPV exited: {}", status.map(|value| value.to_string()).unwrap_or_else(|error| error.to_string())));
                    break;
                }
                reason = end_rx.recv() => {
                    let Some((_, reason)) = reason else {
                        write_live_diagnostic(room_id, "MPV event observer stopped");
                        break;
                    };
                    if !should_reload_live_reason(&reason) {
                        write_live_diagnostic(room_id, &format!("ignored end-file reason ({reason})"));
                        continue;
                    }
                    if loaded_at.elapsed() >= Duration::from_secs(30) {
                        consecutive_failures = 0;
                    }
                    pending_danmaku.clear();
                    write_live_diagnostic(room_id, &format!("stream ended ({reason}); starting failover"));
                    loop {
                        consecutive_failures += 1;
                        if consecutive_failures > 8 {
                            write_live_diagnostic(room_id, "live failover exhausted after 8 attempts");
                            let _ = mpv_ipc(&ipc_path, serde_json::json!(["quit"])).await;
                            break;
                        }
                        if live_or_exit(&mut child, tokio::time::sleep(live_retry_delay(consecutive_failures))).await.is_none() {
                            write_live_diagnostic(room_id, "MPV exited during failover backoff");
                            break 'playback;
                        }
                        if next_url >= urls.len() {
                            let Some(refreshed) = live_or_exit(
                                &mut child,
                                api_client.get_live_stream_urls(room_id, video_quality),
                            ).await else {
                                write_live_diagnostic(room_id, "MPV exited during live URL refresh");
                                break 'playback;
                            };
                            match refreshed {
                                Ok(refreshed) if !refreshed.is_empty() => {
                                    urls = refreshed;
                                    next_url = 0;
                                    write_live_diagnostic(room_id, "refreshed expired live URLs");
                                }
                                Ok(_) => {
                                    write_live_diagnostic(room_id, "live URL refresh returned no streams");
                                    continue;
                                }
                                Err(error) => {
                                    write_live_diagnostic(room_id, &format!("live URL refresh failed: {error}"));
                                    continue;
                                }
                            }
                        }
                        let url = &urls[next_url];
                        next_url += 1;
                        let Some(load_result) = live_or_exit(
                            &mut child,
                            load_live_and_wait(&ipc_path, url),
                        ).await else {
                            write_live_diagnostic(room_id, "MPV exited during stream reload");
                            break 'playback;
                        };
                        match load_result {
                            Ok(_) => {
                                loaded_at = Instant::now();
                                write_live_diagnostic(room_id, &format!("loaded failover stream (attempt {consecutive_failures})"));
                                break;
                            }
                            Err(error) => {
                                let _ = timeout(Duration::from_millis(100), end_rx.recv()).await;
                                while end_rx.try_recv().is_ok() {}
                                write_live_diagnostic(room_id, &format!("MPV failover command failed: {error}"));
                            }
                        }
                    }
                    if consecutive_failures > 8 {
                        let _ = child.wait().await;
                        break;
                    }
                }
                _ = danmaku_flush.tick() => {
                    if !pending_danmaku.is_empty() {
                        let batch = std::mem::take(&mut pending_danmaku);
                        if let Err(error) =
                            send_live_danmaku_batch(&ipc_path, &danmaku_script_path, &batch).await
                        {
                            write_live_diagnostic(room_id, &format!("live danmaku IPC error: {error}"));
                        }
                    }
                }
                message = async {
                    danmaku_rx.as_mut().expect("guarded danmaku receiver").recv().await
                }, if danmaku_rx.is_some() => {
                    match message {
                        Ok(message) if matches!(message, LiveMessage::Danmaku { .. }) => {
                            pending_danmaku.push(message);
                        }
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            danmaku_rx = None;
                            write_live_diagnostic(room_id, "live danmaku hub closed");
                        }
                    }
                }
                changed = danmaku_config_rx.changed(), if danmaku_config_open => {
                    if changed.is_ok() {
                        let config = danmaku_config_rx.borrow_and_update().clone();
                        if let Err(error) =
                            send_live_danmaku_config(&ipc_path, &danmaku_script_path, &config).await
                        {
                            write_live_diagnostic(room_id, &format!("live danmaku config IPC error: {error}"));
                        }
                    } else {
                        danmaku_config_open = false;
                    }
                }
            }
        }
        shutdown_live_child(&ipc_path, &mut child, Duration::from_secs(2)).await;
        observer.abort();
        let _ = tokio::fs::remove_file(ipc_path).await;
        let _ = tokio::fs::remove_file(danmaku_script_path).await;
    });

    Ok(())
}

async fn live_or_exit<F: std::future::Future>(
    child: &mut tokio::process::Child,
    future: F,
) -> Option<F::Output> {
    tokio::select! {
        _ = child.wait() => None,
        output = future => Some(output),
    }
}

async fn shutdown_live_child(
    ipc_path: &std::path::Path,
    child: &mut tokio::process::Child,
    grace: Duration,
) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = mpv_ipc(ipc_path, serde_json::json!(["quit"])).await;
    if timeout(grace, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

fn spawn_live_mpv(
    ipc_path: &std::path::Path,
    danmaku_script_path: &std::path::Path,
) -> Result<tokio::process::Child> {
    let mut cmd = Command::new("mpv");
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    configure_live_mpv(&mut cmd, ipc_path);
    cmd.arg(format!("--script={}", danmaku_script_path.display()));
    Ok(cmd.spawn()?)
}

fn configure_live_mpv(cmd: &mut Command, ipc_path: &std::path::Path) {
    cmd.arg("--idle=yes");
    cmd.arg("--force-window=immediate");
    cmd.arg("--profile=low-latency");
    cmd.arg("--keep-open=yes");
    // A live HLS window must always start at its live edge. Inheriting the
    // user's watch-later state resumes near the end of a finite playlist and
    // produces an EOF loop a few seconds later.
    cmd.arg("--save-position-on-quit=no");
    cmd.arg("--resume-playback=no");
    // A user-level mpv.conf may enable a verbose global log, which would
    // expose short-lived signed stream URLs. Live failover diagnostics are
    // written separately without URLs by write_live_diagnostic().
    cmd.arg("--log-file=");
    cmd.arg(format!("--input-ipc-server={}", ipc_path.display()));
    cmd.arg("--script-opts-append=double_video_fps=no");
    cmd.arg("--referrer=https://live.bilibili.com/");
    cmd.arg("--cache=yes");
    cmd.arg("--cache-secs=3600");
    cmd.arg("--demuxer-readahead-secs=3600");
    cmd.arg("--network-timeout=10");
    cmd.arg("--stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=5");
    cmd.arg("--hwdec=auto-safe");
    #[cfg(target_os = "macos")]
    if let Some(font_dir) = macos_live_font_dir() {
        // Load the selected public Chinese font asset directly and disable
        // CoreText fallback to inaccessible Reserved font files.
        cmd.arg("--sub-font-provider=none");
        cmd.arg(format!("--sub-fonts-dir={}", font_dir.display()));
    }
}

#[cfg(target_os = "macos")]
fn macos_live_font_dir() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{file}\n", "Yuanti SC"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let file = String::from_utf8(output.stdout).ok()?;
    std::path::Path::new(file.lines().next()?.trim())
        .parent()
        .map(std::path::Path::to_path_buf)
}

fn should_reload_live_reason(reason: &str) -> bool {
    matches!(reason, "eof" | "error" | "redirect")
}

fn live_retry_delay(attempt: usize) -> Duration {
    Duration::from_millis((250u64.saturating_mul(1u64 << attempt.min(4))).min(4_000))
}

fn write_live_diagnostic(_room_id: i64, message: &str) {
    let message = crate::api::redact::redact_url_queries(message);
    let Some(mut dir) = dirs::config_dir() else {
        return;
    };
    dir.push("bilibili-tui");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("live.log");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut log) = options.open(&path) {
        use std::io::Write;
        let _ = writeln!(
            log,
            "[{}] live: {message}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(test)]
mod playlist_tests {
    use super::*;

    fn danmaku_at(times: &[f64]) -> Vec<VideoDanmaku> {
        times
            .iter()
            .map(|&time| VideoDanmaku {
                time,
                text: String::new(),
                color: 0,
            })
            .collect()
    }

    fn file_event(json: serde_json::Value) -> MpvFileEvent {
        mpv_file_event(&json).expect("file event")
    }

    #[test]
    fn stale_end_file_of_a_replaced_entry_is_ignored() {
        assert!(!is_current_entry(Some(3), Some(2)));
        assert!(is_current_entry(Some(3), Some(3)));
        assert!(is_current_entry(None, Some(2)));
        assert!(is_current_entry(Some(3), None));
        assert_eq!(
            file_event(
                serde_json::json!({"event": "end-file", "reason": "error", "playlist_entry_id": 2})
            ),
            MpvFileEvent::Ended {
                entry: Some(2),
                reason: "error".to_string()
            }
        );
    }

    #[test]
    fn load_fails_fast_when_the_new_file_errors() {
        let mut progress = LoadProgress::default();
        // The replaced file ending with an error does not fail the new load.
        progress
            .apply(file_event(
                serde_json::json!({"event": "end-file", "reason": "error", "playlist_entry_id": 1}),
            ))
            .unwrap();
        progress
            .apply(file_event(
                serde_json::json!({"event": "start-file", "playlist_entry_id": 2}),
            ))
            .unwrap();
        assert_eq!(progress.entry, Some(2));
        assert!(
            progress
                .apply(file_event(serde_json::json!({"event": "end-file", "reason": "error", "playlist_entry_id": 2})))
                .is_err()
        );
    }

    #[test]
    fn load_completes_after_command_and_file_loaded() {
        let mut progress = LoadProgress::default();
        progress
            .apply(file_event(
                serde_json::json!({"event": "start-file", "playlist_entry_id": 5}),
            ))
            .unwrap();
        progress
            .apply(file_event(serde_json::json!({"event": "file-loaded"})))
            .unwrap();
        assert!(!progress.done());
        progress.command_ok = true;
        assert!(progress.done());
    }

    #[test]
    fn playlist_appends_cid_without_replacing_user_script_opts() {
        assert_eq!(
            playlist_cid_option(42),
            serde_json::json!(["change-list", "script-opts", "append", "cid=42"])
        );
    }

    #[test]
    fn danmaku_resume_after_seeking_back_past_the_last_comment() {
        let danmaku = danmaku_at(&[1.0, 2.0, 3.0]);
        assert_eq!(due_danmaku_range(&danmaku, 3, 10.0, 10.02), 3..3);
        assert_eq!(due_danmaku_range(&danmaku, 3, 10.0, 1.5), 1..1);
        assert_eq!(due_danmaku_range(&danmaku, 1, 1.5, 2.0), 1..2);
    }

    #[test]
    fn forward_seek_skips_comments_instead_of_sending_them_in_bulk() {
        let danmaku = danmaku_at(&[1.0, 2.0, 3.0, 50.0, 60.0]);
        assert_eq!(due_danmaku_range(&danmaku, 0, 0.5, 55.0), 4..4);
        assert_eq!(due_danmaku_range(&danmaku, 0, 0.5, 1.01), 0..1);
    }

    #[test]
    fn ytdl_format_caps_by_stream_qn() {
        assert_eq!(ytdl_format(VideoQuality::Best), "bestvideo+bestaudio/best");
        assert_eq!(
            ytdl_format(VideoQuality::Q1080p),
            "bestvideo[quality<=80]+bestaudio/best[quality<=80]"
        );
        assert_eq!(
            ytdl_format(VideoQuality::Q720p),
            "bestvideo[quality<=64]+bestaudio/best[quality<=64]"
        );
    }

    #[test]
    fn session_files_are_removed_when_guard_drops() {
        let path = std::env::temp_dir().join(format!(
            "bilibili-tui-session-files-test-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"x").unwrap();
        let mut files = SessionFiles::new(Vec::new());
        files.push(path.clone());
        drop(files);
        assert!(!path.exists());
    }

    #[test]
    fn replaying_same_cid_uses_distinct_ipc_paths() {
        assert_ne!(vod_mpv_ipc_path(42), vod_mpv_ipc_path(42));
    }

    fn item(id: i64) -> PlaylistItem {
        PlaylistItem {
            bvid: format!("BV{id}"),
            aid: id,
            cid: None,
            title: id.to_string(),
            uploader_mid: None,
            duration: None,
            page: None,
        }
    }

    #[test]
    fn vod_mpv_uses_normal_buffering_settings() {
        let mut command = Command::new("mpv");
        configure_vod_buffering(&mut command);
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            [
                "--audio-buffer=0.2",
                "--vd-lavc-threads=0",
                "--cache=yes",
                "--cache-pause=yes",
                "--demuxer-lavf-o=",
                "--demuxer-lavf-probe-info=auto",
                "--demuxer-lavf-analyzeduration=0",
                "--video-latency-hacks=no",
                "--stream-buffer-size=128KiB",
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--profile=low-latency"));
    }

    #[test]
    fn reverse_play_all_starts_at_first_reversed_item() {
        let items = vec![item(1), item(2), item(3)];
        let (items, start) = ordered_playlist(items, PlayOrder::Reverse, 2).unwrap();
        assert_eq!(
            items.iter().map(|item| item.aid).collect::<Vec<_>>(),
            [3, 2, 1]
        );
        assert_eq!(start, 0);
    }

    #[test]
    fn playlist_skips_failed_items_and_selects_next_success() {
        let items = [item(1), item(2), item(3)];
        let mut failures = Vec::new();
        assert_eq!(
            accept_prepared_result::<i64>(
                &items[0],
                Err(anyhow::anyhow!("unavailable")),
                &mut failures
            ),
            None
        );
        assert_eq!(
            accept_prepared_result(&items[1], Ok(2), &mut failures),
            Some(2)
        );
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn playlist_reports_all_candidates_failed() {
        let items = [item(1), item(2)];
        let mut failures = Vec::new();
        for item in &items {
            assert_eq!(
                accept_prepared_result::<i64>(item, Err(anyhow::anyhow!("blocked")), &mut failures),
                None
            );
        }
        assert_eq!(failures.len(), items.len());
    }

    #[test]
    fn forward_play_all_preserves_web_order() {
        let items = vec![item(1), item(2), item(3)];
        let (items, start) = ordered_playlist(items, PlayOrder::Forward, 0).unwrap();
        assert_eq!(
            items.iter().map(|item| item.aid).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(start, 0);
    }

    #[test]
    fn recognizes_corrupt_h264_diagnostics() {
        assert!(is_corrupt_video_log(
            "h264: Invalid NAL unit size (123 > 10)"
        ));
        assert!(is_corrupt_video_log(
            "h264: Error splitting the input into NAL units."
        ));
        assert!(!is_corrupt_video_log("AO: [coreaudio] 48000Hz stereo"));
    }

    #[test]
    fn live_failover_reloads_only_stream_failures() {
        assert!(should_reload_live_reason("eof"));
        assert!(should_reload_live_reason("error"));
        assert!(should_reload_live_reason("redirect"));
        assert!(!should_reload_live_reason("stop"));
        assert!(!should_reload_live_reason("quit"));
    }

    #[test]
    fn live_retry_backoff_is_bounded() {
        assert!(live_retry_delay(2) > live_retry_delay(1));
        assert_eq!(live_retry_delay(8), Duration::from_secs(4));
        assert_eq!(live_retry_delay(100), Duration::from_secs(4));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mpv_ipc_ignores_events_before_its_matching_response() {
        let path = std::env::temp_dir().join(format!(
            "bilibili-tui-ipc-test-{}-{}.sock",
            std::process::id(),
            MPV_IPC_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).expect("bind test IPC");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept IPC");
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            reader.read_line(&mut request).await.expect("read request");
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            let request_id = request["request_id"].as_u64().unwrap();
            let stream = reader.get_mut();
            stream
                .write_all(b"{\"event\":\"client-message\"}\n")
                .await
                .unwrap();
            stream
                .write_all(
                    format!("{{\"error\":\"success\",\"request_id\":{request_id}}}\n").as_bytes(),
                )
                .await
                .unwrap();
        });

        let response = mpv_ipc(&path, serde_json::json!(["get_property", "pause"]))
            .await
            .expect("matching response");
        assert_eq!(response["error"], "success");
        server.await.unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn live_danmaku_payload_contains_text_and_color() {
        let message = LiveMessage::Danmaku {
            uid: 7,
            uname: "tester".to_string(),
            content: "你好".to_string(),
            color: 0x12_34_56,
        };
        let payload = live_danmaku_payload(&message).expect("danmaku payload");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("valid payload");
        assert_eq!(value["text"], "你好");
        assert_eq!(value["color"], 0x12_34_56);
        assert!(live_danmaku_payload(&LiveMessage::Popularity(1)).is_none());
    }

    #[test]
    fn live_danmaku_script_registers_the_ipc_message_handler() {
        assert!(LIVE_DANMAKU_SCRIPT.contains("register_script_message"));
        assert!(LIVE_DANMAKU_SCRIPT.contains("danmaku-config"));
        assert!(LIVE_DANMAKU_SCRIPT.contains("create_osd_overlay"));
        assert!(LIVE_DANMAKU_SCRIPT.contains("\\pos("));
        assert!(LIVE_DANMAKU_SCRIPT.contains("observe_property(\"display-fps\""));
        assert!(LIVE_DANMAKU_SCRIPT.contains("add_periodic_timer(1 / fps"));
        assert!(LIVE_DANMAKU_SCRIPT.contains("get_property_number(\"display-fps\""));
        assert!(LIVE_DANMAKU_SCRIPT.contains("video-reconfig"));
        assert!(!LIVE_DANMAKU_SCRIPT.contains("sub-reload"));
    }

    #[test]
    fn live_mpv_is_idle_ipc_controlled_and_disables_global_url_logging() {
        let mut command = Command::new("mpv");
        configure_live_mpv(&mut command, std::path::Path::new("/tmp/live-test.sock"));
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == "--idle=yes"));
        assert!(args.iter().any(|arg| arg == "--keep-open=yes"));
        assert!(args.iter().any(|arg| arg == "--resume-playback=no"));
        assert!(args.iter().any(|arg| arg == "--cache-secs=3600"));
        assert!(args.iter().any(|arg| arg == "--log-file="));
        assert!(args.iter().any(|arg| arg.ends_with("/tmp/live-test.sock")));
        assert!(!args.iter().any(|arg| arg.contains("bilivideo.com")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn live_cleanup_kills_a_child_when_ipc_is_unavailable() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .expect("spawn test child");
        shutdown_live_child(
            std::path::Path::new("/tmp/bilibili-tui-missing-live.sock"),
            &mut child,
            Duration::from_millis(10),
        )
        .await;
        assert!(child.try_wait().expect("query child").is_some());
    }

    #[tokio::test]
    #[ignore = "requires login, network access, and mpv"]
    async fn best_live_stream_decodes_in_mpv() {
        let credentials = crate::storage::load_credentials().expect("load credentials");
        let client = Arc::new(ApiClient::with_cookies(&credentials));
        let room = client
            .get_live_home_rooms()
            .await
            .expect("live rooms")
            .into_iter()
            .next()
            .expect("an active live room");
        let url = client
            .get_live_stream_urls(room.roomid, VideoQuality::Best)
            .await
            .expect("best stream")
            .into_iter()
            .next()
            .expect("stream URL");
        let mut cmd = Command::new("mpv");
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.arg("--no-config");
        cmd.arg("--vo=null");
        cmd.arg("--ao=null");
        cmd.arg("--frames=30");
        let ipc = std::env::temp_dir().join(format!(
            "bilibili-tui-live-test-{}.sock",
            std::process::id()
        ));
        configure_live_mpv(&mut cmd, &ipc);
        cmd.arg("--idle=no");
        cmd.arg(url);
        let mut child = cmd.spawn().expect("spawn mpv");
        let status = tokio::time::timeout(Duration::from_secs(30), child.wait())
            .await
            .expect("mpv decode timeout")
            .expect("wait for mpv");
        assert!(status.success());
    }

    #[tokio::test]
    #[ignore = "requires network access and mpv"]
    async fn mpv_switches_proxy_cdn_without_restarting() {
        let client = ApiClient::new();
        let bvid = "BV1cP7j64E37";
        let info = client.get_video_info(bvid).await.expect("video info");
        let play_url = client
            .get_play_url(bvid, info.cid, VideoQuality::Best)
            .await
            .expect("playurl");
        let streams = crate::api::cdn::rank_streams(&play_url, VideoQuality::Best)
            .await
            .expect("rank CDN streams");
        assert!(streams.video.len() > 1, "test needs a backup video CDN");
        let mut proxy = proxy::MediaProxy::start(streams)
            .await
            .expect("start proxy");
        let ipc = std::env::temp_dir().join(format!(
            "bilibili-tui-switch-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&ipc);
        let mut command = Command::new("mpv");
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("--no-config")
            .arg("--vo=null")
            .arg("--ao=null")
            .arg(format!("--input-ipc-server={}", ipc.display()))
            .arg(format!("--audio-file={}", proxy.audio_url))
            .arg(&proxy.video_url);
        let mut child = command.spawn().expect("spawn mpv");
        let pid = child.id();
        for _ in 0..50 {
            if ipc.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let position = mpv_time_pos(&ipc).await.expect("read playback position");
        let backup = proxy.switch_video_cdn_for_test().expect("backup CDN");
        replace_mpv_stream(&ipc, &backup, &proxy.audio_url, position)
            .await
            .expect("replace stream over IPC");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(child.id(), pid);
        assert!(child.try_wait().expect("query mpv").is_none());
        assert!(mpv_time_pos(&ipc).await.expect("position after switch") >= position);
        child.kill().await.expect("stop test mpv");
        let _ = std::fs::remove_file(ipc);
    }
}
