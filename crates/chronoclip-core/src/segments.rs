//! Index of finished buffer segments and retention-based pruning.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

const PREFIX: &str = "seg_";
const EXTENSION: &str = ".ts";

/// File name for segment number `seq`, e.g. `seg_00000042.ts`.
pub fn segment_file_name(seq: u64) -> String {
    format!("{PREFIX}{seq:08}{EXTENSION}")
}

/// True for file names produced by [`segment_file_name`]. Used to clean the buffer folder
/// without touching anything else the user may have put there.
pub fn is_segment_file_name(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(EXTENSION))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Delete leftover segment files in `dir` (from a previous run or crash). Creates `dir` if
/// missing. Other files are left alone. Returns the number of files deleted.
pub fn clean_buffer_dir(dir: &std::path::Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut deleted = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(is_segment_file_name)
            && entry.file_type()?.is_file()
        {
            std::fs::remove_file(entry.path())?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// Retention for a buffer of `minutes`.
pub fn retention_for_minutes(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes) * 60)
}

/// A finished segment on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub seq: u64,
    /// Segments with the same `stream` share codec parameters (same encoder settings and size),
    /// so they can be joined into one clip. It changes when the recording restarts at a new size.
    pub stream: u64,
    pub path: PathBuf,
    /// Duration of video in the segment.
    pub duration: Duration,
    pub bytes: u64,
}

/// Ordered (oldest first) list of finished segments that together cover at least the
/// retention period, once enough has been recorded.
///
/// While a save is running, its segments are *pinned*: pruning still takes them out of the
/// index, but holds them back instead of handing them out for deletion until [`unpin`].
///
/// [`unpin`]: SegmentIndex::unpin
#[derive(Debug, Default)]
pub struct SegmentIndex {
    segments: VecDeque<Segment>,
    retention: Duration,
    total_duration: Duration,
    total_bytes: u64,
    /// Segments with `seq >= pinned_from` are not deleted.
    pinned_from: Option<u64>,
    /// Pruned while pinned; deleted on unpin.
    deferred: Vec<Segment>,
}

impl SegmentIndex {
    pub fn new(retention: Duration) -> Self {
        Self {
            retention,
            ..Self::default()
        }
    }

    pub fn retention(&self) -> Duration {
        self.retention
    }

    /// Change the retention. Returns segments that are no longer needed; the caller deletes them.
    #[must_use = "the returned segments must be deleted from disk"]
    pub fn set_retention(&mut self, retention: Duration) -> Vec<Segment> {
        self.retention = retention;
        self.prune()
    }

    /// Add a finished segment. Returns segments that fell out of the retention window; the
    /// caller deletes them.
    #[must_use = "the returned segments must be deleted from disk"]
    pub fn push(&mut self, segment: Segment) -> Vec<Segment> {
        self.total_duration += segment.duration;
        self.total_bytes += segment.bytes;
        self.segments.push_back(segment);
        self.prune()
    }

    /// Drop the oldest segments while the rest still cover the retention period.
    fn prune(&mut self) -> Vec<Segment> {
        let mut removed = Vec::new();
        while let Some(oldest) = self.segments.front() {
            if self.total_duration.saturating_sub(oldest.duration) < self.retention {
                break;
            }
            if let Some(oldest) = self.segments.pop_front() {
                self.total_duration -= oldest.duration;
                self.total_bytes -= oldest.bytes;
                if self.pinned_from.is_some_and(|from| oldest.seq >= from) {
                    self.deferred.push(oldest);
                } else {
                    removed.push(oldest);
                }
            }
        }
        removed
    }

    /// The newest segments that together cover at least `length` (or everything available),
    /// oldest first. Only segments that can be joined with the newest one (same `stream`) are
    /// included, so a clip never spans a change of resolution.
    pub fn select_for_save(&self, length: Duration) -> Vec<Segment> {
        let Some(newest) = self.segments.back() else {
            return Vec::new();
        };
        let mut covered = Duration::ZERO;
        let mut selected: Vec<Segment> = self
            .segments
            .iter()
            .rev()
            .take_while(|s| s.stream == newest.stream)
            .take_while(|s| {
                let more = covered < length;
                covered += s.duration;
                more
            })
            .cloned()
            .collect();
        selected.reverse();
        selected
    }

    /// Protect segments with `seq >= from_seq` from deletion until [`unpin`](Self::unpin).
    /// Only one pin exists at a time; pinning again replaces it.
    pub fn pin(&mut self, from_seq: u64) {
        self.pinned_from = Some(from_seq);
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned_from.is_some()
    }

    /// Release the pin. Returns segments that were pruned while pinned; the caller deletes them.
    #[must_use = "the returned segments must be deleted from disk"]
    pub fn unpin(&mut self) -> Vec<Segment> {
        self.pinned_from = None;
        std::mem::take(&mut self.deferred)
    }

    /// Remove all segments (including ones held back by a pin) and release the pin. Returns
    /// them so the caller can delete the files.
    #[must_use = "the returned segments must be deleted from disk"]
    pub fn clear(&mut self) -> Vec<Segment> {
        self.total_duration = Duration::ZERO;
        self.total_bytes = 0;
        let mut all = self.unpin();
        all.extend(self.segments.drain(..));
        all
    }

    pub fn total_duration(&self) -> Duration {
        self.total_duration
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Segments, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(seq: u64, secs: u64) -> Segment {
        seg_in(0, seq, secs)
    }

    fn seg_in(stream: u64, seq: u64, secs: u64) -> Segment {
        Segment {
            seq,
            stream,
            path: PathBuf::from(segment_file_name(seq)),
            duration: Duration::from_secs(secs),
            bytes: 1000,
        }
    }

    #[test]
    fn file_names() {
        assert_eq!(segment_file_name(42), "seg_00000042.ts");
        assert!(is_segment_file_name("seg_00000042.ts"));
        assert!(is_segment_file_name(&segment_file_name(123_456_789_012)));
        assert!(!is_segment_file_name("seg_.ts"));
        assert!(!is_segment_file_name("seg_12a.ts"));
        assert!(!is_segment_file_name("my_clip.ts"));
        assert!(!is_segment_file_name("seg_00000042.mp4"));
    }

    #[test]
    fn keeps_at_least_retention() {
        let mut index = SegmentIndex::new(Duration::from_secs(10));
        for seq in 0..5 {
            assert!(
                index.push(seg(seq, 2)).is_empty(),
                "nothing pruned before 10 s"
            );
        }
        assert_eq!(index.total_duration(), Duration::from_secs(10));
        // 12 s recorded; dropping the oldest still leaves 10 s.
        let removed = index.push(seg(5, 2));
        assert_eq!(removed.iter().map(|s| s.seq).collect::<Vec<_>>(), vec![0]);
        assert_eq!(index.total_duration(), Duration::from_secs(10));
        assert_eq!(index.len(), 5);
        assert_eq!(index.total_bytes(), 5000);
        assert_eq!(index.iter().next().map(|s| s.seq), Some(1));
    }

    #[test]
    fn never_drops_below_retention() {
        let mut index = SegmentIndex::new(Duration::from_secs(10));
        let _ = index.push(seg(0, 3));
        let _ = index.push(seg(1, 3));
        let _ = index.push(seg(2, 3));
        // 12 s; dropping the 3 s oldest would leave 9 s < 10 s, so keep it.
        let removed = index.push(seg(3, 3));
        assert!(removed.is_empty());
        let removed = index.push(seg(4, 3));
        assert_eq!(removed.iter().map(|s| s.seq).collect::<Vec<_>>(), vec![0]);
        assert!(index.total_duration() >= Duration::from_secs(10));
    }

    #[test]
    fn shrinking_retention_prunes() {
        let mut index = SegmentIndex::new(Duration::from_secs(60));
        for seq in 0..30 {
            let _ = index.push(seg(seq, 2));
        }
        let removed = index.set_retention(Duration::from_secs(10));
        assert_eq!(removed.len(), 25);
        assert_eq!(index.total_duration(), Duration::from_secs(10));
    }

    #[test]
    fn clear_returns_everything() {
        let mut index = SegmentIndex::new(Duration::from_secs(60));
        for seq in 0..3 {
            let _ = index.push(seg(seq, 2));
        }
        assert_eq!(index.clear().len(), 3);
        assert!(index.is_empty());
        assert_eq!(index.total_bytes(), 0);
    }

    fn seqs(segments: &[Segment]) -> Vec<u64> {
        segments.iter().map(|s| s.seq).collect()
    }

    #[test]
    fn selects_newest_segments_covering_length() {
        let mut index = SegmentIndex::new(Duration::from_secs(60));
        for seq in 0..10 {
            let _ = index.push(seg(seq, 2));
        }
        assert_eq!(
            seqs(&index.select_for_save(Duration::from_secs(6))),
            vec![7, 8, 9]
        );
        // Not a multiple of the segment length: round up to cover it.
        assert_eq!(
            seqs(&index.select_for_save(Duration::from_secs(5))),
            vec![7, 8, 9]
        );
        // Longer than what was recorded: everything.
        assert_eq!(index.select_for_save(Duration::from_secs(600)).len(), 10);
        assert!(
            SegmentIndex::default()
                .select_for_save(Duration::from_secs(60))
                .is_empty()
        );
    }

    #[test]
    fn selection_stops_at_stream_change() {
        let mut index = SegmentIndex::new(Duration::from_secs(60));
        let _ = index.push(seg_in(0, 0, 2));
        let _ = index.push(seg_in(0, 1, 2));
        let _ = index.push(seg_in(1, 2, 2)); // resized: new stream
        let _ = index.push(seg_in(1, 3, 2));
        assert_eq!(
            seqs(&index.select_for_save(Duration::from_secs(60))),
            vec![2, 3]
        );
    }

    #[test]
    fn pinned_segments_are_deleted_only_after_unpin() {
        let mut index = SegmentIndex::new(Duration::from_secs(4));
        let _ = index.push(seg(0, 2));
        let _ = index.push(seg(1, 2));
        index.pin(0);
        assert!(index.is_pinned());
        // Both would be pruned, but they are pinned.
        assert!(index.push(seg(2, 2)).is_empty());
        assert!(index.push(seg(3, 2)).is_empty());
        // Retention is still enforced for the index itself.
        assert_eq!(seqs(&index.iter().cloned().collect::<Vec<_>>()), vec![2, 3]);
        assert_eq!(seqs(&index.unpin()), vec![0, 1]);
        assert!(!index.is_pinned());
        assert_eq!(seqs(&index.push(seg(4, 2))), vec![2]);
        assert!(index.unpin().is_empty());
    }

    #[test]
    fn pin_protects_only_newer_segments() {
        let mut index = SegmentIndex::new(Duration::from_secs(4));
        for seq in 0..3 {
            let _ = index.push(seg(seq, 2));
        }
        // Index holds 1, 2; a save of just the newest segment pins from 2.
        index.pin(2);
        assert_eq!(seqs(&index.push(seg(3, 2))), vec![1]);
        assert!(index.push(seg(4, 2)).is_empty()); // 2 is pinned
        assert_eq!(seqs(&index.unpin()), vec![2]);
    }

    #[test]
    fn clear_includes_pinned_segments() {
        let mut index = SegmentIndex::new(Duration::from_secs(2));
        let _ = index.push(seg(0, 2));
        index.pin(0);
        assert!(index.push(seg(1, 2)).is_empty());
        assert_eq!(seqs(&index.clear()), vec![0, 1]);
        assert!(!index.is_pinned());
    }

    #[test]
    fn cleans_only_segment_files() -> std::io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("buffer");
        assert_eq!(clean_buffer_dir(&dir)?, 0); // creates it
        std::fs::write(dir.join(segment_file_name(1)), b"x")?;
        std::fs::write(dir.join(segment_file_name(2)), b"x")?;
        std::fs::write(dir.join("notes.txt"), b"keep me")?;
        assert_eq!(clean_buffer_dir(&dir)?, 2);
        assert!(dir.join("notes.txt").exists());
        Ok(())
    }

    #[test]
    fn retention_minutes() {
        assert_eq!(retention_for_minutes(10), Duration::from_secs(600));
    }
}
