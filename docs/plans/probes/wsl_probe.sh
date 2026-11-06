#!/bin/bash
# Verification probe: inode reuse and birth time on ext4 ($HOME) and drvfs (a test folder on a Windows drive).
# The drvfs folder is target/pg-test/probe-wsl in the checkout this script belongs to (run it from a checkout
# under /mnt/<drive>); PG_PROBE_DIR overrides the parent folder.
set -u

echo "== 1. Filesystems and devices"
echo "home fs: $(stat -f -c %T "$HOME")"
echo "home st_dev: $(stat -c %d "$HOME")"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
DRV="${PG_PROBE_DIR:-$ROOT/target/pg-test}/probe-wsl"
mkdir -p "$DRV"
rm -f "$DRV"/* 2>/dev/null
echo "drvfs fs: $(stat -f -c %T "$DRV")"
echo "drvfs st_dev: $(stat -c %d "$DRV")"

ext4=$HOME/pg-probe
rm -rf "$ext4"; mkdir -p "$ext4"

echo "== 2. ext4 inode reuse: delete A, create B at once, 500 cycles"
reuse=0; total=500
for i in $(seq 1 $total); do
  echo "a$i" > "$ext4/a.txt"; ia=$(stat -c %i "$ext4/a.txt")
  rm -f "$ext4/a.txt"
  echo "b$i" > "$ext4/b.txt"; ib=$(stat -c %i "$ext4/b.txt")
  rm -f "$ext4/b.txt"
  [ "$ia" = "$ib" ] && reuse=$((reuse + 1))
done
echo "ext4 immediate reuse (delete then create): $reuse of $total"

echo "== 3. ext4 reuse with other files present (more realistic: 200 files, delete one, create one)"
for i in $(seq 1 200); do echo "x$i" > "$ext4/keep$i.jpg"; done
reuse=0; total=200
for i in $(seq 1 $total); do
  echo "a$i" > "$ext4/victim.jpg"; ia=$(stat -c %i "$ext4/victim.jpg")
  rm -f "$ext4/victim.jpg"
  echo "b$i" > "$ext4/newcomer.jpg"; ib=$(stat -c %i "$ext4/newcomer.jpg")
  rm -f "$ext4/newcomer.jpg"
  [ "$ia" = "$ib" ] && reuse=$((reuse + 1))
done
echo "ext4 reuse with 200 neighbours: $reuse of $total"

echo "== 4. ext4 birth time (statx btime)"
echo "x" > "$ext4/born.jpg"; sleep 1
echo "birth: $(stat -c %w "$ext4/born.jpg")"
echo "mtime: $(stat -c %y "$ext4/born.jpg")"
echo "replace-by-rename over born.jpg:"
echo "y" > "$ext4/tmp.jpg"; sleep 1; mv "$ext4/tmp.jpg" "$ext4/born.jpg"
echo "  after rename-over birth: $(stat -c %w "$ext4/born.jpg")  inode: $(stat -c %i "$ext4/born.jpg")"
echo "in-place rewrite:"
b0=$(stat -c %w "$ext4/born.jpg"); i0=$(stat -c %i "$ext4/born.jpg")
sleep 1; echo "z" > "$ext4/born.jpg"
echo "  birth before $b0 after $(stat -c %w "$ext4/born.jpg")  inode before $i0 after $(stat -c %i "$ext4/born.jpg")"

echo "== 5. drvfs (NTFS through 9p): inode, birth, replace and reuse"
echo "d1" > "$DRV/a.jpg"; sleep 1
echo "  a.jpg inode $(stat -c %i "$DRV/a.jpg") birth $(stat -c %w "$DRV/a.jpg")"
echo "in-place rewrite:"
ia=$(stat -c %i "$DRV/a.jpg"); ba=$(stat -c %w "$DRV/a.jpg")
echo "d2" > "$DRV/a.jpg"
echo "  inode $ia -> $(stat -c %i "$DRV/a.jpg")  birth $ba -> $(stat -c %w "$DRV/a.jpg")"
echo "rename-over:"
echo "d3" > "$DRV/t.jpg"; sleep 1; mv "$DRV/t.jpg" "$DRV/a.jpg"
echo "  inode now $(stat -c %i "$DRV/a.jpg")  birth now $(stat -c %w "$DRV/a.jpg")"
echo "delete then create, 50 cycles:"
reuse=0
for i in $(seq 1 50); do
  ia=$(stat -c %i "$DRV/a.jpg"); rm -f "$DRV/a.jpg"
  echo "n$i" > "$DRV/a.jpg"; ib=$(stat -c %i "$DRV/a.jpg")
  [ "$ia" = "$ib" ] && reuse=$((reuse + 1))
done
echo "  drvfs immediate inode reuse: $reuse of 50"
echo "rename keeps inode:"
ir=$(stat -c %i "$DRV/a.jpg"); mv "$DRV/a.jpg" "$DRV/renamed.jpg"
echo "  before $ir after $(stat -c %i "$DRV/renamed.jpg")"

echo "== 6. drvfs device number for the probe folder (for the record)"
echo "st_dev now: $(stat -c %d "$DRV")"

rm -rf "$ext4"; rm -f "$DRV"/*
echo "== done"
