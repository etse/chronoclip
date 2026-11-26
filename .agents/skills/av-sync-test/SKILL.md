---
name: av-sync-test
description: Measure audio/video sync of Chronoclip clips with a flash-and-beep test window (once per second), recorded by an isolated instance and analyzed frame by frame. Use after changes to capture, audio timeline, encoding, segment writing or remuxing, or to verify a milestone's A/V sync requirement.
---

# A/V sync test

`scripts/sync-test.sh` starts a test window (ffplay) that turns white for 0.1 s and beeps
(1 kHz, 0.1 s) at the start of every second, records it with an isolated Chronoclip instance in
*Game only* mode (via the test-instance skill), saves a clip and runs `scripts/analyze_sync.py`.
The beeps go to a silent null sink: the maintainer hears nothing, and their YouTube or Discord
stays out of the clip.

```sh
S=.agents/skills/av-sync-test/scripts/sync-test.sh
$S                                  # 2 min: quick check after a change (~2.5 min)
$S --minutes 10                     # longer drift check
$S --minutes 46.5 --buffer 45       # full-length clip (~47 min: run it in the background)
$S --minutes 4 --resize-at 120      # sync right after a window resize
```

Output and logs land in `target/cc-sync/` (`result.txt`, `app.log`, `clips/`). Analyze any clip
with `python3 -I .agents/skills/av-sync-test/scripts/analyze_sync.py CLIP`.

## Reading the result

```
    whole clip: n=   45  mean   +0.2 ms  min  -20.4  median   +1.6  max  +15.0
```

Offset = beep onset − flash onset; positive means the sound is late.

- **Good:** median within ±1 frame (±17 ms at 60 fps), spread about −30…+15 ms, and the
  first-minute and last-minute means within a few ms of each other (no drift).
- The picture is sampled at 60 Hz, so ±1 frame of spread is inherent. ffplay's own A/V sync adds
  a few ms, and more on a null sink (its latency estimate is off), so look at **drift** more than
  at the absolute offset.
- `flash gaps` / `beep gaps` ≠ 1 s mean dropped frames or audio gaps. One short gap at the very
  start or end is the clip boundary.
- One unmatched flash is normal (the last frame of the clip).

## Caveats

- **A clip starts at the last window resize** (one MP4 can't change resolution). With
  `--resize-at`, the clip is only the part after the resize; that's by design, not a bug.
- The null sink runs on the system clock, so this test can't show drift from a sound card
  whose clock runs fast or slow. For that, record with a real device (ask the maintainer: they
  would hear the beeps).
- Long runs take as long as they record. Run them in the background and keep working.

Record results in `docs/architecture.md` (the audio measurements table) and, for a milestone
task, as a sub-bullet in `docs/roadmap.md` (see the milestone skill).
