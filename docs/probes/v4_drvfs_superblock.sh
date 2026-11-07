#!/bin/bash
# V4 experiment (run as root): does a new drvfs mount of a Windows drive get a new st_dev? Temporary mount point,
# removed afterwards. Does not touch the existing /mnt/<drive> mount. PG_PROBE_DRIVE selects the drive (default D).
set -u
DRIVE="${PG_PROBE_DRIVE:-D}"
base="/mnt/$(echo "$DRIVE" | tr 'A-Z' 'a-z')"
mp=/mnt/pgprobe-d
err=$(mktemp)
mkdir -p "$mp"
echo "before: $base st_dev=$(stat -c %d "$base")"
if mount -t drvfs "$DRIVE:" "$mp" 2>"$err"; then
  echo "second mount: $mp st_dev=$(stat -c %d "$mp")  $base st_dev=$(stat -c %d "$base")"
  grep -E " $mp | $base " /proc/self/mountinfo
  umount "$mp" && echo "unmounted $mp"
else
  echo "mount failed: $(cat "$err")"
fi
rm -f "$err"
rmdir "$mp" 2>/dev/null
echo "after: $base st_dev=$(stat -c %d "$base")"
echo "== done"
