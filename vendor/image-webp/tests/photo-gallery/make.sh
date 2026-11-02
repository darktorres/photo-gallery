#!/bin/sh
# photo-gallery: makes this directory's WebP files, read by `decoder::photo_gallery` (src/decoder.rs), with
# ImageMagick 7 (libwebp 1.5.0 made the ones here). Run from this directory. Odd sizes, so that macroblocks and
# subsampled images are cut at the edges.
set -eu
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

# Pictures: a plasma (noise and gradients), with a radial alpha; and few-colour ones for the palettes.
magick -seed 7 -size 61x37 plasma:red-blue "$t/small.png"
magick -seed 8 -size 203x151 plasma:green-yellow "$t/large.png"
magick -seed 9 -size 77x45 plasma: \( -size 77x45 radial-gradient:white-black \) -alpha off -compose copy_opacity \
    -composite "$t/alpha.png"
for n in 2 4 16 200; do
    magick "$t/large.png" -dither none -colors "$n" "$t/colors$n.png"
done

# Lossy: one partition and eight, segments, both loop filters, sharp YUV.
magick "$t/small.png" -quality 75 -define webp:partitions=0 lossy_small.webp
magick "$t/large.png" -quality 60 -define webp:partitions=3 -define webp:segments=4 -define webp:filter-type=1 \
    -define webp:filter-strength=60 -define webp:filter-sharpness=3 lossy_large_strong.webp
magick "$t/large.png" -quality 90 -define webp:partitions=2 -define webp:filter-type=0 \
    -define webp:filter-strength=40 -define webp:use-sharp-yuv=true lossy_large_simple.webp
# Lossy with alpha: the plane compressed (lossless, each filter libwebp picks) and not.
for f in 0 1 2; do
    magick "$t/alpha.png" -quality 70 -define webp:alpha-compression=1 -define webp:alpha-filtering=$f \
        "lossy_alpha_filtering$f.webp"
done
magick "$t/alpha.png" -quality 70 -define webp:alpha-compression=0 lossy_alpha_raw.webp

# Lossless: a photo (predictor, colour and subtract-green transforms, colour cache, meta Huffman codes), the
# palettes (1, 2, 4 and 8 bits an index), and alpha.
magick "$t/large.png" -define webp:lossless=true -define webp:method=6 -define webp:image-hint=photo \
    lossless_photo.webp
magick "$t/small.png" -define webp:lossless=true -define webp:method=0 lossless_fast.webp
for n in 2 4 16 200; do
    magick "$t/colors$n.png" -define webp:lossless=true "lossless_colors$n.webp"
done
magick "$t/alpha.png" -define webp:lossless=true -define webp:exact=true lossless_alpha.webp

# Animations: frames of other sizes, disposed and blended, lossy and lossless. ImageMagick puts every frame at the
# origin and makes them all of one kind: the test builds animations at offsets and of mixed kinds from the stills.
magick "$t/alpha.png" -crop 30x20+5+5 +repage "$t/part1.png"
magick "$t/small.png" -crop 25x15+20+10 +repage "$t/part2.png"
magick "$t/colors4.png" -crop 40x24+0+0 +repage "$t/part3.png"
magick "$t/alpha.png" -crop 61x37+0+0 +repage "$t/last.png"
frames() {
    magick -delay 10 -dispose none "$t/small.png" \
        -dispose background "$t/part1.png" \
        -dispose none "$t/part2.png" \
        -dispose none "$t/part3.png" \
        -dispose previous "$t/last.png" \
        -loop 3 "$@"
}
frames -quality 70 anim_lossy.webp
frames -define webp:lossless=true anim_lossless.webp

