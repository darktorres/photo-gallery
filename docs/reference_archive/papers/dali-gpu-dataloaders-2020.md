# Speed-up Your Dataloaders by Image Processing on GPUs! (using NVIDIA DALI in Pytorch)

Blog post on "NLP <3 CV" (practical.codes), published November 10, 2020; no author named on the page. Read from the
Wayback Machine capture of 2 Dec 2023:
<https://web.archive.org/web/20231202115739/https://practical.codes/posts/nvidia_dali.html>.

Excerpts taken on 2026-11-13 from the saved page `NLP _3 CV - Speed-up Your Dataloaders by Image Processing on
GPUs!.mhtml`, which is not kept: the opening argument, the pitfall about corrupt images, and the torchvision-versus-DALI
benchmark. The code samples and the operator lists are left out. The text is quoted as is, without the page's emoji
shortcodes and link markers.

Kept for: PLAN §3 item 9 (decode and I/O are the bottleneck, not the model: "DALI article").

---

## So what?

Image processing is critical for training a good CV model but if you are using the default torchvision & PIL combo to
open and manipulate (resize,augment) your images you are doing it on the CPU. Surely more efficient backends besides
PIL are available, and you can even build some of these libraries from source to enable faster implementations

…OR you can just use the much faster NVIDIA DALI~

Note: Just as I was writing this post Torchvision 0.8.0 released native IO and support for GPU transforms. Comparisons
& benchmarks down below … Spoilers: DALI is still faster!

## Tell me more… why should I care about DALI?

In your dataloader it is not enough to just read the image bytes from disk (IO bound), you also have to decode the
images based on the encoding (JPEG, PNG, TIFF…). This is where DALI shines by using the CUDA accelerated NVJpeg library
for decoding images. You can expect x2 or more speed-up based on your batch/image sizes.

Still not convinced? Check out these blogposts by Nvidia that explain & benchmark nvJPEG decoding in more detail. The
new NVIDIA A100 GPUs even have a hardware JPEG decoder to further improve performance

Of course you might argue that processing your images on the GPU means you will have less memory for your model But
nowadays with larger and larger GPUs you can afford it! Besides the nvJPEG decoder only takes about 200mb~1gb depending
on image/batch size and the speed-up is well worth it! It is more likely that the bottleneck of your model is making
CPU<->GPU copies and a starved input pipeline.

## Highlights

- GPU accelerated JPEG decoding
- Built in prefetching (making you less IO bound)
- Image processing ops (resize,crop etc) on the GPU

ATTOW current DALI version:0.27 (I have been using DALI since 0.19)

## Pitfalls: one bad apple can throw your whole pipeline away

DALI works by building a data processing graph and running it at execution time with some efficient threaded magic
behind the python scenes. So if there is a corrupted image an exception is thrown and you whole pipeline goes [down].

But there is a solution~ You can check your images beforehand for corrupted images using the image-checker
library/tool I have made specifically for this purpose. […] Chances are you have a couple (hundred,thousand…) broken
images clogging your pipeline, especially if using images scraped from the web.

## Torchvision vs DALI

While I was preparing this blogpost torchvision v0.8 came out with; torchscript, batch and GPU support for transforms &
native image IO operations read_image, decode_image (jpeg,png only).

Now I will compare the above DALI pipeline with a torchvision equivalent. Namely, using torchvision.io.decode_image and
batch transforms on the GPU.

*[The benchmark decodes JPEGs, resizes to 256 × 256, centre-crops to 224 and normalises, at batch size 128.]*

Benchmarking the torchvision vs DALI dataloader codes above, with different batch sizes…And the [winner] goes to DALI
which was consistenly ~2.5 faster than torchvision. Although I wasn't able to find a lot of info on these new
torchvision functionalities. I can attribute this difference to a couple of things:

- torchvision.io.decode_image decodes one image at a time.
- decodes on the CPU
- moving decoded image to gpu takes longer than if encoded image bytes were moved to gpu.

I also haven't seen much speed-up between CPU vs GPU transforms on the tested ops
