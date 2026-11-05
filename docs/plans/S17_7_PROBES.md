# S-17.7 probe results

The scripts that produced these results are in `docs/plans/probes/`. The S-17.7 plan cites them (`docs/plans/S17_7_PLAN.md`). Each result says what was measured and on what; nothing here is a claim beyond that.

## Verifications for the S-17.7 plan

Scripts: `docs/plans/probes/v1_symlink.ps1` (run elevated), `v1b_junction.ps1`, `v2_volumes.ps1`, `v34_wsl.sh`, `v4_drvfs_superblock.sh`.

- **V1, reparse root (a junction on D: to `C:\Users\Torres`; no elevation).** `GetVolumePathNameW` on the junction returns `C:\`, and `GetDriveTypeW` on it returns 3. So the path-based checks follow the link to its target's volume, and the file-system check passes (`NTFS`). Opening the link without following reports the reparse attribute, and its final path is the link's own `\\?\D:\...\v1-junction`. The check that refuses a reparse root is therefore the reparse check, not the volume checks. A symlink on D: to `\\localhost\D$\GitHub\photo-gallery\target\pg-test` (run elevated; `v1_symlink.ps1`): `GetVolumePathNameW` returns `D:\` (it does not follow a symlink), `GetDriveTypeW` returns 3 and the handle check reports `NTFS`. Only the final path shows the target, `\\?\UNC\localhost\D$\...`. So the volume checks pass a symlink to a remote share too. Refusal depends on the reparse check (order item 4) and the final-path check (item 5). Both stay in the plan.
- **V2, volumes (this machine).** Seven volumes report a 64-bit `FileIdInfo` serial, read from each volume's root handle. The lettered NTFS volumes all have low 32 bits equal to their 32-bit serial. No two volumes share the low 32 bits. `Z:` (FAT32) has serial `0x19831116`. A FAT32 volume's `FileIdInfo` read fails with error 87, so the identity read must come after the file-system check. Only this machine's volumes were checked.
- **V3, devices (WSL, Ubuntu-26.04).** `/` 2096, `/proc` 132, `/dev/shm` 137, `/tmp` 166, `/mnt/c` 159, `/mnt/d` 160: all distinct. `/` and `/proc` are separate mounts on any Linux, so they are a pair that does not depend on the CI image. Not checked: the GitHub runner's own `/dev/shm` and `/tmp`, which need a CI run.
- **V4, drvfs device numbers.** Boot id `229876ad` is the same as the earlier within-boot change, and `/mnt/d` is now 160 (it was 117 and 74 earlier under that boot id). A second drvfs mount of `D:` got device 180, and `/mnt/d` kept 160 through it. So each drvfs mount has its own device number, and a renumber means `/mnt/d`'s mount was replaced. What replaces it is still unknown. The temporary mount was unmounted, and `/mnt/d` is unchanged.

## Windows (NTFS on D:, the probe folder under `target\pg-test`)

- `GetVolumeInformationByHandleW` works on a directory listing handle with access 0 and with `FILE_LIST_DIRECTORY`. D: and C: report `NTFS`. `Z:` reports `FAT32`, serial `0x19831116`.
- `GetDriveTypeW` returns 3 (fixed) for D:, C: and `Z:`. A drive-type check does not catch `Z:`; the file-system name does.
- The 64-bit `FileIdInfo` volume serial on D: is `0x16cc492ecc490987`. Its low 32 bits equal `dwVolumeSerialNumber`, `0xcc490987`.
- The directory listing's `CreationTime` matches the file's creation time from a handle.
- In-place overwrite keeps the file ID and creation time. Save-by-rename (`MoveFileEx` over the file) and `File.Replace` each give a new file ID and a new creation time.
- Delete and recreate under the same name gives a new file ID each time (5 of 5). Rename keeps the file ID and creation time.
- UNC (`\\localhost\D$\`, an admin share on this machine): `GetVolumePathNameW` returns `\\localhost\D$\`; `GetDriveTypeW` returns 4 (remote); `GetVolumeInformationByHandleW` still reports `NTFS`. So the NTFS name check alone does not refuse an SMB share; the drive-type check does, and it must run.
- Zero file IDs on local NTFS: 0 of 400,000 non-dot entries had file ID 0 (folders under `D:\GitHub`, `C:\Users\Torres`, and `C:\Windows\System32`, capped at 400,000 entries). 19 folders could not be opened (access denied), 16 reparse folders were seen. This is a sample, not an exhaustive scan of the drives.

## WSL2 (Ubuntu-26.04)

- `/` is ext2/ext3 (ext4 on the virtual disk). `/mnt/c` and `/mnt/d` are `9p` (drvfs).
- ext4 inode reuse: the inode of a just-deleted file was reused every time: 500 of 500, and 200 of 200 with 200 neighbouring files present.
- ext4 birth time (`statx`): changes on rename-over, does not change on in-place rewrite.
- drvfs has no birth time: `stat -c %w` prints `-`. `created` is NULL for drvfs files.
- drvfs inode reuse on delete-then-create: 0 of 50, and 0 of 2000. The 95% upper bound for a 2000-cycle run is about 0.15% per cycle. This is evidence against reuse, not proof that it cannot happen.
- Distinct devices have distinct `st_dev`: `/dev/shm` 75, `/tmp` 133, `$HOME` 2096, `/mnt/c` 115, `/mnt/d` 117 (this boot).
- Device renumbering across boots: `/mnt/d` was 214 before one restart and 146 after it; then 146 → 74 after a later restart; then 74 → 74 after the next. `/mnt/c` followed the same pattern (145, 73).
- Device renumbering within one VM boot: `/mnt/d` went from 74 to 117 (and `/mnt/c` from 73 to 115) with the same boot id (`229876ad`). Stopping and restarting `docker-desktop` did not change the numbers. The trigger is unknown; the window also included Windows directory scans of D: and C:, which were not isolated.
