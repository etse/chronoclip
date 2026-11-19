//! The replay buffer shared between the writer thread (adds segments) and saves (read them).

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chronoclip_core::segments::{Segment, SegmentIndex};

#[derive(Debug, Default)]
struct State {
    index: SegmentIndex,
    /// Sequence number of the segment the writer is currently filling.
    open_seq: Option<u64>,
}

/// Finished segments plus the segment being written. Every change wakes up waiters.
#[derive(Debug)]
pub struct ReplayBuffer {
    state: Mutex<State>,
    changed: Condvar,
}

impl ReplayBuffer {
    pub fn new(retention: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                index: SegmentIndex::new(retention),
                open_seq: None,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Writer: the segment now being written (`None` when no segment is open).
    pub fn set_open_segment(&self, seq: Option<u64>) {
        let mut state = self.lock();
        if state.open_seq != seq {
            state.open_seq = seq;
            self.changed.notify_all();
        }
    }

    /// Writer: add a finished segment, apply `retention` and delete segments that fell out of
    /// the buffer (unless a save is using them).
    pub fn add_segment(&self, segment: Segment, retention: Duration) {
        let removed = {
            let mut state = self.lock();
            let mut removed = if retention != state.index.retention() {
                state.index.set_retention(retention)
            } else {
                Vec::new()
            };
            removed.extend(state.index.push(segment));
            self.changed.notify_all();
            removed
        };
        delete(removed);
    }

    /// Length and size of the finished segments.
    pub fn stats(&self) -> (Duration, u64) {
        let state = self.lock();
        (state.index.total_duration(), state.index.total_bytes())
    }

    /// Wait until the segment that is open right now has been finished (at most `timeout`),
    /// so a save includes the moment it was requested.
    pub fn wait_for_open_segment(&self, timeout: Duration) {
        let state = self.lock();
        let Some(open) = state.open_seq else { return };
        let (state, result) = self
            .changed
            .wait_timeout_while(state, timeout, |s| s.open_seq == Some(open))
            .unwrap_or_else(|e| e.into_inner());
        if result.timed_out() {
            tracing::warn!(
                open = ?state.open_seq,
                "segment {open} was not finished in time; saving without it"
            );
        }
    }

    /// Pick the segments for a save (the last buffer length) and protect them from deletion
    /// until the returned guard is dropped. `None` if nothing has been recorded.
    pub fn pin_for_save(self: &Arc<Self>) -> Option<PinnedSegments> {
        let mut state = self.lock();
        let retention = state.index.retention();
        let segments = state.index.select_for_save(retention);
        let first = segments.first()?.seq;
        state.index.pin(first);
        Some(PinnedSegments {
            buffer: Arc::clone(self),
            segments,
        })
    }

    /// Wait until no save is using the buffer (at most `timeout`). Returns false on timeout.
    pub fn wait_until_unpinned(&self, timeout: Duration) -> bool {
        let state = self.lock();
        let (_state, result) = self
            .changed
            .wait_timeout_while(state, timeout, |s| s.index.is_pinned())
            .unwrap_or_else(|e| e.into_inner());
        !result.timed_out()
    }

    /// Forget all segments (the files are deleted by cleaning the buffer folder).
    pub fn clear(&self) {
        let mut state = self.lock();
        let _ = state.index.clear();
        state.open_seq = None;
        self.changed.notify_all();
    }
}

/// Segments selected for a save, oldest first. They stay on disk until this is dropped.
pub struct PinnedSegments {
    buffer: Arc<ReplayBuffer>,
    pub segments: Vec<Segment>,
}

impl Drop for PinnedSegments {
    fn drop(&mut self) {
        let deferred = {
            let mut state = self.buffer.lock();
            let deferred = state.index.unpin();
            self.buffer.changed.notify_all();
            deferred
        };
        delete(deferred);
    }
}

fn delete(segments: Vec<Segment>) {
    for segment in segments {
        if let Err(e) = std::fs::remove_file(&segment.path) {
            tracing::warn!("could not delete {}: {e}", segment.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::thread;
    use std::time::Instant;

    fn segment(dir: &Path, seq: u64) -> Segment {
        let path = dir.join(chronoclip_core::segments::segment_file_name(seq));
        std::fs::write(&path, b"x").unwrap_or_default();
        Segment {
            seq,
            stream: 0,
            path,
            duration: Duration::from_secs(2),
            bytes: 1,
        }
    }

    #[test]
    fn pinned_files_survive_until_the_save_ends() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let buffer = Arc::new(ReplayBuffer::new(Duration::from_secs(4)));
        let retention = Duration::from_secs(4);
        buffer.add_segment(segment(dir.path(), 0), retention);
        buffer.add_segment(segment(dir.path(), 1), retention);

        let pinned = buffer.pin_for_save().ok_or(std::io::ErrorKind::NotFound)?;
        assert_eq!(pinned.segments.len(), 2);
        let first = pinned.segments[0].path.clone();
        buffer.add_segment(segment(dir.path(), 2), retention);
        assert!(first.exists(), "pinned segment was deleted");
        assert!(!buffer.wait_until_unpinned(Duration::from_millis(10)));

        drop(pinned);
        assert!(!first.exists(), "segment not deleted after the save");
        assert!(buffer.wait_until_unpinned(Duration::ZERO));
        Ok(())
    }

    #[test]
    fn nothing_to_pin_when_empty() {
        let buffer = Arc::new(ReplayBuffer::new(Duration::from_secs(4)));
        assert!(buffer.pin_for_save().is_none());
    }

    #[test]
    fn waits_for_the_open_segment() {
        let buffer = Arc::new(ReplayBuffer::new(Duration::from_secs(4)));
        // No open segment: returns at once.
        let start = Instant::now();
        buffer.wait_for_open_segment(Duration::from_secs(5));
        assert!(start.elapsed() < Duration::from_secs(1));

        buffer.set_open_segment(Some(7));
        let writer = {
            let buffer = Arc::clone(&buffer);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                buffer.set_open_segment(Some(8));
            })
        };
        let start = Instant::now();
        buffer.wait_for_open_segment(Duration::from_secs(5));
        let waited = start.elapsed();
        assert!(waited >= Duration::from_millis(90) && waited < Duration::from_secs(2));
        let _ = writer.join();

        // Times out if the writer never finishes it.
        let start = Instant::now();
        buffer.wait_for_open_segment(Duration::from_millis(50));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }
}
