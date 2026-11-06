#!/bin/bash
# V3: which pair of paths is a guaranteed pair of distinct devices on any Linux?
# V4: state of the drvfs mounts (for the within-boot renumbering trigger).
echo "== V3 devices"
for p in / /proc /dev/shm /tmp /mnt/c /mnt/d; do
  printf '%-10s st_dev=%-6s fs=%s\n' "$p" "$(stat -c %d "$p" 2>/dev/null || echo missing)" "$(stat -f -c %T "$p" 2>/dev/null)"
done
echo "== V4 state"
echo "boot_id=$(cat /proc/sys/kernel/random/boot_id)"
echo "uptime_s=$(cut -d' ' -f1 /proc/uptime)"
echo "mountns=$(readlink /proc/self/ns/mnt)"
echo "st_dev /mnt/d=$(stat -c %d /mnt/d) /mnt/c=$(stat -c %d /mnt/c)"
echo "mountinfo rows for /mnt/d and /mnt/c:"
awk '$5=="/mnt/d" || $5=="/mnt/c"' /proc/self/mountinfo
echo "all 9p/drvfs mounts in this namespace:"
grep -E ' - (9p|v9fs) ' /proc/self/mountinfo
echo "dmesg lines mentioning 9p/drvfs (last 10):"
dmesg 2>&1 | grep -iE '9p|drvfs|v9fs' | tail -n 10
echo "== done"
