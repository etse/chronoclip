//! The short sounds played after a save, synthesized in code (no sound files to ship).

use std::f32::consts::TAU;

/// Sample rate of the generated sounds (mono, signed 16-bit).
pub const SAMPLE_RATE: u32 = 48_000;

/// Peak level relative to full scale: clearly audible over game sound without being harsh.
const LEVEL: f32 = 0.35;

/// One bell-like note: a sine with a little second harmonic, a 4 ms fade-in and an
/// exponential decay that reaches silence at the end (no click).
fn note(frequency: f32, millis: u32, out: &mut Vec<f32>) {
    let n = (SAMPLE_RATE * millis / 1000) as usize;
    let attack = (SAMPLE_RATE as usize * 4) / 1000;
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let fade_in = (i as f32 / attack as f32).min(1.0);
        let progress = i as f32 / n as f32;
        // exp decay to ~2 %, then a short linear fade to exactly zero.
        let decay = (-4.0 * progress).exp() * (1.0 - progress).min(0.1) * 10.0;
        let tone = (TAU * frequency * t).sin() + 0.25 * (TAU * 2.0 * frequency * t).sin();
        out.push(tone / 1.25 * fade_in * decay);
    }
}

fn to_pcm(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|s| (s * LEVEL * f32::from(i16::MAX)).round() as i16)
        .collect()
}

/// "Clip saved": two rising notes (C6 → G6), about a quarter second.
pub fn saved() -> Vec<i16> {
    let mut samples = Vec::new();
    note(1046.5, 110, &mut samples);
    note(1568.0, 170, &mut samples);
    to_pcm(&samples)
}

/// "Save failed": two falling, lower notes (A4 → E4).
pub fn failed() -> Vec<i16> {
    let mut samples = Vec::new();
    note(440.0, 140, &mut samples);
    note(329.6, 200, &mut samples);
    to_pcm(&samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(samples: &[i16], min_ms: u32, max_ms: u32) {
        let ms = samples.len() as u32 * 1000 / SAMPLE_RATE;
        assert!((min_ms..=max_ms).contains(&ms), "length {ms} ms");
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        let limit = (LEVEL * f32::from(i16::MAX)) as u16 + 1;
        assert!(peak > limit / 2, "too quiet: {peak}");
        assert!(peak <= limit, "too loud: {peak}");
        // Starts and ends silent, so there is no click.
        assert!(samples.first().is_some_and(|s| s.unsigned_abs() < 50));
        assert!(samples.last().is_some_and(|s| s.unsigned_abs() < 50));
    }

    #[test]
    fn saved_chime_is_short_and_clean() {
        check(&saved(), 250, 320);
    }

    #[test]
    fn failed_tone_is_short_and_clean() {
        check(&failed(), 300, 380);
    }

    #[test]
    fn sounds_differ() {
        assert_ne!(saved(), failed());
    }
}
