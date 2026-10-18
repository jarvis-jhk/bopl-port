"""Refines a golden comparison's frame→tick alignment by matching whole frames
of the original's capture against our game's renders of the same replay.

    source env.sh && python -I scripts/dev/golden/refine.py NAME [--window 20]

Needs scratch/golden/NAME/{NAME.rep, capture.mkv, cmp.csv} (run.sh first).
The blob-based alignment (golden compare) is only as good as the motion in
view; while players stand still it can drift by several ticks. Here every
captured frame is compared (mean absolute difference at 240×135, HUD corners
excluded) with our renders of the ticks around the first alignment, and a
monotonic path through those costs (0..8 ticks per frame, small penalty for
leaving real time) is written to scratch/golden/NAME/map.csv; run
`golden compare ... --map map.csv` with it (run.sh --refine does both).
Presentation differences (trails, particles, HUD) raise all costs equally, so
the minimum still marks the tick. Our renders go to a temporary folder in
scratch/ and are deleted afterwards.
"""
import argparse
import csv
import os
import shutil
import subprocess

from PIL import Image, ImageChops, ImageStat

R = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
SW, SH = 240, 135
CROP = (12, 8, 240, 112)  # skip the "?" button and the circles row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("name")
    ap.add_argument("--window", type=int, default=20)
    ap.add_argument("--fps", type=float, default=60.0)
    a = ap.parse_args()
    d = os.path.join(R, "scratch", "golden", a.name)
    rows = [(int(r["frame"]), int(r["tick"])) for r in csv.DictReader(open(os.path.join(d, "cmp.csv")))]
    if not rows:
        raise SystemExit("empty cmp.csv")
    k0, k1 = rows[0][0], rows[-1][0]
    est = {}
    j = 0
    for k in range(k0, k1 + 1):
        while j + 1 < len(rows) and rows[j + 1][0] <= k:
            j += 1
        fk, ft = rows[j]
        est[k] = ft + round((k - fk) * 60.0 / a.fps) if k >= fk else ft
    t0 = max(1, min(est.values()) - a.window)
    t1 = max(est.values()) + a.window
    tmp = os.path.join(d, "refine-tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    ticks = list(range(t0, t1 + 1))
    subprocess.run(
        [os.path.join(R, "target", "release", "bopl-port"), "--headless", "--replay",
         os.path.join(d, a.name + ".rep"), "--size", f"{SW * 2}x{SH * 2}",
         "--shots", ",".join(map(str, ticks)), "--screenshot", os.path.join(tmp, "o.png"),
         "--ticks", str(t1 + 1)],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ours = {}
    for t in ticks:
        p = os.path.join(tmp, f"o-{t}.png")
        if os.path.exists(p):
            ours[t] = Image.open(p).convert("RGB").resize((SW, SH)).crop(CROP)
    shutil.rmtree(tmp, ignore_errors=True)
    n = k1 - k0 + 1
    raw = subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-ss", f"{k0 / a.fps:.4f}", "-i", os.path.join(d, "capture.mkv"),
         "-frames:v", str(n), "-vf", f"scale={SW}:{SH}", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        capture_output=True, check=True).stdout
    fs = SW * SH * 3
    costs = {}
    for i in range(min(n, len(raw) // fs)):
        k = k0 + i
        im = Image.frombytes("RGB", (SW, SH), raw[i * fs:(i + 1) * fs]).crop(CROP)
        c = {}
        for t in range(est[k] - a.window, est[k] + a.window + 1):
            if t in ours:
                c[t] = sum(ImageStat.Stat(ImageChops.difference(im, ours[t])).mean) / 3
        costs[k] = c
    # Monotonic path (dynamic programming over frames).
    ks = sorted(costs)
    lam = 0.05 * 60.0 / a.fps
    best = {t: (v, None) for t, v in costs[ks[0]].items()}
    back = [best]
    for k in ks[1:]:
        prev = back[-1]
        cur = {}
        for t, v in costs[k].items():
            cand = [(pv[0] + lam * abs((t - pt) - 60.0 / a.fps), pt) for pt, pv in prev.items() if 0 <= t - pt <= 8]
            if cand:
                pc, pt = min(cand)
                cur[t] = (pc + v, pt)
        if not cur:  # gap: restart from the best previous
            pt = min(prev, key=lambda x: prev[x][0])
            cur = {t: (prev[pt][0] + v, pt) for t, v in costs[k].items()}
        back.append(cur)
    t = min(back[-1], key=lambda x: back[-1][x][0])
    path = []
    for i in range(len(ks) - 1, -1, -1):
        path.append((ks[i], t))
        t = back[i][t][1] if back[i][t][1] is not None else t
    path.reverse()
    out = os.path.join(d, "map.csv")
    with open(out, "w") as f:
        f.write("frame,tick\n")
        for k, t in path:
            f.write(f"{k},{t}\n")
    moved = sum(1 for k, t in path if abs(t - est[k]) > 2)
    print(f"{out}: {len(path)} frames; {moved} moved by more than 2 ticks from the first alignment")


if __name__ == "__main__":
    main()
