#!/usr/bin/env python3
"""Checks the FFmpeg pin (the Justfile's `ffmpeg_url`) against BtbN's retention policy and against the static build's commit.
`just lint` runs it.

BtbN keeps the last 14 daily builds, and the last build of each month for two years (its README, "Release Retention
Policy"). So the pin must be a month-end build, and it stops being fetchable two years after its date: a daily build is
refused, and so is one at or past its expiry. A pin expiring within 90 days is a warning, since it is time to repin. The
build's commit must also be the commit tools/build_static_ffmpeg.py builds from, so CI's FFmpeg and the shipped static
build cannot drift apart.
"""

import datetime
import re
import sys

from pgtools import REPO, just_value

# autobuild-YYYY-MM-DD-HH-MM: the tag BtbN names a build with.
TAG = re.compile(r"autobuild-(\d{4})-(\d{2})-(\d{2})-\d{2}-\d{2}")
# The commit in the asset's name, as in ffmpeg-n9.0.2-39-g7adfbe40dc-win64-gpl-shared-9.0.zip.
ASSET_COMMIT = re.compile(r"-g([0-9a-f]{7,40})-")
STATIC_COMMIT = re.compile(r'^FFMPEG_COMMIT = "([0-9a-f]{40})"', re.MULTILINE)
WARN_WITHIN = datetime.timedelta(days=90)


def expiry(built: datetime.date) -> datetime.date:
    """Two years after `built`, when BtbN drops a month-end build. 29 February has no date two years on: 28 February."""
    try:
        return built.replace(year=built.year + 2)
    except ValueError:
        return built.replace(year=built.year + 2, day=28)


def check(url: str, static_source: str, today: datetime.date) -> tuple[list[str], list[str]]:
    """The problems with the pin as (errors, warnings). `url` is the Justfile's ffmpeg_url, `static_source` is the text of
    tools/build_static_ffmpeg.py, and `today` is the date the pin is judged on."""
    tag = TAG.search(url)
    if not tag:
        return [f"ffmpeg_url names no autobuild tag: {url}"], []
    built = datetime.date(*(int(tag.group(i)) for i in (1, 2, 3)))
    errors, warnings = [], []
    if (built + datetime.timedelta(days=1)).month == built.month:
        errors.append(
            f"the FFmpeg pin is the build of {built}, not of the last day of its month: BtbN removes every other build "
            "after 14 days. Pin the month-end build."
        )
    gone = expiry(built)
    if today >= gone:
        errors.append(f"the FFmpeg pin expired on {gone}: BtbN removes it. Pin a newer month-end build.")
    elif gone - today <= WARN_WITHIN:
        warnings.append(f"the FFmpeg pin expires on {gone}: repin to a newer month-end build.")
    asset = ASSET_COMMIT.search(url.rsplit("/", 1)[-1])
    static = STATIC_COMMIT.search(static_source)
    if not asset or not static:
        errors.append("the pin's asset name has no commit, or tools/build_static_ffmpeg.py has no FFMPEG_COMMIT.")
    elif not static.group(1).startswith(asset.group(1)):
        errors.append(
            f"the pin's commit {asset.group(1)} is not the static build's FFMPEG_COMMIT {static.group(1)}: CI and the "
            "shipped static build would run different FFmpeg source."
        )
    return errors, warnings


def main() -> None:
    url = just_value("ffmpeg_url")
    static = (REPO / "tools" / "build_static_ffmpeg.py").read_text(encoding="utf-8")
    errors, warnings = check(url, static, datetime.datetime.now(tz=datetime.UTC).date())
    for line in warnings:
        print(f"warning: {line}")
    if errors:
        for line in errors:
            print(f"error: {line}", file=sys.stderr)
        sys.exit(1)
    print("FFmpeg pin: ok")


if __name__ == "__main__":
    main()
