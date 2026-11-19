//! End-to-end: generated frames → NVENC → MPEG-TS segments. Needs an NVIDIA GPU, so it is
//! ignored by default: `cargo test -p chronoclip-encode -- --ignored`.
//! Set `CHRONOCLIP_TEST_OUT=/some/dir` to keep the segments for inspection.

use std::path::PathBuf;
use std::time::Duration;

use chronoclip_core::segments::Segment;
use chronoclip_encode::{EncoderConfig, NvencEncoder, SegmentWriter, remux_to_mp4};

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

/// Encode `frames` frames in a fresh encoder session (timestamps start at 0 again), like the
/// recorder does after the window was hidden and shown.
fn encode_session(
    dir: &std::path::Path,
    first_seq: u64,
    frames: u32,
) -> Result<Vec<Segment>, Box<dyn std::error::Error>> {
    let mut encoder = NvencEncoder::new(EncoderConfig {
        width: W,
        height: H,
        fps: FPS,
        bitrate_bps: 8_000_000,
        gop_frames: 2 * FPS,
    })?;
    let mut writer = SegmentWriter::new(
        dir,
        encoder.parameters()?,
        encoder.time_base(),
        Duration::from_secs(2),
        0,
        first_seq,
    );
    let mut segments = Vec::new();
    let mut packets = Vec::new();
    for n in 0..frames {
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
    Ok(segments)
}

#[test]
#[ignore = "needs an NVIDIA GPU with NVENC"]
fn remuxes_segments_into_one_mp4() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let dir = std::env::var_os("CHRONOCLIP_TEST_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| tmp.path().to_owned());
    std::fs::create_dir_all(&dir)?;

    // 5 s, then a second session of 3 s whose timestamps restart at 0.
    let mut segments = encode_session(&dir, 0, 5 * FPS)?;
    let next = segments.last().map_or(0, |s| s.seq + 1);
    segments.extend(encode_session(&dir, next, 3 * FPS)?);
    let paths: Vec<PathBuf> = segments.iter().map(|s| s.path.clone()).collect();

    let clip = dir.join("clip.mp4");
    let stats = remux_to_mp4(&paths, &clip)?;
    eprintln!("remuxed {} segments: {stats:?}", paths.len());
    assert_eq!(stats.skipped_segments, 0);
    assert_eq!(stats.bytes, std::fs::metadata(&clip)?.len());
    let expected = Duration::from_secs(8);
    assert!(
        stats.duration.abs_diff(expected) < Duration::from_millis(50),
        "duration {:?}",
        stats.duration
    );
    // Stream copy: the clip is about as large as the segments (MPEG-TS overhead is ~5-10 %).
    let segment_bytes: u64 = segments.iter().map(|s| s.bytes).sum();
    assert!(stats.bytes < segment_bytes && stats.bytes > segment_bytes * 8 / 10);

    // Read it back: MP4, H.264, every frame present, strictly increasing timestamps.
    let mut input = ffmpeg_next::format::input(&clip)?;
    assert!(input.format().name().contains("mp4"));
    let stream = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or("no video stream")?;
    assert_eq!(stream.parameters().id(), ffmpeg_next::codec::Id::H264);
    let (index, time_base) = (stream.index(), stream.time_base());
    let container = Duration::from_micros(u64::try_from(input.duration())?);
    assert!(
        container.abs_diff(expected) < Duration::from_millis(50),
        "{container:?}"
    );
    let mut count = 0;
    let mut last_dts = None;
    let mut keyframes = 0;
    for (st, packet) in input.packets() {
        if st.index() != index {
            continue;
        }
        let dts = packet.dts().ok_or("packet without dts")?;
        if count == 0 {
            assert!(packet.is_key());
            assert_eq!(dts, 0);
        }
        assert!(
            last_dts.is_none_or(|last| dts > last),
            "dts not increasing at {count}"
        );
        last_dts = Some(dts);
        keyframes += usize::from(packet.is_key());
        count += 1;
    }
    assert_eq!(count, 8 * FPS);
    // One keyframe per 2 s segment: 3 + 2.
    assert_eq!(keyframes, 5);
    // The last frame sits just before 8 s.
    let last = Duration::from_secs_f64(
        last_dts.unwrap_or(0) as f64 * f64::from(time_base.numerator())
            / f64::from(time_base.denominator()),
    );
    assert!(
        last.abs_diff(Duration::from_millis(7_983)) < Duration::from_millis(20),
        "{last:?}"
    );
    Ok(())
}
