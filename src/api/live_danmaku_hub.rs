//! Shared live danmaku connection for UI and player consumers.

use super::{ApiClient, LiveClient, LiveMessage};
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, interval};

/// Owns one Bilibili live WebSocket and broadcasts every decoded message to
/// independent subscribers such as the TUI and MPV bridge.
pub struct LiveDanmakuHub {
    room_id: i64,
    message_tx: broadcast::Sender<LiveMessage>,
    connected: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    shutdown_tx: mpsc::Sender<()>,
}

impl std::fmt::Debug for LiveDanmakuHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveDanmakuHub")
            .field("room_id", &self.room_id)
            .finish_non_exhaustive()
    }
}

impl LiveDanmakuHub {
    pub async fn connect(api_client: Arc<ApiClient>, room_id: i64, uid: i64) -> Result<Arc<Self>> {
        let danmu_info = api_client.get_danmu_info(room_id).await?;
        let buvid = api_client.get_buvid3().await.unwrap_or_default();
        let mut client = LiveClient::connect(api_client, room_id, uid, buvid, &danmu_info).await?;
        let (message_tx, _) = broadcast::channel(512);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        let connected = Arc::new(AtomicBool::new(false));
        let last_error = Arc::new(Mutex::new(None));

        let task_tx = message_tx.clone();
        let task_connected = Arc::clone(&connected);
        let task_error = Arc::clone(&last_error);
        tokio::spawn(async move {
            let mut status_tick = interval(Duration::from_millis(100));
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = status_tick.tick() => {
                        task_connected.store(client.is_connected(), Ordering::Release);
                        if let Some(error) = client.take_error()
                            && let Ok(mut value) = task_error.lock()
                        {
                            *value = Some(error);
                        }
                        while let Some(message) = client.try_recv() {
                            let _ = task_tx.send(message);
                        }
                    }
                }
            }
            client.disconnect().await;
            task_connected.store(false, Ordering::Release);
        });

        Ok(Arc::new(Self {
            room_id,
            message_tx,
            connected,
            last_error,
            shutdown_tx,
        }))
    }

    pub fn room_id(&self) -> i64 {
        self.room_id
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LiveMessage> {
        self.message_tx.subscribe()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub fn take_error(&self) -> Option<String> {
        self.last_error.lock().ok()?.take()
    }
}

impl Drop for LiveDanmakuHub {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_independent_copies() {
        let (message_tx, _) = broadcast::channel(4);
        let (shutdown_tx, _shutdown_rx) = mpsc::channel(1);
        let hub = LiveDanmakuHub {
            room_id: 7,
            message_tx,
            connected: Arc::new(AtomicBool::new(true)),
            last_error: Arc::new(Mutex::new(None)),
            shutdown_tx,
        };
        let mut first = hub.subscribe();
        let mut second = hub.subscribe();
        hub.message_tx
            .send(LiveMessage::Popularity(42))
            .expect("subscribers are active");

        assert!(matches!(
            first.recv().await,
            Ok(LiveMessage::Popularity(42))
        ));
        assert!(matches!(
            second.recv().await,
            Ok(LiveMessage::Popularity(42))
        ));
    }
}
