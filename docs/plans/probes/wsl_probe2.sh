#!/bin/bash
# Larger drvfs reuse sample, and device numbers of real, distinct devices.
set -u
DRV=/mnt/d/GitHub/photo-gallery/target/pg-test/probe-wsl2
mkdir -p "$DRV"
echo "drvfs fs: $(stat -f -c %T "$DRV")"
reuse=0; total=2000
echo a > "$DRV/a.jpg"
for i in $(seq 1 $total); do
  ia=$(stat -c %i "$DRV/a.jpg")
  rm -f "$DRV/a.jpg"
  echo n > "$DRV/a.jpg"
  ib=$(stat -c %i "$DRV/a.jpg")
  [ "$ia" = "$ib" ] && reuse=$((reuse + 1))
done
echo "drvfs immediate inode reuse: $reuse of $total"
rm -f "$DRV/a.jpg"
rmdir "$DRV"
echo "st_dev /dev/shm=$(stat -c %d /dev/shm) /tmp=$(stat -c %d /tmp) home=$(stat -c %d "$HOME") /mnt/c=$(stat -c %d /mnt/c) /mnt/d=$(stat -c %d /mnt/d)"
echo "mounts:"
grep -E " /dev/shm | /tmp | /mnt/c | /mnt/d " /proc/mounts
echo "== done"
