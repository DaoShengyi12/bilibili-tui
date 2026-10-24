//! Application-level playback queue state.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub bvid: String,
    pub aid: i64,
    pub cid: Option<i64>,
    pub title: String,
    pub uploader_mid: Option<i64>,
    pub duration: Option<i64>,
    pub page: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlayOrder {
    #[default]
    Forward,
    Reverse,
    Shuffle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistSource {
    Manual,
    Uploader { mid: i64, name: String },
    Favorites { media_id: i64, title: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    #[default]
    Idle,
    Starting,
    Playing,
    Paused,
    Failed,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackEvent {
    /// The player process is running; sent once its setup finished.
    Started {
        session_id: u64,
    },
    ItemChanged {
        session_id: u64,
        index: usize,
        bvid: String,
    },
    Finished {
        session_id: u64,
        bvid: Option<String>,
    },
    Failed {
        session_id: u64,
        error: String,
    },
}

#[derive(Debug, Default)]
pub struct PlaybackState {
    pub queue: Vec<PlaylistItem>,
    pub current_index: Option<usize>,
    pub order: PlayOrder,
    pub source: Option<PlaylistSource>,
    pub status: PlaybackStatus,
    pub last_error: Option<String>,
    pub session_id: Option<u64>,
}

impl PlaybackState {
    pub fn replace_queue(&mut self, source: PlaylistSource, items: Vec<PlaylistItem>) {
        self.queue = items;
        self.source = Some(source);
        self.current_index = (!self.queue.is_empty()).then_some(0);
        self.status = PlaybackStatus::Idle;
        self.last_error = None;
        self.session_id = None;
    }

    /// Claims `session_id` before the player starts in the background, so
    /// events from the new player are accepted as soon as they arrive.
    pub fn begin_starting(&mut self, session_id: u64) {
        self.session_id = Some(session_id);
        self.status = PlaybackStatus::Starting;
        self.last_error = None;
    }

    pub fn begin_session(&mut self, session_id: u64) {
        self.session_id = Some(session_id);
        self.status = PlaybackStatus::Playing;
        self.last_error = None;
    }

    pub fn apply_event(&mut self, event: &PlaybackEvent) -> bool {
        let session_id = match event {
            PlaybackEvent::Started { session_id }
            | PlaybackEvent::ItemChanged { session_id, .. }
            | PlaybackEvent::Finished { session_id, .. }
            | PlaybackEvent::Failed { session_id, .. } => *session_id,
        };
        if self.session_id != Some(session_id) {
            return false;
        }
        match event {
            PlaybackEvent::Started { .. } => {
                if self.status == PlaybackStatus::Starting {
                    self.status = PlaybackStatus::Playing;
                }
            }
            PlaybackEvent::ItemChanged { index, bvid, .. } => {
                self.current_index = if self
                    .queue
                    .get(*index)
                    .is_some_and(|item| item.bvid == *bvid)
                {
                    Some(*index)
                } else {
                    self.queue.iter().position(|item| item.bvid == *bvid)
                };
            }
            PlaybackEvent::Finished { .. } => {
                self.status = PlaybackStatus::Finished;
                self.session_id = None;
            }
            PlaybackEvent::Failed { error, .. } => {
                self.status = PlaybackStatus::Failed;
                self.last_error = Some(error.clone());
                self.session_id = None;
            }
        }
        true
    }

    pub fn play_from(&mut self, index: usize) -> bool {
        if index >= self.queue.len() {
            return false;
        }
        self.current_index = Some(index);
        self.status = PlaybackStatus::Starting;
        self.last_error = None;
        true
    }

    pub fn advance(&mut self) -> bool {
        let Some(current) = self.current_index else {
            return false;
        };
        let next = match self.order {
            PlayOrder::Forward => current
                .checked_add(1)
                .filter(|next| *next < self.queue.len()),
            PlayOrder::Reverse => current.checked_sub(1),
            PlayOrder::Shuffle => current
                .checked_add(1)
                .filter(|next| *next < self.queue.len()),
        };
        if let Some(next) = next {
            self.current_index = Some(next);
            self.status = PlaybackStatus::Starting;
            true
        } else {
            self.status = PlaybackStatus::Finished;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_start_reports_playing_or_failure_for_its_session() {
        let mut state = PlaybackState::default();
        state.begin_starting(1);
        assert_eq!(state.status, PlaybackStatus::Starting);
        assert!(state.apply_event(&PlaybackEvent::Started { session_id: 1 }));
        assert_eq!(state.status, PlaybackStatus::Playing);

        state.begin_starting(2);
        assert!(!state.apply_event(&PlaybackEvent::Started { session_id: 1 }));
        assert_eq!(state.status, PlaybackStatus::Starting);
        assert!(state.apply_event(&PlaybackEvent::Failed {
            session_id: 2,
            error: "启动播放器失败: mpv".into(),
        }));
        assert_eq!(state.status, PlaybackStatus::Failed);
        assert_eq!(state.session_id, None);
    }

    fn item(id: i64) -> PlaylistItem {
        PlaylistItem {
            bvid: format!("BV{id}"),
            aid: id,
            cid: Some(id),
            title: format!("video {id}"),
            uploader_mid: Some(1),
            duration: Some(60),
            page: None,
        }
    }

    #[test]
    fn replacing_queue_resets_position_and_error() {
        let mut state = PlaybackState {
            last_error: Some("old error".into()),
            ..PlaybackState::default()
        };
        state.replace_queue(PlaylistSource::Manual, vec![item(1), item(2)]);
        assert_eq!(state.current_index, Some(0));
        assert_eq!(state.last_error, None);
    }

    #[test]
    fn forward_queue_finishes_after_last_item() {
        let mut state = PlaybackState::default();
        state.replace_queue(PlaylistSource::Manual, vec![item(1), item(2)]);
        assert!(state.advance());
        assert_eq!(state.current_index, Some(1));
        assert!(!state.advance());
        assert_eq!(state.status, PlaybackStatus::Finished);
    }

    #[test]
    fn play_from_rejects_out_of_bounds_index() {
        let mut state = PlaybackState::default();
        state.replace_queue(PlaylistSource::Manual, vec![item(1)]);
        assert!(!state.play_from(1));
        assert_eq!(state.current_index, Some(0));
    }

    #[test]
    fn ignores_events_from_an_old_session() {
        let mut state = PlaybackState::default();
        state.replace_queue(PlaylistSource::Manual, vec![item(1), item(2)]);
        state.begin_session(2);
        assert!(!state.apply_event(&PlaybackEvent::Finished {
            session_id: 1,
            bvid: None
        }));
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert!(state.apply_event(&PlaybackEvent::ItemChanged {
            session_id: 2,
            index: 1,
            bvid: "BV2".into()
        }));
        assert_eq!(state.current_index, Some(1));
    }

    #[test]
    fn failed_player_marks_active_session_failed() {
        let mut state = PlaybackState::default();
        state.replace_queue(PlaylistSource::Manual, vec![item(1)]);
        state.begin_session(7);
        assert!(state.apply_event(&PlaybackEvent::Failed {
            session_id: 7,
            error: "decoder failed".into(),
        }));
        assert_eq!(state.status, PlaybackStatus::Failed);
        assert_eq!(state.last_error.as_deref(), Some("decoder failed"));
        assert_eq!(state.session_id, None);
    }

    #[test]
    fn shuffled_queue_advances_in_its_materialized_order() {
        let mut state = PlaybackState::default();
        state.replace_queue(PlaylistSource::Manual, vec![item(1), item(2)]);
        state.order = PlayOrder::Shuffle;
        assert!(state.advance());
        assert_eq!(state.current_index, Some(1));
        assert!(!state.advance());
    }
}
