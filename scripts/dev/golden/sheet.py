"""Side-by-side frames: our game (left) and the original's capture (right) at
the same ticks of a golden scenario.

    source env.sh && python -I scripts/dev/golden/sheet.py NAME TICK [TICK...]
        [--crop X0,Y0,X1,Y1] [--scale 0.25]

NAME is a scenario already run by scripts/dev/golden/run.sh (it needs
scratch/golden/NAME/{NAME.rep,capture.mkv,cmp.csv}). The original's frame for
a tick comes from the alignment in cmp.csv (the frame aligned to the closest
tick). Our frames: `bopl-port --headless --replay`. Output:
scratch/golden/NAME/sheet-<ticks>.png (scratch only: it shows game art).
"""
import argparse
import csv
import os
import subprocess
import sys

from PIL import Image

R = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("name")
    ap.add_argument("ticks", nargs="+", type=int)
    ap.add_argument("--crop", default=None)
    ap.add_argument("--scale", type=float, default=0.25)
    ap.add_argument("--fps", type=float, default=60.0, help="capture frame rate")
    a = ap.parse_args()
    d = os.path.join(R, "scratch", "golden", a.name)
    rep = os.path.join(d, a.name + ".rep")
    rows = list(csv.DictReader(open(os.path.join(d, "cmp.csv"))))
    if not rows:
        sys.exit("empty cmp.csv")
    frame_of = {}
    for t in a.ticks:
        best = min(rows, key=lambda r: abs(int(r["tick"]) - t))
        # Outside the aligned range (before the first sighting, after the
        # last): extrapolate at real time (60 ticks per second of capture).
        k = int(best["frame"]) + round((t - int(best["tick"])) * float(a.fps) / 60.0)
        frame_of[t] = (k, int(best["tick"]))
    tmp = os.path.join(d, "sheet-tmp")
    os.makedirs(tmp, exist_ok=True)
    shots = ",".join(str(t) for t in a.ticks)
    subprocess.run(
        [os.path.join(R, "target", "release", "bopl-port"), "--headless", "--replay", rep,
         "--shots", shots, "--screenshot", os.path.join(tmp, "ours.png"),
         "--ticks", str(max(a.ticks) + 1)],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    crop = tuple(int(v) for v in a.crop.split(",")) if a.crop else (0, 0, 1920, 1080)
    cw, ch = crop[2] - crop[0], crop[3] - crop[1]
    tw, th = int(cw * a.scale), int(ch * a.scale)
    sheet = Image.new("RGB", (2 * tw, th * len(a.ticks)))
    for i, t in enumerate(a.ticks):
        k, kt = frame_of[t]
        orig = os.path.join(tmp, f"orig-{t}.png")
        subprocess.run(
            ["ffmpeg", "-loglevel", "error", "-y", "-ss", f"{k / 60:.4f}",
             "-i", os.path.join(d, "capture.mkv"), "-frames:v", "1", orig], check=True)
        ours = Image.open(os.path.join(tmp, f"ours-{t}.png")).convert("RGB")
        sheet.paste(ours.crop(crop).resize((tw, th)), (0, i * th))
        sheet.paste(Image.open(orig).convert("RGB").crop(crop).resize((tw, th)), (tw, i * th))
        print(f"tick {t}: original frame {k} (aligned to tick {kt})")
    out = os.path.join(d, "sheet-" + "-".join(str(t) for t in a.ticks[:4]) + ".png")
    sheet.save(out)
    print(out)


if __name__ == "__main__":
    main()
