//! Captures a real window (glxgears) through the full GPU path into NVENC segments.
//! Needs an X11 session, an NVIDIA GPU and `glxgears` (mesa-utils):
//! `cargo test -p chronoclip-capture -- --ignored`.
//! Set `CHRONOCLIP_TEST_OUT=/some/dir` to keep the segments.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use chronoclip_capture::{
    CaptureStatus, CudaDestination, WindowMatcher, X11WindowCapture, find_window,
};
use chronoclip_encode::{EncoderConfig, NvencEncoder, SegmentWriter};

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "needs X11, an NVIDIA GPU and glxgears"]
fn captures_glxgears_window() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .with_test_writer()
        .try_init();
    let _gears = KillOnDrop(
        Command::new("glxgears")
            .args(["-geometry", "1280x720"])
            .spawn()?,
    );

    let matcher = WindowMatcher::TitleContains("glxgears".into());
    let deadline = Instant::now() + Duration::from_secs(5);
    let window = loop {
        if let Some(w) = find_window(&matcher)? {
            break w;
        }
        assert!(Instant::now() < deadline, "glxgears window did not appear");
        std::thread::sleep(Duration::from_millis(100));
    };
    std::thread::sleep(Duration::from_millis(500)); // let it map and draw

    let tmp = tempfile::tempdir()?;
    let dir = std::env::var_os("CHRONOCLIP_TEST_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| tmp.path().to_owned());
    std::fs::create_dir_all(&dir)?;

    // The encoder owns the CUDA context; create it first at the window's size.
    let fps = 60;
    let mut encoder = NvencEncoder::new(EncoderConfig {
        width: window.width,
        height: window.height,
        fps,
        bitrate_bps: 10_000_000,
        gop_frames: 2 * fps,
    })?;
    let mut capture = X11WindowCapture::new(window.id, encoder.cuda_context())?;
    let (w, h) = capture.size();
    assert_eq!((w, h), (window.width, window.height));

    let mut writer = SegmentWriter::new(
        &dir,
        encoder.parameters()?,
        encoder.time_base(),
        Duration::from_secs(2),
        0,
    );
    let mut segments = Vec::new();
    let mut packets = Vec::new();
    let frames = 3 * 60;
    let start = Instant::now();
    let mut capture_time = Duration::ZERO;
    for n in 0..frames {
        let frame = encoder.alloc_frame()?;
        let t = Instant::now();
        let status = capture.capture(CudaDestination {
            context: encoder.cuda_context(),
            device_ptr: frame.device_ptr(),
            pitch: frame.pitch(),
        })?;
        capture_time += t.elapsed();
        assert_eq!(status, CaptureStatus::Captured);
        encoder.encode(Some((frame, n)), &mut packets)?;
        for p in packets.drain(..) {
            segments.extend(writer.write(p)?);
        }
        // Pace roughly at 60 fps.
        let target = Duration::from_micros(16_667) * (n as u32 + 1);
        if let Some(wait) = target.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    encoder.encode(None, &mut packets)?;
    for p in packets.drain(..) {
        segments.extend(writer.write(p)?);
    }
    segments.extend(writer.finish()?);

    eprintln!(
        "captured {frames} frames of {w}x{h}, avg capture+copy {:.3} ms, {} segments",
        capture_time.as_secs_f64() * 1000.0 / f64::from(frames as u32),
        segments.len()
    );
    assert_eq!(segments.len(), 2);
    assert!(segments.iter().all(|s| s.bytes > 1000));
    Ok(())
}
