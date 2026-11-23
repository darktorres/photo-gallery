# On the De-duplication of LAION-2B

Ryan Webster, Julien Rabin, Loic Simon, Frederic Jurie (Unicaen, ENSICAEN). arXiv:2303.12733, 2023. <https://arxiv.org/abs/2303.12733>

Excerpts taken on 2026-11-13 from the PDF `On the De-duplication of LAION-2B.pdf` (7 pages), which is not kept: the abstract, §3 CLIP Feature Compression, §4 Image Similarity Search, §5 De-duplicating LAION 2B and §7 Conclusions. The text is the PDF's text layer as is, joined into paragraphs. Figures, table bodies and display equations are left out, except where marked *transcribed*: those were copied by hand from the rendered page. Exponents are written with `^` (s^r), and `*[p. N]*` marks the PDF page.

Kept for: PLAN §10 (a fixed threshold on quantised distances fails; use a relative one: Eq. Dup).

---

*[p. 1]*

Generative models, such as DALL-E, Midjourney, and Stable Diffusion, have societal implications that extend beyond the field of computer science. These models require large image databases like LAION-2B, which contain two billion images. At this scale, manual inspection is difficult and automated analysis is challenging. In addition, recent studies show that duplicated images pose copyright problems for models trained on LAION2B, which hinders its usability. This paper proposes an algorithmic chain that runs with modest compute, that compresses CLIP features to enable efficient duplicate detection, even for vast image volumes. Our approach demonstrates that roughly 700 million, or about a third of LAION-2B’s images, are duplicates. Our method also provides the histograms of duplication on this dataset, which we use to reveal more examples of verbatim copies by Stable Diffusion and further justify the approach. The current version of the de-duplicated set can be found at github.com/ryanwebster90/snip-dedup.

*[p. 2]*

## 3. CLIP Feature Compression

We begin with a generic auto-encoding baseline using mean squared error, which is a standard technique for feature compression [25]. Denoting (x^T , x^I) as minibatches of CLIP text and image features we have

where, D^I, E^I are the encoder/decoder networks for the image features. The compression rate is thus controlled by the output dimension of E^I, or the latent space dimension, which will be used for index creation and search later on. The compression can be done on either image CLIP or text CLIP descriptors. However, the limitation is that if the compression of the two modalities is done independently of each other, there is a risk of losing the alignment between the modalities. Our experiments with hybrid encoders (fusing modalities or mixing contrastive and reconstruction losses) showed worse results than this baseline or the methods presented next.

Contrastive Compression We propose a second compression method designed to preserve text and image feature alignment. To achieve this, we suggest using the original contrastive loss function proposed in [18]. There are several approaches to compressing features this way, including the use of an autoencoder or applying the clip loss directly in the original feature space. However, we found that using a ”latent” clip loss as LCLIP(E^T (x^T ), E^I(x^I)), with E^I, E^T

as the image and text encoders, was better overall and use this loss for all experiments. Although using CLIP loss alone was effective in some tasks, such as zero-shot ImageNet classification, we observed that to enable more accurate nearest neighbor search, it was better to also introduce a term to maintain distance properties between neighboring elements in the dataset. Therefore, we propose a new approach that involves computing the nearest neighbors (w.r.t. image features) in a ”chunk” (here, we used sliding chunks k = 10M samples), defined as follows

where 1-NN(x^I_k) is the nearest neighbor of sample x^I_k in the chunk k. The nearest neighbor search within each chunk is

*[p. 3]*

Figure 1. Investigating the effects of compression on several tasks requiring multi-modality of image features. MSE based compression (pca, mse) lose semantics for ImageNet classes at higher compression ratios, w.r.t. to the snip based losses.

done exhaustively by measuring the L2 distances between all the pairs of the chunk.

Approximate Search Even compressed, searching through CLIP descriptors at billions scale is still too expensive. We therefore use techniques for approximate nearest neighbor search akin to those found in Johnson et al. [11]. One of the simplest and oldest techniques for fast nearest neighbor search relies on an inverted file system (IVF). First one computes the k-means centroids of the database vectors and then groups vectors to their closest centroid to form the inverted files. During search, queries only look through the τ closest inverted files by exhaustive search over the closest centroids, which is tractable because the number of centroids is typically small. We explore building a two-level quantizer on top of our compressed features, which compresses database vectors as follows:

where y is first approximated with q1, and then its residual quantized by q2. k-NN’s are retrieved as those minimizing a quantity known as the asymmetric distance:

where q(y) is a quantized database vector. In the case when q1 is an IVF, this quantity is only minimized over database vectors mapping to the same centroid as the query or top τ centroids (referred to as the nprobe parameter). Specifically, we explore Product Quantization (PQ) as our secondary quantizer [9]. PQ splits vectors into M sub-vectors, y = [q^0(y^0), ...q^M(y^M)] where M is an even divisor of y^(′)s dimension and the sub-quantizers are again k-means. Higher values of M result in higher compression ratios.

## 4. Image Similarity Search

We generate a synthetic ground truth by computing a small set of k-nearest neighbors using brute force. We then

compared the index retrieval results with this ground truth as follows:

*[Equation, transcribed]* `R∩k := (1/N) · Σ_{q∈Q} |k-NN(q_i) ∩ k-ANN(q_i)| / k`

The approximate k-nearest neighbors (k-ANN) were computed for the index under review, while the exhaustive search on raw features (k-NN) served as the ground truth. It’s worth noting that the approach used to construct the ground truth for the (non-multimodal) Deep1B dataset [2] is similar to the one described in [11]. We did ablations over the latent space dimensions and found that encoding to a relatively high dimension of 128 first (from 1024 for ViT-H-14) was more effective, than for example encoding to a low dimension directly and giving more memory to the quantizer. We note that the approach in [11] also used a similar dimension for the compressed space before quantization. Also like in [11], we used the IVFPQ index for every index we built atop of our descriptors and finally did several comparisons to a vanilla IVFPQ on raw features. For the IVF, we chose 2^16 centroids, and for the PQ we constructed collections of indices with different values of M (the number of dimension chunks) with 2^8

centroids each. Finally, we highlight the popular open source tool AutoFaiss, which will automatically construct and efficient PQ index given a memory budget. All of our AutoFaiss indices use multiple PQ steps, with an initial optimized product quantization (OPQ) step for dimensionality reduction (see [6]), as well as a hierarchical navigable world search (HSNW) [15]. For instance, when providing the tool with “6G” total memory for the index, it produced a faiss construction pipeline of “OPQ16 112,IVF65536 HNSW32,PQ16x8” (Please see the faiss library for more details[11]). We noted these indices were not more effective on the GPU and excluded them from these benchmarks. To form Pareto fronts, we

*[p. 4]*

Figure 2. Image feature only retrieval pareto fronts for ViT-H-14 indices on L400M on the CPU (left), GPU (middle) and L2B on CPU (right). MSE based losses perform similarly on this task compared to the contrastive ones.

merely built a collection of indices as before, but varying the memory parameter.

Multi-modal Search Results The original CLIP network demonstrated impressive performance on the challenging zero-shot ImageNet classification task. With CLIP, this can be achieved by getting an encoding for each category c with the caption “this is an image of [c]”, and then performing an exhaustive search with CLIP encoded ImageNet samples via taking a maximum inner product (as is maximized for positive pairs during contrastive training). We’d like to not only explore how well our descriptors preserve multi-modal information, but the quantized representation in the index as well. For this, we reconstruct database vectors using the sa encode and sa decode functions of the faiss library, and then do multi-modal search on reconstructed vectors. We compare to AutoFaiss (see previous section), which is used for multi-modal search hosted by LAION [22], but in a more extreme compression regime. Figure 1 shows the performance of each index type versus the ”compression ratio,” which is simply the uncompressed size of all database features divided by the index size (plus a negligible size of the encoding network). Again, we create a collection of indices with varying M parameters to form the IVFPQ pareto fronts. The LMSE descriptor indices zero-shot performance are clearly less efficient than the LSNIP ones. Autofaiss performs the worst at these compression ratios (in general we saw a sharp drop in the AutoFaiss performance above a certain compression ratio for every task). For text-to-image search, our snip model also performs better than AutoFaiss. We now move on to a wider variety of comparisons.

Image to Image Search Results Figure 2 presents the retrieval benchmarks for the L400M and L2B datasets. We assessed each method across a range of benchmarks and created Pareto fronts by varying the number of chunks for product quantization (i.e., M = 2, 4, 8) and the n probe parameter (τ = 1, 2, 4). Notably, MSE and SNIP performed

similarly on image similarity search, while CLIP-only loss networks consistently performed worse. However, as mentioned, the CLIP networks (both CLIP and SNIP) do perform better on multi-modal tasks. Thus, SNIP can be seen as a best of both worlds descriptor for these types of indices. Figure 2 also indicates that indices similar to those in [12], using a PCA as a first compression and IVFPQ as a quantizer, have some drop off in performance, as with the CLIP only indices.

## 5. De-duplicating LAION 2B

In this section, we investigate several of the index creation pipelines discussed previously to de-duplicate LAION-2B. Our focus on more ”extreme” compression up until now is justified; de-duplicating such a large database quickly relies on a fast and thus compact index. The goal of this effort was to merely find many duplicates with decent precision quickly, rather than having a perfect deduplication. Our indices return the asymmetric distance (3), which can result in non-zero distances even for in-database queries. Additionally, since the secondary quantizer operates globally over the IVF, different centroids may contain significantly different dADC distributions. Consequently, we found that applying an absolute threshold on dADC alone was insufficient for effective de-duplication performance. Instead of using an absolute threshold on the dADC, which was found to be insufficient for good de-duplication performance due to the global nature of the secondary quantizer over the IVF, an adaptive threshold based on the dADC between a query vector and its quantized version was used. This approach takes into account that duplicated images may not necessarily have a low absolute dADC, but they will all have the same dADC. Specifically, duplicates y ∈ k-NN(x) of a query image x are identified if

*[Equation (Dup), transcribed]* `|d_ADC(x, x) − d_ADC(x, y)| / d_ADC(x, x) < T_ADC`

*[p. 5]*

It is worth noting that this threshold is applied only retrieved k-NN’s. The duplication detection performance was consistent with the previous retrieval experiments, and the best performing indices were selected to perform the deduplication of LAION2B. Table 5 presents the results of de-duplication experiments conducted on the LAION2B dataset using the ViT- L-14 and ViT-H-14 CLIP feature sets. To estimate the precision, we randomly selected 100k labeled duplicated images and computed a ground truth with their raw CLIP features. We chose a conservative threshold on the raw features, which would still only label near duplicates (via inspection). De-duplication was performed on a machine with 32 GB of RAM and took only several days. Finally, we also estimate precision and recall of merely indicating the presence of one duplicate (See Table 4) via exhaustive search, and again comparing to Eq. Dup.

Figure 3. We find several new verbatim copied images by Stable Diffusion (SD), by synthesizing the most copied images on L2B. Left are images duplicated database images and right are synthesized by SD. Top image is highly duplicated (more than 5k times), bottom image is moderately duplicated (2000 times).

To actually compute the de-duplication, we loop over the entire dataset using algorithm 1, that can be used with any index. We note this algorithm will also yield the representative sets, that can be used to group duplicated images together.

*[Table and Algorithm 1, transcribed. The table is captioned "Table 1" on the page; the text calls it Table 5.]*

| Index                    | Duplicates Found | Precision |
|--------------------------|------------------|-----------|
| ViT-L-14 MSE (GPU, M16)  | 624,636,782      | .91       |
| ViT-L-14 SNIP (GPU, M16) | 640,402,551      | .92       |
| ViT-H-14 SNIP (CPU, M4)  | 682,146,751      | .91       |

Table 1. De-duplication of L2B. The top two indices were sharded over 4-GPUs, with M=16 and the ViT-L-14 network. Below
is on ViT-H-14 features, on the cpu and not sharded. Note, the precisions aren't directly comparable here as we use
different raw clip thresholds for each. Image space comparisons will be indicated in future versions so they are
comparable.

```text
Algorithm 1  Deduplication of a dataset D
Require: index for k-NN search
  Set k the number of retrieved nearest neighbors
  Initialize list of representatives by self reference: ∀ i  R_i ← i
  for x_i ∈ D do
      S  ← k-NN(x_i)
      S′ ← {x_j ∈ S | j > i and x_j satisfies (Dup)}
      for x_j ∈ S′ do
          R_j ← R_i
```

*[p. 6]*

## 7. Conclusions

In this work, we discussed an algorithmic chain which has de-duplicated LAION-2B-en with modest resources and decent precision. We release code to download this deduplicated datasets, their representative sets (See Alg. 1, the duplication histograms and finally SNIP indices for dataset exploration. This is important for dataset usability; recent work demonstrates generative models have serious copyright issues when duplicates are present. Our work uses a novel pipeline which does not require pre-training or recomputation of billions of clip features and should provide an accessible tool to the community for greater dataset transparency.
