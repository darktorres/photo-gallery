"""Candidate descriptors for the bake-off. Each loader returns a module mapping
(B, 3, H, W) -> (B, D); outputs are L2-normalized by the caller.

All models see the *whole* image resized to a square (no center crop): center-cropping
would throw away exactly the borders that distinguish crops of the same picture.
"""

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import torch
import torchvision
from torch import nn
from torchvision import transforms as T

from common import CACHE_DIR

IMAGENET_MEAN = (0.485, 0.456, 0.406)
IMAGENET_STD = (0.229, 0.224, 0.225)
CLIP_MEAN = (0.48145466, 0.4578275, 0.40821073)
CLIP_STD = (0.26862954, 0.26130258, 0.27577711)


@dataclass(frozen=True)
class ModelSpec:
    size: int
    mean: tuple[float, float, float]
    std: tuple[float, float, float]
    interpolation: T.InterpolationMode
    # "fp32" or "amp" (fp16 autocast). Only regular modules get "amp": autocast doesn't
    # reach inside TorchScript, and casting TorchScript with .half() breaks SSCD.
    precision: str
    load: Callable[[], nn.Module]


def _sscd_torchscript_path(variant: str) -> Path:
    path = CACHE_DIR / "torch" / f"{variant}.torchscript.pt"
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        url = f"https://dl.fbaipublicfiles.com/sscd-copy-detection/{variant}.torchscript.pt"
        torch.hub.download_url_to_file(url, str(path))
    return path


def _sscd_torchscript(variant: str) -> Callable[[], nn.Module]:
    """Meta's published TorchScript, as-is. Reference only: casting it to fp16 with
    .half() silently breaks the traced backbone (relative error ~1.0)."""
    def load() -> nn.Module:
        return torch.jit.load(str(_sscd_torchscript_path(variant)), map_location="cpu")

    return load


class GeM(nn.Module):
    """Generalized-mean pooling. Always computed in fp32: x**3 overflows fp16 for
    activations above ~40."""

    def __init__(self, p: float = 3.0, eps: float = 1e-6):
        super().__init__()
        self.p = p
        self.eps = eps

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        x = x.float().clamp(min=self.eps).pow(self.p)
        return x.mean(dim=(2, 3)).pow(1.0 / self.p)


class SSCD(nn.Module):
    """SSCD ResNet50 rebuilt as a plain module: torchvision ResNet50 trunk -> GeM ->
    linear projection -> L2 normalization. Unlike the TorchScript it runs under fp16
    autocast and exports cleanly to ONNX."""

    def __init__(self, dim: int):
        super().__init__()
        # resnet.avgpool and resnet.fc exist but are never called (and have no SSCD weights).
        self.resnet = torchvision.models.resnet50()
        self.pool = GeM()
        self.project = nn.Linear(2048, dim)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        r = self.resnet
        x = r.maxpool(r.relu(r.bn1(r.conv1(x))))
        x = r.layer4(r.layer3(r.layer2(r.layer1(x))))
        x = self.project(self.pool(x))
        return nn.functional.normalize(x.float(), dim=1)


# ClassyVision bottleneck layout -> torchvision names.
_CLASSY_CONV = {"0": "conv1", "1": "bn1", "3": "conv2", "4": "bn2", "6": "conv3"}


def _classy_to_torchvision(state: dict[str, torch.Tensor]) -> dict[str, torch.Tensor]:
    out = {}
    for key, value in state.items():
        if key.endswith("num_batches_tracked"):
            continue
        if key.startswith("embeddings.1."):
            out["project." + key.rsplit(".", 1)[1]] = value
            continue
        key = key.removeprefix("backbone.")
        if m := re.fullmatch(r"initial_block\._module\.([01])\.(\w+)", key):
            out[f"resnet.{'conv1' if m[1] == '0' else 'bn1'}.{m[2]}"] = value
            continue
        m = re.fullmatch(r"blocks\.(\d)\.block\d-(\d+)\.(.+)", key)
        if not m:
            raise ValueError(f"unmapped SSCD weight: {key}")
        rest = re.sub(r"^convolutional_block\.(\d)\.", lambda c: _CLASSY_CONV[c[1]] + ".", m[3])
        rest = re.sub(r"^bn\.", "bn3.", rest)
        out[f"resnet.layer{int(m[1]) + 1}.{m[2]}.{rest}"] = value
    return out


def _sscd(variant: str) -> Callable[[], nn.Module]:
    def load() -> nn.Module:
        state = torch.jit.load(str(_sscd_torchscript_path(variant)), map_location="cpu").state_dict()
        model = SSCD(dim=state["embeddings.1.weight"].shape[0])
        missing, unexpected = model.load_state_dict(_classy_to_torchvision(state), strict=False)
        missing = [k for k in missing if not k.endswith("num_batches_tracked") and not k.startswith("resnet.fc.")]
        if missing or unexpected:
            raise ValueError(f"SSCD weight mapping incomplete: missing={missing} unexpected={unexpected}")
        return model

    return load


def _timm(name: str, **kwargs) -> Callable[[], nn.Module]:
    def load() -> nn.Module:
        import timm

        return timm.create_model(name, pretrained=True, num_classes=0, **kwargs)

    return load


class _ClipImageEncoder(nn.Module):
    def __init__(self, model: nn.Module):
        super().__init__()
        self.model = model

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        return self.model.encode_image(x)


def _open_clip(arch: str, pretrained: str) -> Callable[[], nn.Module]:
    def load() -> nn.Module:
        import open_clip

        model, _, _ = open_clip.create_model_and_transforms(arch, pretrained=pretrained)
        return _ClipImageEncoder(model)

    return load


BILINEAR = T.InterpolationMode.BILINEAR
BICUBIC = T.InterpolationMode.BICUBIC

SPECS: dict[str, ModelSpec] = {
    # SSCD (Meta, copy detection). 320x320 "skew" resize is one of the two inference
    # transforms recommended in its README; bilinear matches its default.
    "sscd": ModelSpec(320, IMAGENET_MEAN, IMAGENET_STD, BILINEAR, "fp32", _sscd("sscd_disc_mixup")),
    # Same weights under fp16 autocast, and at 288px.
    "sscd_fp16": ModelSpec(320, IMAGENET_MEAN, IMAGENET_STD, BILINEAR, "amp", _sscd("sscd_disc_mixup")),
    "sscd_288_fp16": ModelSpec(288, IMAGENET_MEAN, IMAGENET_STD, BILINEAR, "amp", _sscd("sscd_disc_mixup")),
    # Meta's TorchScript, fp32 only: the reference the rebuilt module is checked against.
    "sscd_ts": ModelSpec(320, IMAGENET_MEAN, IMAGENET_STD, BILINEAR, "fp32", _sscd_torchscript("sscd_disc_mixup")),
    # ResNeXt101 variant: kept as TorchScript fp32 (its trunk isn't remapped).
    "sscd_large": ModelSpec(320, IMAGENET_MEAN, IMAGENET_STD, BILINEAR, "fp32",
                            _sscd_torchscript("sscd_disc_large")),
    # DINOv2 (self-supervised, general purpose).
    "dinov2_s": ModelSpec(224, IMAGENET_MEAN, IMAGENET_STD, BICUBIC, "amp",
                          _timm("vit_small_patch14_dinov2.lvd142m", img_size=224)),
    "dinov2_b": ModelSpec(224, IMAGENET_MEAN, IMAGENET_STD, BICUBIC, "amp",
                          _timm("vit_base_patch14_dinov2.lvd142m", img_size=224)),
    # CLIP (semantic). Expected to over-merge "same subject" images; included as a reference.
    "clip_b32": ModelSpec(224, CLIP_MEAN, CLIP_STD, BICUBIC, "amp", _open_clip("ViT-B-32", "laion2b_s34b_b79k")),
}


def load_model(spec: ModelSpec, device: torch.device) -> nn.Module:
    return spec.load().to(device).eval()


def run(model: nn.Module, spec: ModelSpec, x: torch.Tensor) -> torch.Tensor:
    """Forward pass in the spec's precision; returns float32 (B, D)."""
    with torch.autocast(x.device.type, dtype=torch.float16, enabled=spec.precision == "amp"):
        return model(x).float()


def build_transform(spec: ModelSpec) -> T.Compose:
    # torchvision resizes PIL images with PIL itself, which antialiases on downscale.
    return T.Compose([
        T.Resize((spec.size, spec.size), interpolation=spec.interpolation, antialias=True),
        T.ToTensor(),
        T.Normalize(spec.mean, spec.std),
    ])
