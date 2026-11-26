"""Measure A/V offset in a clip with a white flash and a beep at the start of every second.

Usage: python3 -I analyze_sync.py CLIP
Offset = beep onset - flash onset (positive: sound later than picture). Needs ffmpeg/ffprobe.
"""
import json
import re
import subprocess
import sys

clip = sys.argv[1]


def run(args):
    return subprocess.run(args, capture_output=True, text=True, check=True)


probe = json.loads(run(["ffprobe", "-v", "error", "-show_entries",
                        "format=duration:stream=codec_type,codec_name,width,height:stream_tags=title",
                        "-of", "json", clip]).stdout)
print("streams:", [(s["codec_type"], s["codec_name"], s.get("tags", {}).get("title")) for s in probe["streams"]])
print("duration: %.3f s" % float(probe["format"]["duration"]))

# Video: mean luma per frame (downscaled for speed); a flash starts at the first bright frame.
out = run(["ffmpeg", "-hide_banner", "-nostats", "-i", clip, "-map", "0:v:0", "-vf",
           "scale=32:18,signalstats,metadata=mode=print:key=lavfi.signalstats.YAVG:file=-",
           "-f", "null", "-"]).stdout
flashes, frames, bright, t = [], 0, False, None
for line in out.splitlines():
    m = re.search(r"pts_time:([\d.]+)", line)
    if m:
        t = float(m.group(1))
        frames += 1
        continue
    m = re.search(r"YAVG=([\d.]+)", line)
    if m and t is not None:
        b = float(m.group(1)) > 128
        if b and not bright:
            flashes.append(t)
        bright = b

# Audio: beep onsets = ends of silence (sample accurate).
err = run(["ffmpeg", "-hide_banner", "-nostats", "-i", clip, "-map", "0:a:0", "-af",
           "aformat=channel_layouts=mono,silencedetect=noise=-30dB:duration=0.3", "-f", "null", "-"]).stderr
beeps = [float(x) for x in re.findall(r"silence_end: ([\d.]+)", err)]
print(f"video frames: {frames}, flashes: {len(flashes)}, beeps: {len(beeps)}")

pairs, j = [], 0
for f in flashes:
    while j + 1 < len(beeps) and abs(beeps[j + 1] - f) <= abs(beeps[j] - f):
        j += 1
    if beeps and abs(beeps[j] - f) < 0.4:
        pairs.append((f, (beeps[j] - f) * 1000))


def stats(name, sel):
    if not sel:
        print(f"{name:>14}: no data")
        return
    offs = sorted(o for _, o in sel)
    mean = sum(offs) / len(offs)
    print(f"{name:>14}: n={len(offs):5}  mean {mean:+6.1f} ms  min {offs[0]:+6.1f}  "
          f"median {offs[len(offs) // 2]:+6.1f}  max {offs[-1]:+6.1f}")


end = flashes[-1] if flashes else 0
stats("whole clip", pairs)
stats("first 60 s", [p for p in pairs if p[0] < 60])
stats("last 60 s", [p for p in pairs if p[0] > end - 60])
for minute in range(5, int(end // 60) + 1, 5):
    stats(f"minute {minute}", [p for p in pairs if minute * 60 <= p[0] < minute * 60 + 60])
print(f"unmatched flashes: {len(flashes) - len(pairs)}")
gaps = [round(b - a, 3) for a, b in zip(flashes, flashes[1:]) if abs(b - a - 1) > 0.05]
print(f"flash gaps != 1 s: {len(gaps)} {gaps[:10]}")
bgaps = [round(b - a, 3) for a, b in zip(beeps, beeps[1:]) if abs(b - a - 1) > 0.05]
print(f"beep gaps != 1 s: {len(bgaps)} {bgaps[:10]}")
