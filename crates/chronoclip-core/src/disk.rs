//! Disk usage estimates and buffer location checks.

use std::path::Path;

/// Typical bitrate for 1080p H.264/HEVC at the given frame rate, in Mbit/s.
///
/// Only used for the estimate shown in the UI until real encoder settings exist.
pub fn nominal_bitrate_mbps(fps: u32) -> u32 {
    match fps {
        0..=30 => 12,
        31..=60 => 20,
        61..=120 => 35,
        _ => 40,
    }
}

/// Target bitrate in bits/s for a capture of `width`x`height` at `fps`, scaled from the
/// 1080p nominal rate (sub-linearly, since encoders get more efficient at higher resolutions).
pub fn bitrate_bps(width: u32, height: u32, fps: u32) -> u64 {
    let pixels = f64::from(width.max(1)) * f64::from(height.max(1));
    let scale = (pixels / (1920.0 * 1080.0)).powf(0.75);
    let mbps = f64::from(nominal_bitrate_mbps(fps)) * scale;
    (mbps.clamp(2.0, 80.0) * 1e6) as u64
}

/// `m:ss` (or `h:mm:ss` from one hour).
pub fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Human-readable size: MB below 1 GB, otherwise GB.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1_000_000_000 {
        format!("{} MB", bytes / 1_000_000)
    } else {
        format_gb(bytes)
    }
}

/// Bytes needed to hold `minutes` of video at `bitrate_mbps`.
pub fn estimate_buffer_bytes(minutes: u32, bitrate_mbps: u32) -> u64 {
    u64::from(minutes) * 60 * u64::from(bitrate_mbps) * 1_000_000 / 8
}

/// Human-readable size in GB (decimal, like file managers on Linux).
pub fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Returns a warning if `dir` is likely on a RAM-backed tmpfs, which a multi-GB buffer must avoid.
pub fn tmpfs_warning(dir: &Path, runtime_dir: Option<&Path>) -> Option<&'static str> {
    let ram_backed = ["/tmp", "/dev/shm", "/run"]
        .iter()
        .map(Path::new)
        .chain(runtime_dir)
        .any(|prefix| dir.starts_with(prefix));
    ram_backed.then_some(
        "This folder is often stored in RAM (tmpfs). A long buffer needs several GB, so pick a folder on your SSD.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_matches_readme() {
        // 10 min at 20 Mbit/s ≈ 1.5 GB, 45 min ≈ 6.8 GB
        assert_eq!(format_gb(estimate_buffer_bytes(10, 20)), "1.5 GB");
        assert_eq!(format_gb(estimate_buffer_bytes(45, 20)), "6.8 GB");
    }

    #[test]
    fn bitrate_grows_with_fps() {
        assert!(nominal_bitrate_mbps(30) < nominal_bitrate_mbps(60));
        assert!(nominal_bitrate_mbps(60) < nominal_bitrate_mbps(144));
    }

    #[test]
    fn bitrate_scales_with_resolution() {
        assert_eq!(bitrate_bps(1920, 1080, 60), 20_000_000);
        let qhd = bitrate_bps(2560, 1440, 60);
        assert!((28_000_000..34_000_000).contains(&qhd), "{qhd}");
        assert!(bitrate_bps(3840, 2160, 144) <= 80_000_000);
        assert!(bitrate_bps(320, 200, 30) >= 2_000_000);
    }

    #[test]
    fn formats_durations_and_sizes() {
        use std::time::Duration;
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_secs(200)), "3:20");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1:02:05");
        assert_eq!(format_size(512_400_000), "512 MB");
        assert_eq!(format_size(2_700_000_000), "2.7 GB");
    }

    #[test]
    fn warns_about_tmpfs() {
        let runtime = Path::new("/run/user/1000");
        assert!(tmpfs_warning(Path::new("/tmp/chronoclip"), Some(runtime)).is_some());
        assert!(tmpfs_warning(Path::new("/dev/shm/x"), None).is_some());
        assert!(tmpfs_warning(Path::new("/run/user/1000/buf"), Some(runtime)).is_some());
        assert!(
            tmpfs_warning(
                Path::new("/home/me/.cache/chronoclip/buffer"),
                Some(runtime)
            )
            .is_none()
        );
        // `/tmpfoo` is not inside `/tmp`
        assert!(tmpfs_warning(Path::new("/tmpfoo"), None).is_none());
    }
}
