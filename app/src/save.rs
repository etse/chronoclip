//! Saving a replay: the last buffer length of segments → one MP4 in the clips folder.
//!
//! Runs on its own thread (see `Controller::start_save`); capture and encoding continue.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chronoclip_core::clips::{check_free_space, partial_path, unique_clip_path};
use chronoclip_core::disk::{format_duration, format_size};

use crate::buffer::ReplayBuffer;
use crate::recorder::SEGMENT_SECONDS;
use crate::ui::capitalize;

/// How long a save waits for the segment being written to finish, so the clip contains the
/// moment the hotkey was pressed. One segment length plus slack for a busy disk.
const OPEN_SEGMENT_WAIT: Duration = Duration::from_secs(SEGMENT_SECONDS as u64 + 2);

#[derive(Debug, Clone)]
pub struct SavedClip {
    pub path: PathBuf,
    pub duration: Duration,
    pub bytes: u64,
    /// Time from the request to the finished file, including the wait for the open segment.
    pub elapsed: Duration,
}

impl SavedClip {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// e.g. `Chronoclip_2026-11-20_21-04-33.mp4 (10:00, 1.5 GB)`
    pub fn summary(&self) -> String {
        format!(
            "{} ({}, {})",
            self.file_name(),
            format_duration(self.duration),
            format_size(self.bytes)
        )
    }
}

/// Save the last buffer length from `buffer` into `output_dir`. Blocks until the clip is
/// written. Errors are messages for the user.
pub fn save_replay(buffer: &Arc<ReplayBuffer>, output_dir: &Path) -> Result<SavedClip, String> {
    let started = std::time::Instant::now();
    buffer.wait_for_open_segment(OPEN_SEGMENT_WAIT);
    let pinned = buffer.pin_for_save().ok_or(if buffer.has_saved() {
        "Nothing new recorded since the last clip."
    } else {
        "Nothing recorded yet. Start World of Warcraft and try again in a few seconds."
    })?;
    let last_seq = pinned.segments.last().map(|s| s.seq);

    std::fs::create_dir_all(output_dir).map_err(|e| {
        format!(
            "Cannot create the clips folder {}: {e}",
            output_dir.display()
        )
    })?;
    let clip_bytes: u64 = pinned.segments.iter().map(|s| s.bytes).sum();
    if let Some(available) = free_space(output_dir) {
        check_free_space(clip_bytes, available).map_err(|e| capitalize(&e.to_string()) + ".")?;
    }

    let path = unique_clip_path(output_dir, chrono::Local::now().naive_local());
    let partial = partial_path(&path);
    let inputs: Vec<PathBuf> = pinned.segments.iter().map(|s| s.path.clone()).collect();
    tracing::info!(
        segments = inputs.len(),
        bytes = clip_bytes,
        clip = %path.display(),
        "saving replay"
    );

    let result = chronoclip_encode::remux_to_mp4(&inputs, &partial, &buffer.audio_tracks());
    drop(pinned); // the buffer may delete old segments again
    let stats = match result {
        Ok(stats) => stats,
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("Could not write the clip: {e}"));
        }
    };
    if let Err(e) = std::fs::rename(&partial, &path) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("Could not write {}: {e}", path.display()));
    }
    // The next clip starts where this one ended: clips never overlap.
    if let Some(seq) = last_seq {
        buffer.consume_through(seq);
    }
    if stats.skipped_segments > 0 {
        tracing::warn!(
            skipped = stats.skipped_segments,
            "some buffer segments could not be read and are missing from the clip"
        );
    }
    let clip = SavedClip {
        path,
        duration: stats.duration,
        bytes: stats.bytes,
        elapsed: started.elapsed(),
    };
    tracing::info!(
        clip = %clip.path.display(),
        duration = ?clip.duration,
        bytes = clip.bytes,
        remux = ?stats.elapsed,
        total = ?clip.elapsed,
        "replay saved"
    );
    Ok(clip)
}

/// Bytes available to us on the file system holding `dir`, if it can be determined.
fn free_space(dir: &Path) -> Option<u64> {
    match rustix::fs::statvfs(dir) {
        Ok(st) => Some(st.f_bavail.saturating_mul(st.f_frsize)),
        Err(e) => {
            tracing::warn!("could not check free space in {}: {e}", dir.display());
            None
        }
    }
}
