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
    pub path: PathBuf,
    /// Duration of video in the segment.
    pub duration: Duration,
    pub bytes: u64,
}

/// Ordered (oldest first) list of finished segments that together cover at least the
/// retention period, once enough has been recorded.
#[derive(Debug, Default)]
pub struct SegmentIndex {
    segments: VecDeque<Segment>,
    retention: Duration,
    total_duration: Duration,
    total_bytes: u64,
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
                removed.push(oldest);
            }
        }
        removed
    }

    /// Remove all segments. Returns them so the caller can delete the files.
    #[must_use = "the returned segments must be deleted from disk"]
    pub fn clear(&mut self) -> Vec<Segment> {
        self.total_duration = Duration::ZERO;
        self.total_bytes = 0;
        self.segments.drain(..).collect()
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
        Segment {
            seq,
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
