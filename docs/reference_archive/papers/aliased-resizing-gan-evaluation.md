# On Aliased Resizing and Surprising Subtleties in GAN Evaluation

Gaurav Parmar, Richard Zhang, Jun-Yan Zhu (Carnegie Mellon University, Adobe Research).

Excerpts taken on 2026-11-13 from the PDF `On Aliased Resizing and Surprising Subtleties in GAN Evaluation.pdf` (11 pages), which is not kept: the abstract and §1 Introduction, §3.2 Image Resizing, Table 1 with §4.1–4.2, and §5 Recommendations. The text is the PDF's text layer as is, joined into paragraphs. Figures, table bodies and display equations are left out, except where marked *transcribed*: those were copied by hand from the rendered page. Exponents are written with `^` (s^r), and `*[p. N]*` marks the PDF page.

Kept for: PLAN §3 item 8 (the resize filter is part of the model: freeze one antialiased resize, Table 1) and §10; also `docs/temp/THUMBNAIL_RESAMPLING.md`.

---

*[p. 1]*

Metrics for evaluating generative models aim to measure the discrepancy between real and generated images. The often-used Fr´echet Inception Distance (FID) metric, for example, extracts “high-level” features using a deep network from the two sets. However, we find that the differences in “low-level” preprocessing, specifically image resizing and compression, can induce large variations and have unforeseen consequences. For instance, when resizing an image, e.g., with a bilinear or bicubic kernel, signal processing principles mandate adjusting prefilter width depending on the downsampling factor, to antialias to the appropriate bandwidth. However, commonly-used implementations use a fixed-width prefilter, resulting in aliasing artifacts. Such aliasing leads to corruptions in the feature extraction downstream. Next, lossy compression, such as JPEG, is commonly used to reduce the file size of an image. Although designed to minimally degrade the perceptual quality of an image, the operation also produces variations downstream. Furthermore, we show that if compression is used on real training images, FID can actually improve if the generated images are also subsequently compressed. This paper shows that choices in low-level image processing have been an underappreciated aspect of generative modeling. We identify and characterize variations in generative modeling development pipelines, provide recommendations based on signal processing principles, and release a reference implementation to facilitate future comparisons.

## 1. Introduction

With the proliferation of generative modeling techniques, such as Generative Adversarial Networks (GANs) [24], accurately discerning which methods are performing better has become a critical aspect of the field. For visual data, metrics such as Inception Score (IS) [59], Kernel Inception Distance (KID) [4], and the ubiquitously-used Fr´echet Inception Distance (FID) [26] have become standard practice for developing and adopting models. Under the hood, these methods evaluate the discrepancy between generated and natural images, in a deep feature space, to capture relevant

Figure 1. Downsampling a circle. We resize an input image (left) by a factor of 8, using different image processing libraries. The Lanczos, bicubic, and bilinear implementations by PIL (top row) adjust the antialiasing filter width by the downsampling factor (marked as

). Other implementations (including those used for PyTorch-FID and TensorFlow-FID) use fixed filter widths, introducing aliasing artifacts (marked as

) and resemble naive nearest subsampling. Aliasing artifacts induce inconsistencies in the calculation of downstream metrics such as Fr´echet Inception Distance [26], KID [4], IS [59], and PPL [33]. Note that antialias flag is available in TensorFlow 2, but is set to False (default value) for the FID calculation.

features of the two distributions. After all, at its core, generative modeling involves learning and mimicking high-order, complex statistics of visual data. However, we find that low-level, seemingly innocuous operations, can induce surprisingly large discrepancies in high-level statistics. For example, consider Figure 1. Given the same input image, different image processing libraries produce drastically different results. Specifically, the implementations using OpenCV, TensorFlow and PyTorch libraries with default flags, contain severe aliasing artifacts. Similarly, the simple act of saving images as JPEG with the

*[p. 2]*

Figure 2. Interpolation Filters. We show adaptive filters by PIL (top) and non-adaptive filter from PyTorch (bottom). The FID implementations in PyTorch and TensorFlow use a fixed-width bilinear interpolation, independent of resizing ratio. In contrast, the proposed Clean-FID uses an implementation that follows standard signal processing principles and adaptively stretches the filter to prevent aliasing. The horizontal axes represent the spatial coordinates and the vertical axes represents the kernel intensity.

default parameters, either when building the training dataset or collection of generated images, adds quantization and low-level statistical differences to the underlying data. The differences induced cause meaningful variations when used for evaluation protocols. As the Fr´echet Inception Distance (FID) metric [26] is the most ubiquitous [6,26,33,36,56], it is the focus of our experiments. We offer a standard benchmark, clean-fid (github.com/GaParmar/clean-fid), and concrete suggestions on resizing and quantization procedures to enable clean comparisons in future evaluation protocols. First, we investigate the implications of image resizing. When downsampling, signal processing techniques recommend “prefiltering” the input, to prevent high-frequency elements from aliasing into the output. When the downsampling factor is larger, the prefilter kernel should be correspondingly stretched. However, as shown in Figure 2, the resizing function used by the FID implementations in TensorFlow and PyTorch do not prefilter the image, resulting in aliasing artifacts shown in Figure 1. Resizing can occur in two locations – during data preprocessing (training with lower resolution) or at evaluation time (resizing to 299 resolution to compute the FID metric). In both cases, inconsistent resizing functions induce variations downstream. If used for data preprocessing, the training data distribution itself is changed. When used for the evaluation metric, small variations in resizing can cause changes in subsequent feature extraction. We quantify the effects of these inconsistencies and offer standard recommendations. Specifically, we propose to use a stronger bicubic filter [35]; more importantly, we propose to adjust prefiltering width based on the resizing factors, as guided by signal processing principles.

Secondly, we investigate the implication of image compression. While the JPEG protocol is a lossy compression scheme, designed to preserve perceptual similarity to the original [67], it can perturb an image enough to corrupt downstream feature extraction. This affects performance drastically and can create mismatches when comparing methods. Perhaps more surprisingly, when training images are saved with JPEG compression, modern GANs are unable to fully mimic the induced artifacts, and large FID improvements can actually be artificially achieved by tweaking the JPEG compression ratios when storing the generated images. We quantify the surprising effects of this compression operation, and again offer a concrete, standardized protocol to avoid inconsistencies and hindrances to proper evaluation. In conclusion, we characterize the surprising importance of low-level image processing steps, resizing and quantization, when training and evaluating generative models, such as GANs. We focus our experiments on the widely adopted FID metric, and show additional results on the KID metric [4] as well as IS [59] and Perceptual Path Length (PPL) metrics [33] (in the supplement). Importantly, any metric, present or future, that derives statistics from images undergoing these processing steps, will be affected by these factors.

*[p. 4]*

## 3.2. Image Resizing

Depending on the dataset and training size, the resizing operations (ψFID, ψ̂FID) in Figure 3 can either be downsampling or upsampling. Downsampling is the primary focus of this investigation, as it involves throwing away information. Methods for downsampling is a common study in the fields of signal and image processing [23,49].

Antialiasing by prefiltering. The most naive approach is to simply subsample (taking every N^th element if performing downsampling by an integer factor N), sometimes referred to as nearest. This corresponds to filtering the input image with Kronecker delta function, as only a single value is drawn. Such an approach leads to aliasing, as high-frequency elements of the input alias to the output. A central principle in signal processing, graphics, and vision [20,21,23,49,64] is to blur or “prefilter” before subsampling, as a means of removing high-frequency information (thus preventing its misrepresentation downstream). For linear filters, this corresponds to a “depth-wise convolution”, using deep learning parlance [27,62]. We explain two important ways in which prefiltering implementations can vary.

Filter size adaptation to downsampling factor. First, according to signal processing principles, the size of the filter should be adjusted, in accordance with the downsampling factor. Widening the low-pass filter in the spatial domain corresponds to reducing its bandwidth and filtering more aggressively in frequency space. As a larger downsampling factor means a lower bandwidth can be represented on the output signal, widening the filter accordingly is necessary to prevent aliasing. However, in many common implementations, this is not implemented (or is not used by default); instead, a filter of fixed, non-adaptive size is used.

*[p. 5]*

Figure 5. Differences in Inception features induced by inconsistent resizing. We resize full resolution 1024 × 1024 FFHQ [33] image (left) to 299 × 299 using PIL-bicubic (top), TensorFlow-bilinear (used by TF-FID) (middle), and PyTorch-bilinear (used by PyTorch-FID) (bottom). The resizing functions using current FID implementations (middle and bottom rows) introduce artifacts; for example, the hair and glasses appear noisier and aliased, as compared to the top row. We observe similar behavior on other commonly-used datasets - AFHQ-Cats (512 × 512) and MetFaces (1024 × 1024). Furthermore, these resizing implementations are inconsistent with each other, inducing different activation maps when passed through the Inception-V3 network [63]. We propose to resolve this inconsistency and also reduce the aliasing, by standardizing bicubic downsampling as the preprocessing function for a “Clean-FID” (using filtering that adjusts to the downsampling factor, adhering to signal processing principles).

Choice of filters. Secondly, there is a choice of different convolutional filters. The idealized low-pass filter is a sinc, requiring infinite support. As such, approximate filters with different subtle tradeoffs in runtime and behavior are used instead. The box, also known as area filter, corresponds to a rectangular filter, computing the average of values within a neighborhood. The bilinear filter is a triangular filter, bicubic [35] is a stronger cubic function, and the lanczos filter is an enveloped sinc. All perform a weighted average and have stronger antialiasing, closer to the idealized sinc.

Practical implications of implementation variations. We investigate inconsistencies that arise, when these two factors are varied, and show a toy example in Figure 1. While the choice of filter is largely constant across libraries (lanczos, bicubic, bilinear are shown in each column), the choice of whether the filter adapts to the downsampling factor is not. While the PIL library adapts the filter (top row), other libraries do not by default, leading to aliased results. In particular, FID implementations of TensorFlow-FID and PyTorch-FID, use bilinear downsampling implementations that exhibit aliasing, and thus are the focus of our study. An implication of aliasing is a suboptimal representation of the original image. In Figure 4, we show the result of downsampling and upsampling 300 FFHQ images, and comparing it to the original with PSNR. The methods with fixed filters achieve worse reconstruction than a method that adapts the filter. This effect is accentuated by larger downsampling factors, where high-frequency aliasing dominates

when using non-adaptive filters. Figure 5 shows how the Inception features are affected by aliased resizing functions.

Recommendation. Above, we have established that the implementations of FID are inconsistent and aliased. Ideally, the community can (a) use a consistent pipeline to facilitate fair comparisons across papers, and (b) follow signal processing principles and antialias, in order to best represent the underlying data being characterized. We propose to use an adaptive filter (and thus produce consistently antialiased results). Second, we propose to use a bicubic, instead of bilinear filter, which offers stronger reconstruction. While such an implementation is currently found in PIL, future equivalent and efficient implementations would be of use.

*[p. 6]*

*[Table 1, transcribed. Left: real FFHQ images resized 1024 → 299 with each function, compared with
the same images resized with PIL–bicubic. Right: the same on StyleGAN2 images. ✓: the filter width adapts to the
downsampling factor; ✗: fixed width.]*

| Resize function | FID ↓ | KID ×10³ ↓ | PSNR dB ↑ | StyleGAN2 FID ↓ | StyleGAN2 KID ×10³ ↓ |
|---|---|---|---|---|---|
| PIL–bicubic ✓ | 0 | 0 | ∞ | 2.98 | 0.51 |
| PIL–bilinear ✓ | 0.64 | 0.61 | 45.7 | 4.03 | 1.52 |
| TensorFlow–bilinear ✗ | 4.34 | 4.32 | 37.66 | 7.45 | 5.12 |
| PyTorch–bilinear ✗ | 4.36 | 4.31 | 37.66 | 7.45 | 5.15 |
| Naive nearest ✗ | 7.43 | 7.54 | 35.16 | 10.67 | 8.47 |

Table 1. Deviations induced by varying resizing implementations. We measure the discrepancy between real images
downsampled with PIL–bicubic (1024 → 299) vs. other functions (ψ̂FID) on the left. If all downsampling functions were
equivalent, the metrics (FID & KID) should be 0 and PSNR ∞. PIL–bilinear and bicubic adjust antialiasing to the
downsampling factor (✓) and produce relatively low neural metric scores and high PSNRs. Functions using fixed width
filters (✗) produce higher discrepancies. Naive nearest does not antialias at all. A similar trend holds on synthetic
StyleGAN2 [34] images.

FID evaluation step (ψFID, ψ̂FID), resizing in the data preprocessing step (ψdata), and quantizing of images (Q, Q̂). We introduce sources of variation at these steps and investigate their impacts in Sections 4.1, 4.2, and 4.3 respectively.

## 4.1. Variation due to FID Resizing

Here we investigate the effects of different resizing methods (ψFID, ψ̂FID) used in the FID calculation step.

Variation induced by resizing functions on real images. We start with two sets of full-resolution 1024 × 1024 face images - from the FFHQ dataset, and from a pre-trained StyleGAN2 generator. Each of the sets of images is resized from 1024→299 using different methods. In Table 1 (left), we compare the set of real images resized with the antialiased resizing operation (PIL bicubic) to the same set of real images, resized using other aliased functions that use a fixed width prefiltering kernel. As we compare the same set of images, we anticipate all FID and KID scores to be close to 0 and the PSNR values to be very high. However, as shown in Figures 1, 2, and 5, only a subset of the commonly used resizing operators adjust the filter width and antialias the images. These differences in resizing operations cause drastic changes in the Inception-V3 [63] activation maps. Filters that antialias are more consistent, even with different filter types – PIL-bilinear has FID 0.64 when compared to PIL-bicubic. On the other hand, implementations that ignore the downsampling factor (PyTorch and TensorFlow) show much larger deviation (FID 4.3), with scores nearing naive nearest (FID 7.4), that does not filter at all. This indicates that whether the filter adapts to the downsampling factor can change the modeled data distribution by non-trivial amounts.

Variation induced by resizing functions on generated images. After studying the effects on real images, we evaluate how different resizing function ψ̂FID choices affect the metric

*[p. 7]*

Resize function Resize(Dataset Images) vs. Resize(StyleGAN2)

FFHQ MetFaces AFHQ-Cats AFHQ-Dogs FID ↓ FID ↓ FID ↓ FID ↓ PIL–bicubic (

Table 2. Resizing functions affect FID scores. Here, both resizing functions on real and synthetic images (ψFID, ψ̂FID) are the same. If all resizing functions were consistent, all rows would be equal. Interestingly, the downsampling methods that alias result in lower scores; the lowest score is achieved by naive nearest subsampling. Methods that adjust the prefilter size to downsampling factor (implemented by PIL) better preserve information of the original images. This indicates that antialiasing enables subsequent FID to more sensitive to differences in the distributions.

when used in a full generative modeling pipeline. Here, we evaluate a pretrained StyleGAN2 generator [34] trained on FFHQ (1024), MetFaces (1024), and AFHQ (512) dataset images, and compute the FID with 50,000 images. In Table 1 (right), we consider the asymmetric case, where features for the real images and generated images use different resizing functions. This case arises when features for real images are pre-computed and shared by one group of authors, while generated features may be calculated on the fly with a different library. Here, we observe that using the same resizing function as the reference dataset (PIL-bicubic) achieves the lowest performance. Using a different resize function, such as PIL-bilinear increases the score to 4. Using an aliased function increases the score drastically to 7, close to naive subsampling (> 10). Next, in Table 2, we show a comparison when the same resizing function is used for the real dataset images and the StyleGAN2 generated images. Interestingly, we observe that the aliased resizing functions result in lower FID scores across multiple commonly used datasets - FFHQ (1024), MetFaces [31] (1024), and AFHQ [9] (512). This indicates that using the antialiased function as preprocessing makes the downstream FID calculation more sensitive at measuring the discrepancies between distributions.

## 4.2. Variation due to Dataset Resizing

Previously, we considered scenarios where the dataset was not downsampled. However, as discussed in Section 1 and illustrated in Figure 3, dataset downsampling is needed when training a model on a low-resolution version of the original dataset [31, 72, 75] (e.g., 256 for FFHQ). Before, the target distribution was fixed, and differences were purely introduced during post-hoc metric evaluation. Now, the situation is much more intricate. Different resizing choices will result in different training distributions entirely. In Table 3, we train three different StyleGAN2 [34] (config-e) models, following the official PyTorch implementation^(\*) for 25k it-

\*<https://github.com/NVlabs/stylegan2-ada>

Dataset preprocessing FID ↓on FFHQ PIL-bicubic Naive Nearest (

Table 3. Dataset resizing. We downsample the FFHQ dataset using different resize functions ψdata from 1024 to 256. We train StyleGAN2 [34] (Config-E) models, using the identical training procedure and report FID of the result. The score is computed across three different training runs for each of the setting. The scores show large variation, indicating the resizing function can greatly affect the training distribution. Using a preprocessing function that antialiases (marked by

) preserves more information from the original images and interestingly results in a higher score.

Figure 7. Effects of JPEG compression. The FFHQ dataset images are resized from 1024 to different resolutions (512 and 256) using PIL-bicubic and compressed using the JPEG format, with different compression ratios. Subsequently, we plot the FID (left) and KID (right) between the compressed images and uncompressed images, at the same resolution, as a function of JPEG compression. The effect of JPEG compression is more severe for smaller images.

erations. We resize FFHQ [33] to 256 using Naive Nearest, PIL–bicubic, PyTorch–bilinear, and TensorFlow–bilinear. We use the same PIL–bicubic function (ψFID, ψ̂FID) for FID evaluation; note that here, it is upsampling (256 →299). Qualitatively, using an aliased downsampling function produces a training distribution with visual artifacts for the generative model to mimic, likely different than the natural visual data we wish to model. Quantitatively, interestingly, we observe that that the aliased pre-processing results in lower FID values. As the antialiased function better preserves signal in the original images, we hypothesize that retaining more information from the original input actually produces a more difficult distribution to model.

*[p. 8]*

## 5. Recommendations

We have shown surprisingly sensitivities to seemingly inconsequential implementation details when evaluating generative models. The resize operation and the image quantization/compression are especially impactful. Based on our observations, we discuss some best practices when training and evaluating generative models. We recommend using implementations that adapt the filter size to the downsampling factor, following signal processing principles, at each resizing step (ψdata, ψFID, and ψ̂FID). There are many details one needs to keep track of when computing FID. Any inconsistency leads to results that are no longer comparable to other methods. To facilitate an easy comparison, avoid inconsistent comparisons, and encourage the usage of critical operations that are correctly implemented, we provide an easyto-use library (github.com/GaParmar/clean-fid), and pre-computed Inception statistics for common datasets.

Acknowledgments. We thank Jaakko Lehtinen and Assaf Shocher for bringing attention to this issue and for helpful discussion. We thank Sheng-Yu Wang, Nupur Kumari, Kangle Deng, and Andrew Liu for useful discussions. We thank William S. Peebles, Shengyu Zhao, Taesung Park, Kris Kitani, and Stanislav Panev for proofreading our manuscript. We are grateful for the support of Adobe, Naver Corporation, and Sony Corporation.
