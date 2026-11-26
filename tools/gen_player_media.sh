#!/bin/sh
# Player test bed: generates every fixture the video-player spikes need into one directory, then checks the
# properties each file exists for (colour tags, HDR metadata, display matrix, Dolby Vision record, channels).
# docs/plans/VIDEO_PLAYER.md ("Research → Test media") says why each case is made the way it is.
#
#   sh tools/gen_player_media.sh OUT_DIR
#
# Tools: PG_FFMPEG (the ffmpeg executable; ffprobe is taken from the same folder), default our pinned
# BtbN gpl-shared 9.0 build on D:. PG_DOVI_TOOL (dovi_tool 2.3.x) is optional: without it the Dolby Vision
# fixtures are skipped (reported, not failed). Needs the GPL build (libx264, libx265, libxvid).
# Exit status: the number of failed steps and checks (0 = all good).
set -u
set -f # no globbing: the lavfi expressions below contain '*'

[ $# -eq 1 ] || { echo "usage: $0 OUT_DIR" >&2; exit 2; }
FFMPEG=${PG_FFMPEG:-/d/PhotoGallery/tools/ffmpeg-9.0/bin/ffmpeg.exe}
FFPROBE=$(dirname "$FFMPEG")/ffprobe$(case $FFMPEG in *.exe) echo .exe;; esac)
DOVI=${PG_DOVI_TOOL:-}
mkdir -p "$1" && cd "$1" || exit 2
"$FFMPEG" -hide_banner -version | head -n 1
SVT_LOG=1; export SVT_LOG # SVT-AV1: errors only
FAIL=0
START=$(date +%s)

ff() { "$FFMPEG" -hide_banner -nostdin -loglevel error -y "$@"; }
# r NAME CMD...: run one step; a failure (or an empty output) is counted and the rest goes on.
r() { n=$1; shift; if "$@" && [ -s "$n" ]; then echo "ok   $n"; else echo "FAIL $n"; FAIL=$((FAIL + 1)); fi; }
# half SRC DST: the first half of a file (a truncated download).
half() { sz=$(wc -c < "$1"); dd if="$1" of="$2" bs=1024 count=$((sz / 2048)) 2>/dev/null; }

# Sources: 3 s each. Bars for HDR (flat fields), testsrc2 (motion, colour) for the rest.
V='-f lavfi -i testsrc2=s=640x360:r=30:d=3'
H='-f lavfi -i smptehdbars=s=1280x720:r=24:d=3'
A='-f lavfi -i sine=f=440:r=48000:d=3'
A441='-f lavfi -i sine=f=440:r=44100:d=3'
# One tone per channel, so a wrong channel map is audible and measurable (LFE at 50 Hz).
A51='-f lavfi -i aevalsrc=sin(300*2*PI*t)|sin(400*2*PI*t)|sin(500*2*PI*t)|0.3*sin(50*2*PI*t)|sin(600*2*PI*t)|sin(700*2*PI*t):c=5.1:s=48000:d=3'
A71='-f lavfi -i aevalsrc=sin(300*2*PI*t)|sin(400*2*PI*t)|sin(500*2*PI*t)|0.3*sin(50*2*PI*t)|sin(600*2*PI*t)|sin(700*2*PI*t)|sin(800*2*PI*t)|sin(900*2*PI*t):c=7.1:s=48000:d=3'

# Colour: FFmpeg 9's ffmpeg negotiates matrix and range through the filter graph (so -colorspace and
# -color_range convert the pixels and tag them), but copies primaries and transfer from the frames
# (fftools/ffmpeg_enc.c), so -color_primaries/-color_trc on the output are ignored: set those on the frames.
# col PIXFMT RANGE MATRIX PRIMARIES TRANSFER -> the option list for one tagged picture.
col() { echo "-vf setparams=color_primaries=$4:color_trc=$5 -pix_fmt $1 -colorspace $3 -color_range $2"; }
C709=$(col yuv420p tv bt709 bt709 bt709)
C709_10=$(col yuv420p10le tv bt709 bt709 bt709)
CPQ=$(col yuv420p10le tv bt2020nc bt2020 smpte2084)
CHLG=$(col yuv420p10le tv bt2020nc bt2020 arib-std-b67)
CHLG8=$(col yuv420p tv bt2020nc bt2020 arib-std-b67)
# Untagged: the pixels converted with a matrix, no tags written (what a player has to guess).
untag() { echo "-vf scale=out_color_matrix=$1:out_range=tv,format=yuv420p,setparams=range=unknown:color_primaries=unknown:color_trc=unknown:colorspace=unknown"; }

X264='-c:v libx264 -preset ultrafast'
X265='-c:v libx265 -preset ultrafast -tag:v hvc1'
XQ='log-level=error'
MD='G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)'
HDR10P="$XQ:hdr10=1:repeat-headers=1:master-display=$MD:max-cll=1000,400"
# x265's Dolby Vision profile 8 needs VBV (it refuses without), on top of the HDR10 settings.
DVP="$HDR10P:vbv-bufsize=20000:vbv-maxrate=20000"

echo "--- HEVC, AV1, VP9 (SDR)"
r hevc_main8.mp4  ff $V $C709    $X265 -x265-params $XQ hevc_main8.mp4
r hevc_main10.mp4 ff $V $C709_10 $X265 -x265-params $XQ hevc_main10.mp4
r av1_svt_8.mp4   ff $V $C709    -c:v libsvtav1 -preset 12 av1_svt_8.mp4
r av1_svt_10.mkv  ff $V $C709_10 -c:v libsvtav1 -preset 12 av1_svt_10.mkv
r av1_aom_10.webm ff $V $C709_10 -c:v libaom-av1 -cpu-used 8 -usage realtime av1_aom_10.webm
r av1_rav1e_8.mp4 ff $V $C709    -c:v librav1e -speed 10 av1_rav1e_8.mp4
r vp9_p2_10.webm  ff $V $C709_10 -c:v libvpx-vp9 -deadline realtime -cpu-used 8 vp9_p2_10.webm

echo "--- HDR10"
# x265's master-display/max-cll go into the bitstream's SEI only; the container gets mdcv/clli (Matroska
# MasteringMetadata) when ffmpeg sees them on decoded frames. So: one master, every HDR10 file encoded from it.
r hdr10_master.hevc ff $H $CPQ -c:v libx265 -preset ultrafast -x265-params "$HDR10P" -f hevc hdr10_master.hevc
r hdr10_hevc.mp4  ff -i hdr10_master.hevc $X265 -x265-params "$HDR10P" hdr10_hevc.mp4
r hdr10_hevc.mkv  ff -i hdr10_hevc.mp4 -c copy hdr10_hevc.mkv
r hdr10_av1.mkv   ff -i hdr10_master.hevc -c:v libsvtav1 -preset 12 hdr10_av1.mkv
r hdr10_av1.mp4   ff -i hdr10_master.hevc -c:v libsvtav1 -preset 12 hdr10_av1.mp4
r hdr10_vp9.webm  ff -i hdr10_master.hevc -c:v libvpx-vp9 -profile:v 2 -deadline realtime -cpu-used 8 hdr10_vp9.webm
# The bitstream-only case (container carries no mdcv/clli): what a direct x265 encode gives.
r hdr10_sei_only.mp4 ff $H $CPQ $X265 -x265-params "$HDR10P" hdr10_sei_only.mp4

echo "--- HLG"
r hlg_hevc10.mp4  ff $H $CHLG  $X265 -x265-params "$XQ:atc-sei=18:pic-struct=0" hlg_hevc10.mp4
r hlg_h264_8.mp4  ff $H $CHLG8 $X264 hlg_h264_8.mp4

echo "--- SDR matrices and ranges"
r sdr_601_525.mp4 ff $V $(col yuv420p tv smpte170m smpte170m smpte170m) $X264 sdr_601_525.mp4
# 625-line: BT.470BG primaries and matrix; transfer BT.601 (as broadcast tags it), not BT.470BG's gamma 2.8.
r sdr_601_625.mp4 ff $V $(col yuv420p tv bt470bg bt470bg smpte170m) $X264 sdr_601_625.mp4
r sdr_709.mp4     ff $V $C709 $X264 sdr_709.mp4
r sdr_2020_10.mp4 ff $V $(col yuv420p10le tv bt2020nc bt2020 bt2020-10) $X265 -x265-params $XQ sdr_2020_10.mp4
r untagged_sd.mp4 ff -f lavfi -i testsrc2=s=720x480:r=30:d=3  $(untag bt601) $X264 untagged_sd.mp4
r untagged_hd.mp4 ff -f lavfi -i testsrc2=s=1280x720:r=30:d=3 $(untag bt709) $X264 untagged_hd.mp4
r full_709.mp4    ff $V $(col yuv420p pc bt709 bt709 bt709) $X264 full_709.mp4
r full_601_mjpeg.avi ff $V -pix_fmt yuvj420p -c:v mjpeg -q:v 3 full_601_mjpeg.avi

echo "--- Rotation and mirror (display matrix; input options, and only with -c copy: a re-encode applies it)"
r base.mp4 ff $V $A $X264 -c:a aac -shortest base.mp4
for a in 90 180 270; do
  r rot$a.mp4 ff -display_rotation $a -i base.mp4 -c copy rot$a.mp4
  r rot$a.mov ff -display_rotation $a -i base.mp4 -c copy rot$a.mov
done
r hflip.mp4       ff -display_hflip -i base.mp4 -c copy hflip.mp4
r vflip.mov       ff -display_vflip -i base.mp4 -c copy vflip.mov
r rot90_hflip.mp4 ff -display_rotation 90 -display_hflip -i base.mp4 -c copy rot90_hflip.mp4
r rot90_hflip.mov ff -display_rotation 90 -display_hflip -i base.mp4 -c copy rot90_hflip.mov

echo "--- Audio"
r a51_aac.mp4      ff $V $A51 $X264 -c:a aac -b:a 384k -shortest a51_aac.mp4
r a71_aac.mp4      ff $V $A71 $X264 -c:a aac -b:a 512k -shortest a71_aac.mp4
r a51_ac3.mp4      ff $V $A51 $X264 -c:a ac3 -shortest a51_ac3.mp4
r a51_eac3.mp4     ff $V $A51 $X264 -c:a eac3 -shortest a51_eac3.mp4
r a51_opus.mkv     ff $V $A51 $X264 -c:a libopus -shortest a51_opus.mkv
r a71_opus.webm    ff $V $A71 -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -c:a libopus -shortest a71_opus.webm
r a71_opus.mp4     ff $V $A71 $X264 -c:a libopus -shortest a71_opus.mp4
r mono_aac44.mp4   ff $V $A441 $X264 -c:a aac -shortest mono_aac44.mp4
r stereo_aac44.mp4 ff $V $A441 $X264 -c:a aac -ac 2 -shortest stereo_aac44.mp4
r stereo_aac48.mp4 ff $V $A $X264 -c:a aac -ac 2 -shortest stereo_aac48.mp4
r noaudio.mp4      ff $V $X264 noaudio.mp4
# (E-AC-3 7.1 is not made: FFmpeg's eac3 encoder stops at 5.1.)

echo "--- Containers"
r c.mkv          ff $V $A $X264 -c:a libvorbis -shortest c.mkv
r c_xvid_mp3.avi ff $V $A -c:v libxvid -q:v 4 -c:a libmp3lame -shortest c_xvid_mp3.avi
r c.wmv          ff $V $A -c:v wmv2 -b:v 1M -c:a wmav2 -shortest c.wmv
r c.mpg          ff $V $A -c:v mpeg2video -b:v 2M -c:a mp2 -shortest -f mpeg c.mpg
r c.ts           ff $V $A -c:v mpeg2video -b:v 2M -c:a mp2 -shortest c.ts
r c.m4v          ff $V $A $X264 -c:a aac -shortest -f ipod c.m4v
r c.3gp          ff -f lavfi -i testsrc2=s=352x288:r=15:d=3 -f lavfi -i sine=f=440:r=8000:d=3 -c:v h263 \
                    -c:a libopencore_amrnb -ar 8000 -ac 1 -b:a 12.2k -shortest c.3gp
r c.webm         ff $V $A -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -c:a libopus -shortest c.webm

echo "--- Edge cases"
r trunc_src.mp4    ff -f lavfi -i testsrc2=s=640x360:r=30:d=10 $X264 trunc_src.mp4
r trunc_fs_src.mp4 ff -f lavfi -i testsrc2=s=640x360:r=30:d=10 $X264 -movflags +faststart trunc_fs_src.mp4
r trunc_nomoov.mp4    half trunc_src.mp4 trunc_nomoov.mp4       # moov at the end: no index at all
r trunc_faststart.mp4 half trunc_fs_src.mp4 trunc_faststart.mp4 # moov first: a playable prefix
r trunc.mkv           half c.mkv trunc.mkv
rm -f trunc_src.mp4 trunc_fs_src.mp4
# VFR: 30 fps for 1 s, then every third frame (10 fps).
r vfr.mp4          ff $V -vf "select='lt(n\,30)+not(mod(n\,3))'" -fps_mode vfr $X264 vfr.mp4
# Video starts 0.5 s after audio (an empty edit), audio from 0.
r delay_video.mp4  ff -itsoffset 0.5 $V $A $X264 -c:a aac delay_video.mp4
# B-frames: a composition-offset edit list, the usual camera case.
r bframes_edts.mp4 ff $V -c:v libx264 -preset medium -bf 3 bframes_edts.mp4

echo "--- Dolby Vision (8.1, 8.4, AV1 profile 10)"
if [ -n "$DOVI" ] && "$DOVI" --version >/dev/null 2>&1; then
  # The RPU's length is the frame count (3 s at 24 fps).
  printf '%s\n' '{ "cm_version": "V40", "profile": "8.1", "length": 72, "level6": { "max_display_mastering_luminance": 1000, "min_display_mastering_luminance": 1, "max_content_light_level": 1000, "max_frame_average_light_level": 400 } }' > dv81.json
  printf '%s\n' '{ "cm_version": "V40", "profile": "8.4", "length": 72 }' > dv84.json
  r dv81.rpu "$DOVI" generate -j dv81.json -o dv81.rpu
  r dv84.rpu "$DOVI" generate -j dv84.json -o dv84.rpu
  r hlg_master.hevc ff $H $CHLG -c:v libx265 -preset ultrafast -x265-params "$XQ:repeat-headers=1:atc-sei=18:pic-struct=0" -f hevc hlg_master.hevc
  r dv81_rpu.hevc "$DOVI" inject-rpu -i hdr10_master.hevc --rpu-in dv81.rpu -o dv81_rpu.hevc
  r dv84_rpu.hevc "$DOVI" inject-rpu -i hlg_master.hevc   --rpu-in dv84.rpu -o dv84_rpu.hevc
  # A remux of the Annex B stream gets no configuration record; a re-encode writes it (dvcC/dvvC, Matroska
  # BlockAddIDExtraData) and carries the RPU over. Encode straight to each container.
  r dv81.mp4 ff -i dv81_rpu.hevc $X265 -dolbyvision 1 -strict unofficial -x265-params "$DVP" dv81.mp4
  r dv81.mkv ff -i dv81_rpu.hevc $X265 -dolbyvision 1 -strict unofficial -x265-params "$DVP" dv81.mkv
  r dv84.mp4 ff -i dv84_rpu.hevc $X265 -dolbyvision 1 -strict unofficial \
                -x265-params "$XQ:repeat-headers=1:atc-sei=18:pic-struct=0:vbv-bufsize=20000:vbv-maxrate=20000" dv84.mp4
  r dv10_av1.mkv ff -i dv81_rpu.hevc -c:v libsvtav1 -preset 12 -dolbyvision 1 -strict unofficial dv10_av1.mkv
  r dv10_av1.mp4 ff -i dv81_rpu.hevc -c:v libsvtav1 -preset 12 -dolbyvision 1 -strict unofficial dv10_av1.mp4
  rm -f dv81.json dv84.json dv81.rpu dv84.rpu dv81_rpu.hevc dv84_rpu.hevc hlg_master.hevc
else
  echo "skip Dolby Vision: set PG_DOVI_TOOL to dovi_tool"
fi

echo "--- Checks"
# chk FILE ENTRIES EXPECTED: ffprobe's first video (or v/a per ENTRIES' stream spec) values, compact form.
chk() {
  f=$1; sel=$2; ent=$3; want=$4
  [ -f "$f" ] || { echo "MISS $f"; FAIL=$((FAIL + 1)); return; }
  got=$("$FFPROBE" -v error -select_streams "$sel" -show_entries "$ent" -of csv=p=0:nk=1 "$f" 2>&1 | tr -d '\r' | tr '\n' ' ' | sed 's/ *$//')
  case $got in *"$want"*) echo "ok   $f: $want";; *) echo "BAD  $f: want '$want', got '$got'"; FAIL=$((FAIL + 1));; esac
}
# ffprobe prints fields in its own order, whatever the order asked: range, matrix, transfer, primaries.
CT=stream=color_range,color_space,color_transfer,color_primaries
chk hevc_main10.mp4  v:0 stream=codec_name,profile,pix_fmt 'hevc,Main 10,yuv420p10le'
chk av1_svt_10.mkv   v:0 stream=codec_name,pix_fmt 'av1,yuv420p10le'
chk vp9_p2_10.webm   v:0 stream=codec_name,profile 'vp9,Profile 2'
chk sdr_709.mp4      v:0 $CT 'tv,bt709,bt709,bt709'
chk sdr_601_525.mp4  v:0 $CT 'tv,smpte170m,smpte170m,smpte170m'
chk sdr_601_625.mp4  v:0 $CT 'tv,bt470bg,smpte170m,bt470bg'
chk sdr_2020_10.mp4  v:0 $CT 'tv,bt2020nc,bt2020-10,bt2020'
chk full_709.mp4     v:0 $CT 'pc,bt709,bt709,bt709'
chk full_601_mjpeg.avi v:0 stream=pix_fmt 'yuvj420p'
chk untagged_hd.mp4  v:0 $CT 'unknown,unknown,unknown,unknown'
chk untagged_sd.mp4  v:0 $CT 'unknown,unknown,unknown,unknown'
chk hlg_hevc10.mp4   v:0 $CT 'tv,bt2020nc,arib-std-b67,bt2020'
chk hlg_h264_8.mp4   v:0 $CT 'tv,bt2020nc,arib-std-b67,bt2020'
for f in hdr10_hevc.mp4 hdr10_hevc.mkv hdr10_av1.mkv hdr10_av1.mp4 hdr10_vp9.webm; do
  chk $f v:0 $CT 'tv,bt2020nc,smpte2084,bt2020'
  chk $f v:0 stream_side_data=side_data_type 'Mastering display metadata'
  chk $f v:0 stream_side_data=max_content 1000
done
# The SEI-only file: tags yes, container metadata no (ffprobe prints nothing for the side data).
chk hdr10_sei_only.mp4 v:0 $CT 'tv,bt2020nc,smpte2084,bt2020'
if [ -n "$("$FFPROBE" -v error -select_streams v:0 -show_entries stream_side_data=side_data_type -of csv=p=0 hdr10_sei_only.mp4 | tr -d '\r\n')" ]; then
  echo "BAD  hdr10_sei_only.mp4: the container has HDR metadata"; FAIL=$((FAIL + 1))
else echo "ok   hdr10_sei_only.mp4: no container HDR metadata"; fi
for a in 90 180 270; do chk rot$a.mp4 v:0 stream_side_data=rotation "$(( a > 180 ? a - 360 : a ))"; chk rot$a.mov v:0 stream_side_data=rotation "$(( a > 180 ? a - 360 : a ))"; done
chk hflip.mp4        v:0 stream_side_data=side_data_type 'Display Matrix'
chk rot90_hflip.mov  v:0 stream_side_data=side_data_type 'Display Matrix'
chk a51_aac.mp4      a:0 stream=channels,channel_layout '6,5.1'
chk a71_aac.mp4      a:0 stream=channels,channel_layout '8,7.1'
chk a51_eac3.mp4     a:0 stream=codec_name,channels 'eac3,6'
chk a71_opus.webm    a:0 stream=codec_name,channels 'opus,8'
chk mono_aac44.mp4   a:0 stream=sample_rate,channels '44100,1'
chk c.3gp            v:0 stream=codec_name 'h263'
chk c.m4v            v:0 stream=codec_name 'h264'
chk vfr.mp4          v:0 stream=nb_frames '50'
if [ -f dv81.mp4 ]; then
  for f in dv81.mp4 dv81.mkv; do chk $f v:0 stream_side_data=dv_profile,dv_bl_signal_compatibility_id '8,1'; done
  chk dv84.mp4 v:0 stream_side_data=dv_profile,dv_bl_signal_compatibility_id '8,4'
  for f in dv10_av1.mkv dv10_av1.mp4; do chk $f v:0 stream_side_data=dv_profile '10'; done
fi

END=$(date +%s)
set +f; n=0; for f in *; do [ -f "$f" ] && n=$((n + 1)); done
echo "done: $n files, $(du -sk . | cut -f1) KiB, $((END - START)) s, $FAIL failed"
exit $FAIL
