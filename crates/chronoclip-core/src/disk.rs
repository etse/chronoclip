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
