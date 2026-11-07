"""The video reader's test input (crates/video/tests/sample.mp4): a short synthetic clip, made by the FFmpeg on PATH (or
FFMPEG, when set).

    python tools/make_test_video.py crates/video/tests/sample.mp4

The clip is 64x48, six frames at 6 fps (one second), H.264 in MP4, with no audio. The encoder runs bit-exact, so the file
is the same on every run with the same encoder. The tests copy it under names FFmpeg can't open by path (a non-UTF-8
name) and read it through the reader.
"""

import argparse
import os
import subprocess
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    ffmpeg = os.environ.get("FFMPEG", "ffmpeg")
    subprocess.run(
        [
            ffmpeg,
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=6:duration=1",
            "-c:v",
            "libx264",
            "-preset",
            "veryslow",
            "-crf",
            "30",
            "-pix_fmt",
            "yuv420p",
            "-fflags",
            "+bitexact",
            "-flags:v",
            "+bitexact",
            "-movflags",
            "+faststart",
            "-an",
            str(args.out),
        ],
        check=True,
    )


if __name__ == "__main__":
    main()
