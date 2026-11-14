# Perceptually Based Downscaling of Images

A. Cengiz Öztireli and Markus Gross (ETH Zurich), 2015.

Excerpts taken on 2026-11-13 from the PDF `Perceptually Based Downscaling of Images.pdf` (10 pages), which is not kept: the abstract and §1 Introduction, §4 Results (with the user study, performance and limitations), §5 Conclusions, and the pseudo-code of Appendix B. The text is the PDF's text layer as is, joined into paragraphs. Figures, table bodies and display equations are left out, except where marked *transcribed*: those were copied by hand from the rendered page. Exponents are written with `^` (s^r), and `*[p. N]*` marks the PDF page.

Kept for: not cited yet; an alternative to resample-then-sharpen for `docs/temp/THUMBNAIL_RESAMPLING.md`.

---

*[p. 1]*

We propose a perceptually based method for downscaling images that provides a better apparent depiction of the input image. We formulate image downscaling as an optimization problem where the difference between the input and output images is measured using a widely adopted perceptual image quality metric. The downscaled images retain perceptually important features and details, resulting in an accurate and spatio-temporally consistent representation of the high resolution input. We derive the solution of the optimization problem in closed-form, which leads to a simple, efficient and parallelizable implementation with sums and convolutions. The algorithm has running times similar to linear filtering and is orders of magnitude faster than the state-of-the-art for image downscaling. We validate the effectiveness of the technique with extensive tests on many images, video, and by performing a user study, which indicates a clear preference for the results of the new algorithm.

CR Categories: I.4.1 [Image Processing and Computer Vision]: Digitization and Image Capture—Sampling;

Keywords: images, video, perceptual, downscaling, structural similarity, unsharp masking

## 1 Introduction

Image downscaling is a fundamental operation performed constantly in digital imaging. The abundance of high resolution capture devices and the variety of displays with different resolutions

make it an essential component of virtually any application involving images or video. However, this problem has so far received substantially less attention than other sampling alterations.

Classical downscaling algorithms aim at minimizing aliasing artifacts by linearly filtering the image via convolution with a kernel before subsampling and subsequent reconstruction, following the sampling theorem [Shannon 1998]. However, along with aliasing, these strategies also smooth out some of the perceptually important details and features, as shown in Figure 1, since the kernels used are agnostic to the image content. A solution to this problem is adapting the kernel shapes to local image patches [Kopf et al. 2013] in the spirit of bilateral filtering [Tomasi and Manduchi 1998], so that they are better aligned with the local image features to be preserved. This strategy can significantly increase the crispness of the features while avoiding ringing artifacts typical for post-sharpening filters. However, it still cannot capture all perceptually relevant details, and as a result, might distort some of the perceptually important features and the overall look of the input image (Figure 1, content-adaptive), or lead to artifacts such as jagged edges [Kopf et al. 2013].

Loss of some of the perceptually important features and details stems from the common shortcoming of these methods that they operate with simple error metrics that are known to correlate poorly with human perception [Wang and Bovik 2009]. Significant improvements have been obtained for many problems in image processing by replacing these classical metrics with perceptually based image quality metrics [Zhang et al. 2012; He et al. 2014].

In this paper, we propose a perceptually based method for downscaling images. We formulate the downscaling problem as an optimization where we solve for the downscaled output image given the input image. The error between the two images is measured using the widely adopted structural similarity (SSIM) index [Wang et al. 2004]. The use of SSIM in optimization problems has been hindered by the resulting non-linear non-convex error functions [Brunet et al. 2012]. However, we show that for the downsampling problem, it is possible to derive a closed-form solution to this optimization. The solution leads to a non-linear filter, which involves computing local luminance and contrast measures on the original and a smoothed version of the input image. Although the filter is seemingly different than SSIM without any covariance term, we show that it maximizes the mean SSIM between the original and

*[p. 2]*

Figure 2: Commonly used filters for downscaling such as the box or bicubic filter result in oversmoothing. Trying to avoid oversmoothing by post-sharpening the downscaled images or using the Lanczos filter can lead to ringing artifacts and the small-scale features can still not be recovered. The recent methods, generalized sampling [Nehab and Hoppe 2011] and content-adaptive downscaling [Kopf et al. 2013] can produce crisper images, but cannot preserve all perceptually important details. In contrast to the previous methods, we utilize a perceptual metric and generate a perceptually optimum image as measured by this metric. This makes our results look closer to the original high resolution images by retaining apparent details. Input image courtesy of Flickr user Nicolas Raymond.

downscaled images.

Our downscaling method preserves perceptually important fine details and features that cannot be captured with other metrics, resulting in crisper images that provide a better depiction of the original image. The downscaled images do not exhibit disturbing aliasing artifacts for natural images and are spatio-temporally more coherent than methods based on kernel optimizations [Kopf et al. 2013]. This allows us to apply the technique to video downscaling as well. The resulting algorithm has a very simple, efficient, and parallelizable implementation with sums and convolutions. It thus has a computational complexity similar to the classical filtering methods, and runs orders of magnitude faster than the state-of-the-art [Kopf et al. 2013]. We illustrate that our method significantly improves the quality of downscaled images by comparisons to a variety of classical and state-of-the-art downscaling methods on many images. We validate our results with a user study, which indicates a clear preference for our algorithm over previous methods.

*[p. 6]*

## 4 Results

We performed a large number of experiments to validate the practical value of our method with thousands of images and many different downscaling factors, a detailed analysis, comparisons to existing methods, and a formal user study.

## 4.1 Downscaling Results and Analysis

We show several example results in Figures 1, 2, 5, 9–11, 15– 17. Please refer to the supplementary images and video for many additional downscaling results. Our technique generates local pixel patterns that form structures resembling those in the input image, when viewed by a human observer. This effect is most apparent when there are perceptually important features (Figures 1, 10), textures (Figures 15, 16), or other small-scale details (Figures 1, 2, 15, 16, 17) in the input images. While trying to capture as much structure as possible, it also preserves the local contrast and luminance of the input image, which makes the overall look of the downscaled image close to the input (e.g. Figures 1, 16).

The algorithm does not significantly alter the features that are already captured by low-pass filters. This results in less jagged edge artifacts than previous high quality downscaling methods. We show an example downscaled edge in Figure 9. Our method performs a slight enhancement on the edge, resulting in less artifacts than with

Figure 11: From left to right: Bicubic filtering, subsampling, our result. We get crisp details without Moirr´e patterns. Original image courtesy of Flickr user Chi King.

Subsampling Content- Adaptive Box Bicubic Bilateral Generalized Lanczos

Figure 12: User study results. The blue bars represent how many times our algorithm is selected, green is for no preference, and red is for the other algorithm.

the previous content-adaptive method of Kopf et al. [2013]. If some details cannot be captured with the pixel budget in the downscaled image, they are mapped to noise-like structures that resemble those in the input image if viewed at the native resolution, as opposed to Moirr´e patterns, as with subsampling (Figure 11).

The method is also spatio-temporally consistent, leading to accurate representation of features, as can be clearly seen in Figures 1, right, and Figure 10. Classical filtering methods such as bicubic filtering are also consistent, but fail to generate crisp images. Aligning the kernels to local image features [Kopf et al. 2013] can generate crisper results, but the resulting kernels can miss or distort some features as in Figure 10, and small changes in input images are sometimes amplified, leading to flickering, as we illustrate in the accompanying video.

## 4.2 User Study

There are numerous studies on the correlation of the SSIM index with human perception when used as an image quality measure [Wang and Bovik 2009]. However, our particular problem of downscaling calls for a tailored formal user study. The design of our user study follows that of the previous study performed by Kopf et al. [2013], including the images used and all design choices.

The study is based on presenting the participants a large image, and two downscaled versions of that image. The participant is then asked to select the small image that she/he thinks represents a better downscaled version of the large image, or indicate no preference. One of the small images presented for each test is computed using our algorithm, and the other by a previous algorithm. We included subsampling, the classical box, bicubic, and Lanczos filtering, as well as bilateral filtering, and the state-of-the-art algorithms generalized sampling [Nehab and Hoppe 2011] and content-adaptive downscaling [Kopf et al. 2013]. There were 125 participants in the study.

The 13 natural images used in the study, originally from the MSRA Salient Object Database [Liu et al. 2011], are the same as the ones used in the previous study [Kopf et al. 2013]. We show some example results in Figure 16 (please see the supplementary material

*[p. 7]*

Figure 13: Our algorithm runs only a few times slower than basic filters, and orders of magnitude faster than the state-of-theart [Kopf et al. 2013].

for the complete set of images and downscaling algorithms). They cover a variety of scenes with different types and scales of structures. The images were shown at the native resolution of the display, and zooming was not provided. The long side of the large images is 400 pixels, and that of the small images is 128 pixels. The study was performed online with participants from different parts of the world, educational backgrounds, occupations, and computer experience. Similar to the previous study [Kopf et al. 2013], we allowed the participants to move closer to the screen if they would like to, as would happen in practice for real-world situations. Each test for a particular participant involved a different image, and was repeated twice to check for consistency. All the results coming from subjects with consistency lower than 80% were discarded [Kopf et al. 2013], leaving results from 64 participants (the results do not change significantly for other rejection rates). There was no time limit to finish the study.

We present the results of the study in Figure 12. There is a clear preference for the results of our algorithm against all methods. The best competing downscaling method is simple subsampling, which was also the case for the previous study [Kopf et al. 2013]. Since subsampling does not involve any filtering, it preserves the crisp look of the images well, of course at the cost of well-known strong aliasing artifacts. For the user study images where these artifacts are not visible, the participants could not decide which algorithm to choose. For other images where the artifacts are noticeable, there is a clear preference for our algorithm. Hence, our algorithm preserves the crisp look of the images as in subsampling, but without the visible aliasing artifacts.

## 4.3 Implementation and Performance

Although the final method is based on a non-linear filter on the input image, it can be implemented very efficiently and robustly with simple convolutions and sums. We present the pseudo code of the algorithm in Appendix B. We implemented our algorithm in Matlab with native Matlab operators, some of which use multiple CPU cores.

We performed a performance test with 100 randomly chosen images on a computer with the configuration Intel Core i7 3770K CPU @350GHz. The method of Kopf et al. [2013] was run as a native executable. The results of the test are reported in Figure 13 for different input image sizes (with output image size fixed to 80 × 60), and output sizes (with input image size 640 × 480).

Our algorithm is only a few times slower than the box filter we used in the implementation of our algorithm, and ×500 − ×5000 faster than the method of Kopf et al. [2013] that relies on an iterative expectation-maximization based optimization. Our algorithm involves two box filterings followed by supsampling on the input image, and further operations on images of size proportional to the output image, as can be seen in the pseudo code in Appendix B. Hence, for smaller output sizes relative to the input size, it performs

Figure 14: Insets from left to right: original image, bicubic filtering, our result. Since our method lacks scene semantics, it tries to preserve the noise in the input image. Original image courtesy of Flickr user City of Boston Archives.

closer to the initial box filter we used, while increasing the output size slows it down a few times, as can be seen in Figure 13, right.

## 4.4 Limitations

A fundamental limitation of our method is its indifference to scene semantics. Like all previous methods, ours see the local structures in an image without any reference to what they actually represent. This, for example, leads to preservation of undesired details such as noise present in the input image, as we show in Figure 14, which is smoothed out by non-adaptive filters.

Our results exhibit fewer jagged edges (Figure 9) and aliasing artifacts (Figure 11) than methods that generate crisp images. However, if the image contains very regular repeating structures with a high frequency, aliasing can happen. The SSIM index tends to not prefer patches with a constant value, since this makes the index 0. Instead, our algorithm tries to reproduce the local contrast and structure. However, for perfectly regular structures, this might not be possible and a constant patch value is preferred instead. For those cases, such as on standard aliasing tests, we get artifacts similar to those produced by previous enhancement methods [Kopf et al. 2013]. Fortunately, such regular structures are rarely present in natural images. We observed that the small perturbations to regular structures that exist in most natural images can break the artifacts, as in Figure 11.

The SSIM index is known to not preserve the blur in the images [Chen et al. 2006]. We also observed that as opposed to thumbnail generation methods [Trentacoste et al. 2011; Didyk et al. 2012], our downscaling results do not contain the same amount of blur in the input image, especially for high downscaling ratios. We experimented with an extension of SSIM in the gradient domain [Chen et al. 2006], by solving for the gradients of the downscaled image, and subsequently a Poisson equation to get the actual image, but could not get satisfactory results so far.

## 5 Conclusions

We presented a novel method for image downscaling that aims to optimize for the perceptual quality of the downscaled results. Our extensive tests involving hundreds of images, and the user study clearly indicate that it generates perceptually accurate and appealing downscaling results, outperforming previous techniques. Despite its effectiveness and non-linear nature, it has a very simple, robust, efficient, and parallelizable implementation, making the algorithm a practical addition to the arsenal of image filters.

We used the basic form of the SSIM index. There are numerous extensions that modify the local similarity measure, the patch averaging stage, or extend it to feature and color spaces. It will be interesting to see how such extensions can affect the resulting downscaled images. However, some might also come with additional computational complexity. Although the downscaled videos exhibit less

*[p. 8]*

Figure 15: Our technique is able to capture small-scale details and textures while preserving local contrast and luminance to produce a perceptually accurate downscaled image. Input image courtesy of Flickr user ruru.

flickering due to the consistency of the filter, better downscaling results can be obtained by incorporating extensions of the SSIM index to videos, e.g. models of speed perception [Wang and Li 2007]. The SSIM index does not model all aspects of human perception. We believe it is a very interesting direction to investigate how other perceptual measures can be utilized to improve image scaling results.

The SSIM index sees the image at the level of patches, and cannot by itself adapt to scene semantics. This leads to problems such as the noise amplification in Figure 14. Scene semantics such as background/foreground separation, properties of the objects in the scene, or saliency maps can be integrated into our algorithm by adaptively weighting the patches, or adjusting the parameters (α, γ) and patch size locally.

Finally, we believe that the SSIM index, with its simple definition and excellent correlation with perception, can be utilized more widely in similar image processing problems, and in particular for algorithms that rely on matching patches [Darabi et al. 2012].

*[p. 9]*

## B Pseudo Code of the Algorithm

In the algorithm below, all operations are element-wise on the single channel images, denoted with big letters. The
function convValid(X, P(y)) convolves the image X with an averaging filter of size y × y for the valid range of the
image such that the kernel stays within the image limits. The function convFull is similar but the image is assumed to
be padded with zeros to allow the kernel go out of the image limits. The function subSample(X, y) subsamples the image
X at intervals of y, I_X produces an image of the size of X with all ones, X(C) gets all entries of the image X for
which the corresponding entry in the image C returns true, and ϵ = 10^−6.

*[Algorithm 1, transcribed]*

```text
Algorithm 1  Downscale Image
Input:  Input image H, downscaling factor s, patch size n_p.
Output: Downscaled image D.
 1: procedure DownscaleImage
 2:     L   ← subSample(convValid(H, P(s)), s)
 3:     L_2 ← subSample(convValid(H², P(s)), s)
 4:     M   ← convValid(L, P(√n_p))
 5:     S_l ← convValid(L², P(√n_p)) − M²
 6:     S_h ← convValid(L_2, P(√n_p)) − M²
 7:     R   ← √(S_h / S_l)
 8:     R(S_l < ϵ) ← 0
 9:     N   ← convFull(I_M, P(√n_p))
10:     T   ← convFull(R × M, P(√n_p))
11:     M   ← convFull(M, P(√n_p))
12:     R   ← convFull(R, P(√n_p))
13:     D   ← (M + R × L − T)/N
```
