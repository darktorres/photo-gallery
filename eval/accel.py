"""SSCD on ONNX Runtime and TensorRT: ONNX export, int8 quantization, engine builds, and runners that plug into
models.SPECS like any torch module (CUDA tensors in, L2-normalized fp32 embeddings out).

Files (built on first use, under CACHE_DIR):
  onnx/sscd_<size>_fp32.onnx, _fp16.onnx   exported from models.SSCD (see SSCDExport)
  onnx/sscd_<size>_int8.onnx               int8 Q/DQ on the convolutions, fp16 elsewhere (NVIDIA ModelOpt PTQ)
  onnx/calib_<size>.npy                    the calibration batch: CALIB_IMAGES unlabeled images not in files.txt
  trt/sscd_<size>_<precision>.plan         TensorRT engines (batch 1..MAX_BATCH)

TensorRT 11 only builds strongly typed networks: the precision is the ONNX graph's own (fp16 weights and
activations, or int8 Q/DQ), there is no fp16/int8 builder switch. ModelOpt pins older onnx / onnxruntime
versions, so it runs from its own venv (PG_QUANT_PYTHON, default D:\\PhotoGallery\\venv-quant). ONNX Runtime's
TensorRT provider is built against TensorRT 10, whose DLLs live in TRT10_DIR (outside this venv's TensorRT 11).
"""

import copy
import json
import os
import random
import shutil
import subprocess
from dataclasses import replace
from pathlib import Path

import numpy as np
import torch
from torch import nn
from torch.nn.utils.fusion import fuse_conv_bn_eval

from common import CACHE_DIR, DATA_ROOT, GROUPS_DIR, IMAGE_EXTS, NEGATIVES_DIR, image_list, load_rgb
from models import SPECS, SSCD, build_transform

ONNX_DIR = CACHE_DIR / "onnx"
TRT_DIR = CACHE_DIR / "trt"
QUANT_PYTHON = Path(os.environ.get("PG_QUANT_PYTHON", r"D:\PhotoGallery\venv-quant\Scripts\python.exe"))
TRT10_DIR = Path(os.environ.get("PG_TRT10_DIR", r"D:\PhotoGallery\tools\tensorrt-10.16.1.11-cu13"))
CALIB_IMAGES = 512
MAX_BATCH = 128
OPT_BATCH = 64
FP16_TINY = 2.0 ** -14  # smallest normal fp16 number


def _fold(conv: nn.Module, bn: nn.Module) -> nn.Conv2d:
    assert isinstance(conv, nn.Conv2d) and isinstance(bn, nn.BatchNorm2d)
    return fuse_conv_bn_eval(conv, bn)


class SSCDExport(nn.Module):
    """models.SSCD for export: the same weights and math, safe to run entirely in fp16.

    - BatchNorm is folded into the convolutions, in fp32, on construction. SSCD's BN running variances reach
      ~3.5e7, beyond fp16's 65504: cast to fp16 they become inf and zero their channels, which is what broke
      every pure-fp16 SSCD (autocast keeps BN parameters in fp32, so it never showed there).
    - GeM is computed as m * mean((x / m)^p)^(1/p) with m the per-channel maximum: identical to mean(x^p)^(1/p) in
      exact arithmetic, but every power stays <= 1, where x^3 itself overflows fp16 for activations above ~40.
      m is clamped at fp16's smallest normal number: GeM's eps (1e-6) is subnormal in fp16, TensorRT flushes it
      to zero, and an all-zero channel (common after ReLU) then gives 0/0 = NaN.
    - The L2 normalization is left to the caller: a 512-d sum of squares can overflow fp16 too."""

    def __init__(self, sscd: SSCD):
        super().__init__()
        self.sscd = copy.deepcopy(sscd).float().eval()
        r = self.sscd.resnet
        r.conv1, r.bn1 = _fold(r.conv1, r.bn1), nn.Identity()
        for layer in (r.layer1, r.layer2, r.layer3, r.layer4):
            for block in layer:
                for k in (1, 2, 3):
                    setattr(block, f"conv{k}", _fold(getattr(block, f"conv{k}"), getattr(block, f"bn{k}")))
                    setattr(block, f"bn{k}", nn.Identity())
                if isinstance(ds := getattr(block, "downsample", None), nn.Sequential):
                    block.downsample = _fold(ds[0], ds[1])

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        r = self.sscd.resnet
        x = r.maxpool(r.relu(r.conv1(x)))
        x = r.layer4(r.layer3(r.layer2(r.layer1(x))))
        pool = self.sscd.pool
        x = x.clamp(min=pool.eps)
        m = x.amax(dim=(2, 3), keepdim=True).clamp(min=FP16_TINY)
        x = (x / m).pow(pool.p).mean(dim=(2, 3)).pow(1.0 / pool.p) * m.flatten(1)
        return self.sscd.project(x)


def onnx_path(size: int, precision: str) -> Path:
    path = ONNX_DIR / f"sscd_{size}_{precision}.onnx"
    if path.exists():
        return path
    ONNX_DIR.mkdir(parents=True, exist_ok=True)
    if precision == "int8":
        _quantize_int8(onnx_path(size, "fp32"), calibration_batch(size), path)
        return path
    dtype = {"fp32": torch.float32, "fp16": torch.float16}[precision]
    sscd = SPECS["sscd"].load()
    assert isinstance(sscd, SSCD)
    model = SSCDExport(sscd).eval().to(dtype)
    x = torch.zeros(2, 3, size, size, dtype=dtype)
    torch.onnx.export(model, (x,), str(path), input_names=["x"], output_names=["y"], opset_version=17,
                      dynamic_axes={"x": {0: "batch"}, "y": {0: "batch"}}, dynamo=False)
    return path


def calibration_batch(size: int) -> Path:
    """CALIB_IMAGES random unlabeled images that are not in the evaluated files.txt, preprocessed like SSCD's
    input (float32 NCHW)."""
    path = ONNX_DIR / f"calib_{size}.npy"
    if path.exists():
        return path
    evaluated = set(image_list())
    pool = sorted(p.relative_to(DATA_ROOT).as_posix() for p in DATA_ROOT.rglob("*")
                  if p.is_file() and p.suffix.lower() in IMAGE_EXTS)
    pool = [p for p in pool if p.split("/")[0] not in (GROUPS_DIR, NEGATIVES_DIR) and p not in evaluated]
    random.Random(0).shuffle(pool)
    transform = build_transform(replace(SPECS["sscd"], size=size))
    batch = []
    for rel in pool:
        try:
            batch.append(transform(load_rgb(DATA_ROOT / rel)).numpy())
        except Exception:
            continue
        if len(batch) == CALIB_IMAGES:
            break
    if len(batch) < CALIB_IMAGES:
        raise RuntimeError(f"only {len(batch)} calibration images outside files.txt: run on a subset work dir")
    np.save(path, np.stack(batch).astype(np.float32))
    return path


QUANTIZE_SCRIPT = """
import json, sys
import numpy as np
from modelopt.onnx.quantization import quantize
src, calib, dst, options = sys.argv[1:]
quantize(src, quantize_mode="int8", calibration_data={"x": np.load(calib)}, calibration_eps=["cuda:0", "cpu"],
         high_precision_dtype="fp16", output_path=dst, **json.loads(options))
"""


def _quantize_int8(src: Path, calib: Path, dst: Path, method: str = "entropy", exclude: tuple[str, ...] = ()) -> None:
    """Post-training int8 quantization by ModelOpt, in its own venv (calibrating on the GPU with torch's CUDA
    DLLs). method: "entropy" or "max" activation ranges; exclude: node-name regexes kept in fp16. ModelOpt gets a
    copy of the source graph: it rewrites its input file in place."""
    work = dst.with_suffix(".src.onnx")
    shutil.copyfile(src, work)
    options = json.dumps({"calibration_method": method, "nodes_to_exclude": list(exclude) or None})
    env = {**os.environ, "PATH": str(Path(torch.__file__).parent / "lib") + os.pathsep + os.environ["PATH"]}
    try:
        subprocess.run([str(QUANT_PYTHON), "-c", QUANTIZE_SCRIPT, str(work), str(calib), str(dst), options],
                       check=True, env=env)
    finally:
        work.unlink(missing_ok=True)


# --- TensorRT (this venv's TensorRT 11) ---

def engine_path(size: int, precision: str) -> Path:
    import tensorrt as trt

    path = TRT_DIR / f"sscd_{size}_{precision}_trt{trt.__version__.split('.')[0]}.plan"
    if not path.exists():
        build_engine(onnx_path(size, precision), path, size)
    return path


def build_engine(onnx: Path, path: Path, size: int) -> None:
    import tensorrt as trt

    TRT_DIR.mkdir(parents=True, exist_ok=True)
    logger = trt.Logger(trt.Logger.WARNING)
    builder = trt.Builder(logger)
    network = builder.create_network(1 << int(trt.NetworkDefinitionCreationFlag.STRONGLY_TYPED))
    parser = trt.OnnxParser(network, logger)
    if not parser.parse_from_file(str(onnx)):
        raise RuntimeError("; ".join(str(parser.get_error(i)) for i in range(parser.num_errors)))
    config = builder.create_builder_config()
    config.set_memory_pool_limit(trt.MemoryPoolType.WORKSPACE, 2 << 30)
    profile = builder.create_optimization_profile()
    name = network.get_input(0).name
    profile.set_shape(name, (1, 3, size, size), (OPT_BATCH, 3, size, size), (MAX_BATCH, 3, size, size))
    config.add_optimization_profile(profile)
    plan = builder.build_serialized_network(network, config)
    if plan is None:
        raise RuntimeError(f"TensorRT build failed for {path.name}")
    path.write_bytes(bytes(plan))


class TRTModule(nn.Module):
    """Runs a TensorRT engine on torch's current CUDA stream (the cached engine for size and precision, or the
    given plan file)."""

    def __init__(self, size: int, precision: str, plan: Path | None = None):
        super().__init__()
        import tensorrt as trt

        runtime = trt.Runtime(trt.Logger(trt.Logger.WARNING))
        self.engine = runtime.deserialize_cuda_engine((plan or engine_path(size, precision)).read_bytes())
        self.context = self.engine.create_execution_context()
        names = [self.engine.get_tensor_name(i) for i in range(self.engine.num_io_tensors)]
        self.input = next(n for n in names if self.engine.get_tensor_mode(n) == trt.TensorIOMode.INPUT)
        self.output = next(n for n in names if self.engine.get_tensor_mode(n) == trt.TensorIOMode.OUTPUT)
        to_torch = {trt.float32: torch.float32, trt.float16: torch.float16}
        self.in_dtype = to_torch[self.engine.get_tensor_dtype(self.input)]
        self.out_dtype = to_torch[self.engine.get_tensor_dtype(self.output)]

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        x = x.to(self.in_dtype).contiguous()
        self.context.set_input_shape(self.input, tuple(x.shape))
        y = torch.empty(tuple(self.context.get_tensor_shape(self.output)), dtype=self.out_dtype, device=x.device)
        self.context.set_tensor_address(self.input, x.data_ptr())
        self.context.set_tensor_address(self.output, y.data_ptr())
        if not self.context.execute_async_v3(torch.cuda.current_stream().cuda_stream):
            raise RuntimeError("TensorRT execution failed")
        return nn.functional.normalize(y.float(), dim=1)


# --- ONNX Runtime ---

def _ort_session(path: Path, provider: str, precision: str, size: int):
    import onnxruntime as ort

    if provider == "trt":
        os.environ["PATH"] = str(TRT10_DIR) + os.pathsep + os.environ["PATH"]
        cache = TRT_DIR / "ort"
        cache.mkdir(parents=True, exist_ok=True)
        shape = lambda b: f"x:{b}x3x{size}x{size}"  # noqa: E731
        options = {"trt_engine_cache_enable": True, "trt_engine_cache_path": str(cache),
                   "trt_timing_cache_enable": True, "trt_fp16_enable": precision != "fp32",
                   "trt_int8_enable": precision == "int8", "trt_max_workspace_size": 2 << 30,
                   "trt_profile_min_shapes": shape(1), "trt_profile_opt_shapes": shape(OPT_BATCH),
                   "trt_profile_max_shapes": shape(MAX_BATCH)}
        providers = [("TensorrtExecutionProvider", options), "CUDAExecutionProvider"]
    else:
        providers = ["CUDAExecutionProvider"]
    so = ort.SessionOptions()
    so.log_severity_level = 3
    session = ort.InferenceSession(str(path), so, providers=providers)
    want = "TensorrtExecutionProvider" if provider == "trt" else "CUDAExecutionProvider"
    if session.get_providers()[0] != want:
        raise RuntimeError(f"{want} unavailable: {session.get_providers()}")
    return session


class ORTModule(nn.Module):
    """ONNX Runtime with the CUDA or TensorRT provider, inputs and outputs bound to torch CUDA memory.

    Both providers get the graph of the requested precision (provider="trt": ONNX Runtime's own TensorRT 10
    build, which has no strongly typed mode: an fp16 graph also needs trt_fp16_enable, or it runs in fp32 at
    a quarter of the speed)."""

    def __init__(self, size: int, precision: str, provider: str):
        super().__init__()
        self.session = _ort_session(onnx_path(size, precision), provider, precision, size)
        meta_in, meta_out = self.session.get_inputs()[0], self.session.get_outputs()[0]
        to_torch = {"tensor(float)": torch.float32, "tensor(float16)": torch.float16}
        self.input, self.output = meta_in.name, meta_out.name
        self.in_dtype, self.out_dtype = to_torch[meta_in.type], to_torch[meta_out.type]

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        x = x.to(self.in_dtype).contiguous()
        y = torch.empty((x.shape[0], 512), dtype=self.out_dtype, device=x.device)
        binding = self.session.io_binding()
        np_dtype = {torch.float32: np.float32, torch.float16: np.float16}
        binding.bind_input(self.input, "cuda", 0, np_dtype[self.in_dtype], tuple(x.shape), x.data_ptr())
        binding.bind_output(self.output, "cuda", 0, np_dtype[self.out_dtype], tuple(y.shape), y.data_ptr())
        torch.cuda.current_stream().synchronize()  # ONNX Runtime runs on its own stream
        self.session.run_with_iobinding(binding)
        return nn.functional.normalize(y.float(), dim=1)

