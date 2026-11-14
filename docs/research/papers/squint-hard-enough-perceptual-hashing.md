# Squint Hard Enough: Evaluating Perceptual Hashing with Machine Learning

Jonathan Prokos, Tushar M. Jois, Neil Fendley, Roei Schuster, Matthew Green, Eran Tromer, Yinzhi Cao (Johns Hopkins University, Vector Institute, Tel Aviv University, Columbia University), 2021.

Excerpts taken on 2026-11-13 from the PDF `Squint Hard Enough - Evaluating Perceptual Hashing with Machine Learning.pdf` (46 pages), which is not kept: the abstract, §2.2 Case Studies: PhotoDNA and PDQ, §5.2–5.3 (baseline thresholds and attack results) and §7 Conclusion. The text is the PDF's text layer as is, joined into paragraphs. Figures, table bodies and display equations are left out, except where marked *transcribed*: those were copied by hand from the rendered page. Exponents are written with `^` (s^r), and `*[p. N]*` marks the PDF page.

Kept for: PLAN §10 (PDQ's behaviour and robustness); PDQ is Tier 1 (`crates/dedup/src/pdq_index.rs`).

---

*[p. 1]*

Many online communications systems use perceptual hash matching systems to detect illicit files in user content. These systems employ specialized perceptual hash functions such as Microsoft’s PhotoDNA or Facebook’s PDQ to produce a compact digest of an image file that can be approximately compared to a database of known illicit-content digests. Recently, several proposals have suggested that hash-based matching systems be incorporated into client-side and end-to-end encrypted (E2EE) systems: in these designs, files that register as illicit content will be reported to the provider, while the remaining content will be sent confidentially. By using perceptual hashing to determine confidentiality guarantees, this new setting significantly changes the function of existing perceptual hashing – thus motivating the need to evaluate these functions from an adversarial perspective, using their perceptual capabilities against them. For example, an attacker may attempt to trigger a match on innocuous, but politically-charged, content in an attempt to stifle speech. In this work we develop threat models for perceptual hashing algorithms in an adversarial setting, and present attacks against the two most widely deployed algorithms: PhotoDNA and PDQ. Our results show that it is possible to efficiently generate targeted second-preimage attacks in which an attacker creates a variant of some source image that matches some target digest. As a complement to this main result, we also further investigate the production of images that facilitate detection avoidance attacks, continuing a recent investigation of Jain et al. Our work shows that existing perceptual hash functions are likely insufficiently robust to survive attacks on this new setting.

*[p. 6]*

## 2.2 Case Studies: PhotoDNA and PDQ

We now describe the PhotoDNA and PDQ PHM schemes.

Microsoft PhotoDNA. PhotoDNA [29] was invented in 2009 by Microsoft Research and Dartmouth College. The precise design of the algorithm was never published, and IP ownership was subsequently donated to NCMEC [29] who have maintained the secrecy of the technology by licensing it to technology providers under non-disclosure agreement. While the precise operation of PhotoDNA is not published, several sources have provided algorithm descriptions of varying completeness: these include partial descriptions published by Microsoft [29], as well as more complete purported descriptions derived from reverse-engineering and/or partial descriptions published by researchers [23, 24]. In 2020 a binary implementation of PhotoDNA was extracted from forensic software [21,22], and we use this algorithm in its binary form for our experiments in Section 5. Although our attacks use the PhotoDNA binary in a black-box manner, we now briefly summarize the description drawn from public documents, as summarized by Krawetz [24]. The algorithm works by first reducing the image size to 26 pixels per side, and then converting it into grayscale. The resulting is then split into a 6 × 6 grid of separate “bins”. These bins are laid out in an overlapping pattern to ensure that minor trimming does not severely affect the hash, resulting in 36 bins across the entire image. Each grid is then passed through the Sobel operator [39], a type of 1-dimensional convolutional filter based on the

*[p. 7]*

Figure 2: An image from ImageNet [36] (left) and a visualization of its associated PhotoDNA (center) and PDQ (right) hashes.

difference between pixel values. This has the effect of intensifying (and therefore delineating) the edges of an image. The hash for a grid is a 4-tuple of the sum of these Sobel outputs leftwards, rightwards, upwards, and downwards across the grid. The final hash consists of the 36 4-tuples for each of the grids, for a total of 144 values. The output digest of PhotoDNA is a vector of 144 numeric values that can be compared to another digest using a simple distance metric. The exact parameters of the PHM system as deployed by online service providers, namely the distance metric comp and the chosen distance bound ∆d (Section 2.1), are not public. Our experiments utilize an L1 distance metric (as used in [13]); to evaluate the appropriate bound ∆d we conduct experiments with false-positive rates in §5.2.

Facebook PDQ. The PDQ algorithm was published in 2019 by Facebook, in response to the company’s need for an updated and more flexible perceptual hash algorithm. Unlike PhotoDNA, the implementation and design goals of PDQ are public [11]. PDQ begins with a normalization step, which processes the luminance data from the input image, followed by a two-pass Jarosz filter to downsample the image to 64 × 64. Within the downsampled image, compute a sum of absolute values of horizontal and vertical gradients along with a two-dimensional discrete cosine transform, resulting in a 16 × 16 DCT. For each of the 16x16 bits of the output hash, emit a 1 if the corresponding element of transform is greater than the median, otherwise emit a 0; the resulting 256 bits form the output digest. Similar to PhotoDNA, we compare the hash output of PDQ to another digest using an L1 distance metric (as used in [11]). The designers of PDQ provide a metric of choice: since the output is a 256-bit binary string, they recommend that the two hashes should be compared utilizing the Hamming distance metric. Our experiments use this to implement the function comp.

Discussion. PhotoDNA and PDQ both specify inexact matching algorithms. Notably, both employ Sobel gradients in their analysis of an image. This filter, applied horizontally and vertically, prominently detects the edges of an image, as these edges have high gradient disruption along them. This is useful in perceptually identifying an image, as pixel-level changes, such as compression or minor cropping, will not fundamentally change an image’s edges, and therefore should have a minimal effect on the digest. In both algorithms, the hash value is calculated as a sum of the value of gradients. This can be equivalently understood as a count function, and is therefore non-differentiable. While this can be estimated using tanh, our attacks below do not require differentiablity, as we abstract this portion of the hash as a black box.

*[p. 13]*

## 5.2 Baseline Experiments

Our analysis consists of two stages. First, to establish a baseline for hashing accuracy and distance thresholds in non-adversarial settings, we evaluated the hash algorithms on unmodified image files drawn from a standard dataset, and then consider the same images with light (but non-adversarial) transforms. Each of PhotoDNA and PDQ uses inexact matching and a distance metric to compare digest. The threshold ∆d is adjustable, and can vary by deployment. The specific value used in a system has a complex impact for our purposes: higher thresholds are likely to make a PHM system more sensitive to re-encoded content, with the possibility of a higher natural false-positive rate. At the same time, lower thresholds can potentially make collisions more difficult to find, while improving the success of avoidance attacks.

Determining match thresholds. We conduct our attacks at a variety of collision thresholds for each hash function. To provide some context for these thresholds, we conducted a series of experiments as described by Facebook in its evaluation of PDQ [11]. These involve computing the pairwise distances of a set of the hashed CopyDays dataset, which consists of 157 perceptually-distinct images [18,20]. The resulting 24,492 pairwise results are plotted in Fig. 4 for both PhotoDNA and PDQ. We find that a matching distance of approximately 92 for PDQ, and 1,809 are an appropriate threshold to eliminate false positives in this representative dataset. For simplicity, in our later experiments we will use 90 as our baseline matching threshold for PDQ and 1,800 for PhotoDNA.

Accuracy under simple transforms. To evaluate the matching efficacy of these algorithms under simple image transforms, we compared the unmauled CopyDays images with transformed images from that dataset.

*[p. 14]*

Table 1: Precision and Recall values for matching tasks utilizing various transformed datasets. For JPEG compression we create two datasets, one containing JPEG-compressed images with 5% and 8% degradation, and a second containing JPEG-compressed images 50% and 75% degradation. For crops, we similarly create two datasets, one with crops missing 10% and 15% of the image, and another dataset removing 70% or 80%. These four datasets are of size 471. Strong is adversarially perturbed, size 314, as defined by [18].

These include a first set of JPEG compressed images with 5% and 8% degradation respectively, and a second set containing JPEG compressed images 50% and 75% degradation. For crops, we similarly compare two datasets, one with crops missing 10% and 15% of the image, and another dataset removing 70% and 80%. These four datasets are each of size 471 = 3 × 157. Finally, we compared against a “Strong” that is adversarially perturbed, size 314 = 2 × 157. By computing the pairwise distances of both the original and transformed versions of the dataset, we can see how those transforms affect the resulting hash distances under each algorithm. These results are shown in Table 1. Using the baseline thresholds, we observe that PDQ performs slightly better at matching minor JPEG compression, but much better than PhotoDNA when matching both minor and major cropping. This behavior is likely due to the underlying primitives used to perform the perceptual hashing; PDQ applies a modified version of the Discrete Cosine Transform (DCT), the same as the transform in JPEG compression [42], while PhotoDNA relies primarily on its binning algorithm. Interestingly, PhotoDNA performed relatively poorly on crops, even though it takes specific steps to mitigate changes in hashing due to crops [24].

## 5.3 Attack Results

We implement the following two attacks utilizing images from the ImageNet Validation dataset [36]^7.

## 5.3.1 Targeted-Second-Preimage Generation

Given a target image we attempt to generate an image which collides in hash space, but remain perceptually distant (kayak to toilet bowl in Fig. 1a, boy to shark in Fig. 1b, etc.) utilizing the attack described in Section 4. This corresponds to being able to cause false positive illicit content hash matches in a deployed system using benign images and the database of illicit-content hashes.

Results. We were able to find collisions at various levels below the baseline ∆d discovered in Section 5.1 for complete attacks on PhotoDNA and PDQ. We successfully created targeted preimages on randomlychosen ImageNet sample image pairs, using our minimal threshold parameter ∆d = 1, 800 for PhotoDNA and ∆d = 90 for PDQ. Figures 5 and 6 show two such example images, with total hash distances 1, 800 and 88 respectively. We ran each of our algorithms until ∆d approached zero – a perfect, exact collision on the hash function – but with visible effects on image quality.

Attack progression. We performed additional experimentation to characterize the relationship between the number of iterations our attack executes and the resulting hash distance. Figure 7 show the progress of our attack on 30 randomly selected ImageNet image pairs. For PhotoDNA, 17 image pairs reached the

7This is a common image dataset, and has been used in prior perceptual hashing work [19,31].

*[p. 15]*

Figure 5: A set of intermediate collision images using the PhotoDNA hash function, with visualizations of their respective PhotoDNA hashes. The left image shows the original source image, and the rightmost image represents the desired target image. Each hash distance represents the difference between that image and the target image. The intermediate images show generated collisions at hash distances of 1797, 963 and 342 respectively. Image quality is only slightly degraded even for very low hash distances.

Figure 6: A set of intermediate collision images using the PDQ hash function, with visualizations of their respective PDQ hashes. The left image shows the original source image, and the rightmost image represents the desired target image. Each hash distance represents the difference between that image and the target image. The intermediate images show generated collisions at hash distances of 88, 38 and 0 respectively. It is easy to see that image quality is greatly reduced as hash distance decreases.

*[p. 16]*

PhotoDNA Targeted-Second-Preimage Generation Attack Progression

Figure 7: Targeted Second-Preimage-Generation Attack progression for 30 image pairs selected at random from ImageNet validation set [36]. PhotoDNA (left) achieved the baseline collision threshold within 20k iterations for 17 image pairs, PDQ (right) is able to reach the baseline threshold for all 30 runs.

PhotoDNA Targeted-Second-Preimage Generation Attack Progression

Figure 8: Targeted Second-Preimage-Generation Attack for the same 30 image pairs as above showing the relationship between L1 hash distance and L2 attack image distance for PhotoDNA (left) and PDQ (right).

*[p. 17]*

Figure 9: Detection avoidance attack on PhotoDNA. The starting image is on the left. The center image is the baseline avoidant image with a hash distance of 1800 from the starting image, and the rightmost image is an avoidant image with a hash distance of 4000. The images also show the L2 similarity metric between the starting image and each avoidant image.

baseline threshold ∆d within 20,000 iterations. For PDQ, all 30 image pairs reached the baseline threshold within 6,350 iterations. Figure 7 shows the distance from the hash of the attack image to the hash of the target image as it progresses over the duration of the attack. Due to the hash domain being significantly smaller for PDQ, small changes to the attack image do not affect the hash as much, requiring increased hyperparameters. This causes our attack to learn in coarse steps for PDQ, never taking a step which increases the hash distance, whereas PhotoDNA has more fine-grained direction, allowing for smoother convergence. We also would like to understand how the quality of the image changes as the attack progresses. We use the L2 distance metric between the source image and the perturbed attack image for this purpose. We plot this L2 distance versus the L1 hash distance in Fig. 8. We can see that as the L2 distance increases (i.e., image quality degrades), the hash distance decreases. In other words, the worse the image, the better the collision. Two hyperparameters differed between our attacks - PDQ required a learning rate of 5 and δ of 100 whereas PhotoDNA required 0.5 and 1 respectively to converge^8. Due to these changes, additional learning on PDQ past the baseline led to noisy images, thus runs were stopped early. All image pairs that converged (17 for PhotoDNA, 30 for PDQ) can be found in Appendix A.

Attack runtime. We aimed for a 4 hour completion time for each of our targeted-second-preimage attacks in our progression experiments above. On average each of our PhotoDNA attacks took approximately 4 hours to complete 20,000 iterations, after which 17 of our 30 randomly-selected image pairs converged (i.e., ∆d < 1, 800) On the other hand, our PDQ attacks achieved convergence (i.e., ∆d < 90) for all 30 images in approximately 3 hours on average, corresponding to 600–2,000 iterations. The PhotoDNA attack time is not optimal: the need to execute PhotoDNA as a binary on CPU no doubt increased the attack time significantly, and further optimization and parallelization is likely to substantially improve overall runtime.

## 5.3.2 Detection Avoidance

Jain et al. [19] evaluated a detection evasion attack against PDQ and other hashes but not PhotoDNA; we complement their work by presenting results on PhotoDNA. Using the methodology described in Section 4,

8This refers to the perturbation parameter δ described in [8]. This has no relation to our hash distance notation ∆d.

*[p. 18]*

we are able to generate images with hashes that satisfy the matching thresholds we chose in the previous section. We attempt to generate an image whose distance from the original is above the matching threshold ∆d while remaining perceptually similar to it.^9 We ran the attack against PhotoDNA for 300 iterations and presented the image with the lowest L2 distance to the target image.

Attack runtime. Unlike targeted-second-preimage generation, detection avoidance was relatively quick to execute. Our attack against PhotoDNA took less than 1 hour.

Results. Figure 9 shows an image of a boat, perturbed such that the PhotoDNA hash distances from the original image are above 1800 and 4000, respectively. Both of the perturbed images remain perceptually equivalent to the original, whereas the distance in hash space increases beyond the baseline ∆d = 1800 to even remain relatively clear at ∆d = 4000.

*[p. 19]*

## 7 Conclusion

Perceptual hash functions have become an increasingly important component of the modern communications infrastructure, generally with only limited consideration of their properties. In the past this has been an acceptable tradeoff, because hash matching was performed online and confidentiality was not guaranteed by encrypted communications systems. In the next era of increasingly E2EE communications systems, the use of hash-based matching in E2EE protocols may carry significantly higher risks. Our results demonstrate that these functions are vulnerable to machine-learning based attacks that produce both collisions and avoidant images, and that these attacks have the potential to violate the guarantees of E2EE communication participants. Going forward this observation should help guide the designers of future PHF/PHM systems, and inform the debate around the viability of deploying these systems within encrypted communication mechanisms. Our results leave several open problems. While we focused on PhotoDNA and PDQ, there are several other hash functions that, while less widely-deployed in PHM systems, could also be analyzed using our techniques. Additionally, in this work we did not evaluate whether it was possible to extract information from PhotoDNA and PDQ hashes. Finally, there are many questions facing practitioners and policymakers surrounding the design of future scanning systems, if these systems are to be built.
