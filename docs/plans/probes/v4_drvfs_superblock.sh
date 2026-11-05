#!/bin/bash
# V4 experiment (root): does a new drvfs mount of D: get a new st_dev? Temporary mount point, removed afterwards.
# Does not touch /mnt/d.
set -u
mp=/mnt/pgprobe-d
mkdir -p "$mp"
echo "before: /mnt/d st_dev=$(stat -c %d /mnt/d)"
if mount -t drvfs D: "$mp" 2>/tmp/pg_mount_err; then
  echo "second mount: $mp st_dev=$(stat -c %d "$mp")  /mnt/d st_dev=$(stat -c %d /mnt/d)"
  grep -E " $mp | /mnt/d " /proc/self/mountinfo
  umount "$mp" && echo "unmounted $mp"
else
  echo "mount failed: $(cat /tmp/pg_mount_err)"
fi
rm -f /tmp/pg_mount_err
rmdir "$mp" 2>/dev/null
echo "after: /mnt/d st_dev=$(stat -c %d /mnt/d)"
echo "== done"
