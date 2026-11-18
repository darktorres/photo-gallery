"""The reader the decoder scripts share for the reference PPMs: the pixels every decoder's output is compared with.

Imported by debesheedas.py, gpu_encoders.py and nvimgcodec_bench.py, which run from their own venvs with this folder on
sys.path. Needs only numpy and Pillow.
"""

import numpy as np
from PIL import Image


def read_ppm(path):
    """The pixels of `path` as RGB: uint8, shape (h, w, 3). A grey or palette file is converted, so it reads as the
    three channels the comparison needs; anything else with a different shape is an error here, not a size string
    reported by the comparison later."""
    pixels = np.ascontiguousarray(np.asarray(Image.open(path).convert("RGB")))
    if pixels.dtype != np.uint8 or pixels.ndim != 3 or pixels.shape[2] != 3:
        raise ValueError(f"{path}: expected RGB pixels of shape (h, w, 3), got {pixels.dtype} {pixels.shape}")
    return pixels
