//! End-to-end: generated frames → NVENC → MPEG-TS segments. Needs an NVIDIA GPU, so it is
//! ignored by default: `cargo test -p chronoclip-encode -- --ignored`.
//! Set `CHRONOCLIP_TEST_OUT=/some/dir` to keep the segments for inspection.

use std::path::PathBuf;
use std::time::Duration;

use chronoclip_encode::{EncoderConfig, NvencEncoder, SegmentWriter};

const W: u32 = 1280;
const H: u32 = 720;
const FPS: u32 = 60;

/// A moving gradient with a white bar so motion is visible.
fn test_image(frame: u32) -> Vec<u8> {
    let mut img = vec![0u8; (W * H * 4) as usize];
    let bar = (frame * 8) % W;
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let on_bar = x.abs_diff(bar) < 20;
            img[i] = if on_bar { 255 } else { (x * 255 / W) as u8 };
            img[i + 1] = if on_bar { 255 } else { (y * 255 / H) as u8 };
            img[i + 2] = if on_bar { 255 } else { 128 };
        }
    }
    img
}

#[test]
#[ignore = "needs an NVIDIA GPU with NVENC"]
fn encodes_keyframe_aligned_segments() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let dir = std::env::var_os("CHRONOCLIP_TEST_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| tmp.path().to_owned());
    std::fs::create_dir_all(&dir)?;

    let mut encoder = NvencEncoder::new(EncoderConfig {
        width: W,
        height: H,
        fps: FPS,
        bitrate_bps: 8_000_000,
        gop_frames: 2 * FPS,
    })?;
    assert!(!encoder.cuda_context().is_null());

    let mut writer = SegmentWriter::new(
        &dir,
        encoder.parameters()?,
        encoder.time_base(),
        Duration::from_secs(2),
        0,
    );

    let mut segments = Vec::new();
    let mut packets = Vec::new();
    let total_frames = 7 * FPS; // 7 s → segments of 2 + 2 + 2 + 1 s
    for n in 0..total_frames {
        let frame = encoder.upload_rgb0(&test_image(n), (W * 4) as usize)?;
        encoder.encode(Some((frame, i64::from(n))), &mut packets)?;
        for p in packets.drain(..) {
            segments.extend(writer.write(p)?);
        }
    }
    encoder.encode(None, &mut packets)?;
    for p in packets.drain(..) {
        segments.extend(writer.write(p)?);
    }
    segments.extend(writer.finish()?);

    let durations: Vec<_> = segments.iter().map(|s| s.duration).collect();
    assert_eq!(
        durations,
        vec![
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_secs(1)
        ]
    );
    for (i, s) in segments.iter().enumerate() {
        assert_eq!(s.seq, i as u64);
        assert!(s.path.exists());
        assert!(
            s.bytes > 10_000,
            "segment {i} suspiciously small: {}",
            s.bytes
        );
    }

    // Every segment must be independently decodable: first packet is a keyframe.
    for s in &segments {
        let mut input = ffmpeg_next::format::input(&s.path)?;
        let stream = input
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or("no video stream")?;
        let params = stream.parameters();
        assert_eq!(params.id(), ffmpeg_next::codec::Id::H264);
        let index = stream.index();
        let (_, first) = input
            .packets()
            .find(|(st, _)| st.index() == index)
            .ok_or("no packets")?;
        assert!(
            first.is_key(),
            "{} does not start with a keyframe",
            s.path.display()
        );
    }
    Ok(())
}
