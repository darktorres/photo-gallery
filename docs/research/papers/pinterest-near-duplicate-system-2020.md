# Evolution of a Web-Scale Near Duplicate Image Detection System

Andrey Gusev and Jiajing Xu (Pinterest). The Web Conference (WWW '20), 2020. <https://doi.org/10.1145/3366423.3380031>. Published under CC BY 4.0.

Excerpts taken on 2026-11-13 from the PDF `Evolution of a Web-Scale Near Duplicate Image Detection System.pdf` (7 pages), which is not kept: the whole paper except the acknowledgments and references; the three pseudo-code listings are replaced by notes. The text is the PDF's text layer as is, joined into paragraphs. Figures, table bodies and display equations are left out, except where marked *transcribed*: those were copied by hand from the rendered page. Exponents are written with `^` (s^r), and `*[p. N]*` marks the PDF page.

Kept for: PLAN §3 items 1, 2, 4, 5 and 6 (tiers, radius query, non-transitivity and k-cut, cluster heads with augmentation) and §10.

---

*[p. 1]*

Detecting near duplicate images is fundamental to the content ecosystem of photo sharing web applications. However, such a task is challenging when involving a web-scale image corpus containing billions of images. In this paper, we present an efficient system for detecting near duplicate images across 8 billion images. Our system consists of three stages: candidate generation, candidate selection, and clustering. We also demonstrate that this system can be used to greatly improve the quality of recommendations and search results across a number of real-world applications. In addition, we include the evolution of the system over the course of six years, bringing out experiences and lessons on how new systems are designed to accommodate organic content growth as well as the latest technology. Finally, we are releasing a humanlabeled dataset of ~53,000 pairs of images introduced in this paper.

near-duplicate detection, recommendation systems, locality sensitive hashing, transfer learning, clustering

ACM Reference Format: Andrey Gusev\* and Jiajing Xu\*. 2020. Evolution of a Web-Scale Near Duplicate Image Detection System. In Proceedings of The Web Conference 2020 (WWW ’20), April 20–24, 2020, Taipei, Taiwan. ACM, New York, NY, USA, 7 pages. <https://doi.org/10.1145/3366423.3380031>

## 1 INTRODUCTION

Near duplicate (near-dupe) image detection is a task of clustering exact and near identical images. There is no universal precise definition of near-dupe as it is very application platform specific. Typically, we refer to the same image with pixel-level modifications (e.g. cropping, flipping, scaling, rotating), or with a different perspective as a near-dupe image. Figure 1 shows some examples of near-dupe image pairs.

\*Both authors contributed equally.

This paper is published under the Creative Commons Attribution 4.0 International (CC-BY 4.0) license. Authors reserve their rights to disseminate the work on their personal and corporate Web sites with the appropriate attribution. WWW ’20, April 20–24, 2020, Taipei, Taiwan © 2020 IW3C2 (International World Wide Web Conference Committee), published under Creative Commons CC-BY 4.0 License. ACM ISBN 978-1-4503-7023-3/20/04. <https://doi.org/10.1145/3366423.3380031>

Figure 1: Examples of near duplicate image pairs.

Figure 2: Results for search query "lisa blackpink kill this love" on Pinterest: (a) without image de-dupe filter (undesired user experience) (b) with image de-dupe filter.

Near-dupe image detection is fundamental to photo-sharing web applications, such as Pinterest. For example, users expect to see relevant and diverse image search results. An excessive amount of near-dupe images would lead to an undesired user experience, as shown in Figure 2a. As another example, recommendation systems can benefit from less noisy label and training data by aggregating meta data from near-dupe images. However, near-dupe image detection remains a challenging problem for a web-scale image corpus, because it is difficult to (1) capture the criteria of near-dupe and build a high-precision classifier, (2) scale to billions of images, and (3) process new images incrementally. Prior web-scale efforts [9, 14, 22, 27] have mainly focused on one of the above problems. But there has been limited work that addresses all of the above problems. This paper presents the design of our near-dupe image detection system, and shares our approach to handling the above three challenges. For the first challenge, we create a gold standard dataset using a crowdsourcing platform and train an initial classifier to optimize for precision using this dataset; we then bootstrap over synthetic labels produced by the initial classifier and train a more

*[p. 2]*

(b) Figure 3: (a) System Diagram (b) Output of the near-dupe image detection system are near-dupe clusters. Each row represents one near-dupe cluster, with the cluster head in dotted line and a few examples from the cluster.

powerful classifier based on visual embeddings. For the second challenge, we map the visual embeddings into locality sensitive hashing (LSH) [12] terms and implement batch LSH search efficiently on Spark. For the third challenge, we develop an incremental update algorithm to process a new batch of images on a daily basis. Our system detects near-dupe images across 8 billion images, the largest dataset to the best of our knowledge. Our method has three stages: candidate generation using a batch LSH algorithm, candidate selection through transfer learning over visual embeddings, and near-dupe clustering with incremental updates. We describe how to apply near-dupe signals to make a big impact on real-world applications such as Pinterest. We also include how to aggregate meta data in a general recommendation system using near-dupe signals. We share the lessons learned from the evolution of the system over the past six years, including methods that could improve efficiency. The rest of the paper is organized as follows: Sections 2 and 3 survey related works and give a brief overview of the near-dupe image detection systems built over the years. Section 4 presents in detail the candidate generation, candidate selection, and clustering components of the latest system. Sections 5 and 6 describe how near-dupe signals can be used to enhance Pinterest experiences. Section 7 shares what we have learned over the system’s evolution.

2 RELATED WORK 2.1 Image Representation and Candidate Generation

Previous work represents images by global descriptor [19, 22, 30], local descriptor [14, 18, 20], bag-of-visual-words (BOW) [1, 2], and a hybrid of global and local descriptor [27, 33]. Global descriptor is easy to compute but is vulnerable to heavy editing such as cropping. Local descriptor and BOW-based methods are great alternatives to address this issue [1, 2, 21, 29, 32]. The BOW model, based on local descriptor, is favorable for efficiency and scalability. One challenge for the BOW model is that global context is lost in the representation. For the task of detecting near-dupe copies, an overlapping region-based global context descriptor is developed to increase the robustness of the features [32]. In recent years, deep convolutional neural networks (CNN) have been extremely successful for computer vision applications and have become the dominant approach

for classification [16, 24] and object detection [6, 8]. CNN have also been used for the task of near-dupe detection [11, 25, 31]. However, training an end-to-end CNN can be time-consuming and costly, and it also comes with extra computation and storage cost for the task-specific image features. For web-scale application, it is more desirable to reuse visual embeddings that are already available. In this work, each image is represented by visual embeddings trained for other computer vision tasks in [13]. For candidate generation, web-scale efforts use variants of approximate nearest neighbor search (aKNN) [22] and min-hash [3] to generate cluster seeds and grow the clusters [14, 27]. LSH, an aKNN method that maps high-dimensional image representations into a low-dimensional space to produce binary hash code, has recently become a popular indexing method for large-scale datasets [1, 3, 11, 17, 26]. This paper adopts the LSH method and optimizes the implementation extensively on Spark. We discuss in Section 7 that task-agnostic LSH works well as a candidate generator.

## 2.2 Near-dupe Classifier and Web-scale System

Prior web-scale near-dupe detection systems use a decision stump as the near-dupe classifier: [22] is based on global descriptors; [27] relies on the number of overlapping visual words; and [14] uses the Jaccard similarity of the sets of visual words of two images. Because of the limitations of the global and local descriptors, [14] has encountered noisy clusters, which limits its application in recommendation systems. Geometric verification, e.g. RANSAC [7], greatly improves the precision of the classifier [27], but is very computationally expensive on a web-scale dataset. In this work, we collect human curated near-dupe criteria, and train an embeddingbased deep near-dupe classifier to replace RANSAC in order to achieve high precision efficiently. Compared with existing webscale efforts, we improve the design such that our system is capable of incremental updates.

## 3 SYSTEM OVERVIEW AND EVOLUTION

The near-dupe image detection system is composed of three main components illustrated in Figure 3a: candidate generation largely solves scaling and drives the recall, candidate selection prunes the candidates to drive the precision, and clustering approximates equivalence partitions over the universe of images.

*[p. 3]*

Table 1: Evolution of the Near-dupe system over time.

First Generation (Mid 2013) Second Generation (Early 2014) Third Generation (Mid 2018) Applications De-duplicate search feed Aggregate pin meta data Remove copyrighted content Canonical pin selection De-duplicate home feed Requirements Medium precision Medium recall High precision Medium recall High precision Medium-high recall High efficiency Image Representation Global features Bag-of-visual-words (BOW) Visual embedding Candidate Generation LSH Inverted index LSH on Spark Candidate Selection RANSAC RANSAC + Decision tree Deep classifier Clustering Map-Reduce Map-Reduce Greedy K-cut on Spark Detected Near-dupes in Image Corpus (Includes Spam) 23% 46% 64%

Each component has evolved drastically as application needs change and with the emergence of newer approaches and technology. Table 1 illustrates how each component evolves over time. The first near-dupe detection system, developed in 2013, aimed to remove near-dupe pin^1 images on the search page for better user experience, as shown in Figure 2. In this system, images were represented by global image descriptors, and we used LSH-based search to generate near-dupe candidates. At the candidate selection stage, we extracted local image descriptors and applied geometric verification using RANSAC as the near-dupe classifier. The clustering stage implemented Algorithm 2 on Map-Reduce, detailed in Section 4.3. The first system was efficient, but the drawback of global descriptors limited the recall and precision. Since false positives did not affect user experience in the search result, the system did not require high precision. As the volume of data grew significantly on Pinterest, new applications emerged and required higher precision, such as removing copyrighted images and aggregating pin meta data. In order to overcome the limitation of global image descriptors, the second generation system in 2014 adopted the popular BOW image representation, which is ideal for retrieval. We obtained near-dupe candidates by retrieving similar images from an inverted index containing all images. To improve the precision of the near-dupe classifier, we employed a two-stage near-dupe classifier containing (1) geometric verification using RANSAC to reject false matches and (2) a decision tree model to guard against near-dupe criteria. The decision tree model was trained using human-labelled data and used features such as differences in global image descriptors, size and location of the overlapping area of the two images after being geometrically aligned, etc. The second generation had much better recall and precision, but it was inefficient in space and had high computational cost. Leveraging the emergence of deep learning and Spark, the third generation focused on bringing the recall higher while increasing the efficiency of daily processing to keep up with the growth of the content corpus. The system was deployed in mid-2018. Section 4 describes the design in more detail.

1On Pinterest, a pin is created when a user bookmarks an image, and it contains meta data such as user-generated text description, board title, destination url, etc.

## 4 THE LATEST GENERATION

In this section, we present each component shown in Figure 3a.

## 4.1 Candidate Generation

4.1.1 Image Representation as LSH Terms. In this system, each image is represented by the visual embedding from [13]. To cluster images via the near-dupe relation, each day we compare tens of millions of new images to billions of existing clusters. Approaching such a nearest neighbor search without an optimization would yield quadratic time complexity and a runtime proportional to more than 10 quadrillion image comparisons. Instead, we use a reduction of visual embeddings into LSH terms to drastically improve the tractability of such a problem. Through a process based on bit selection of binarized embeddings, we first reduce the dimensionality of the original embedding space. Next, the derived bits are grouped into numeric LSH terms in a process that fundamentally trades off detection probability and runtime. The smaller the groupings, the more computationally expensive it is to run the nearest neighbor search, but this increases the probability of accurate detection. Nearest neighbor search uses LSH terms and their Jaccard overlap as an approximation of cosine similarity between vectors in the original embedded space.

4.1.2 Batch LSH Search. With every image being represented by a set of LSH terms, we build an inverted index and implement a batch-oriented search over images. It is important to note that we only search against images that represent the entire cluster (known as cluster heads). The selection of cluster heads is a combination of engagement- and content-based heuristics at the time of initialization of the cluster. This allows to reduce the search space with minimal recall loss due to candidate augmentation, described in Section 4.1.3. To make batch-oriented search efficient at a high level, we use a combination of functional transformations, compressed inverted indexes and joins to calculate a result set for all the query images at once. The batch LSH search is implemented on Spark and requires a series of optimizations, such as dictionary encoding [28] and variable byte encoding [4], to ensure we can handle the data volumes even in a much more computationally tractable LSH term space. A combination of dictionary encoding and variable encoding on image id can reduce inverted index storage space by 20%. Algorithm 1 details the transformations.

*[Algorithm 1: Candidate Generation on Spark (listing omitted; see the paper)]*

*[p. 4]*

4.1.3 Candidate Augmentation. A key observation is that if the query image does not match the cluster head via a trained classifier, it may match some other images in the cluster - even those that are extremely similar to the cluster head. To illustrate this, assume a cluster A←−{B: 99,C: 99, D: 70}, where Ais the cluster head, the similarity scores with respect to Aare as listed, ranging from 0 to 100. A query image Qmay have scored relatively poorly with cluster head A, i.e. sim(A,Q) = 10, which is below a chosen tℎresℎold. We have observed that sim(B,Q) or sim(C,Q) may be above the tℎresℎoldeven though Band Care extremely close to A visually. The combination of (A,Q) presents a difficult or adversarial example for the trained classifier in Section 4.2, but other images in the cluster may not necessarily share that characteristic. Top augmentation images that are close to the cluster head (e.g. B, C) are added to the candidate list and increase end-to-end recall of the system by 13%.

4.1.4 Bootstrapping Over High Precision. A number of iterations of the improvements in the near-dupe system have relied on data produced in a previous iteration and the property of near-exact precision. Candidate augmentation is also heavily reliant on nearexact precision of both the classifier and the consistency of the existing clusters. We are able to consider Qas matching cluster Awhen any of top knear-dupes in Amatch Q. This can only be possible when clusters are consistently precise. While the neardupe relation is not transitive, in general by keeping the classifier at near-exact precision we are able to make that assumption for close neighbors.

## 4.2 Candidate Selection

4.2.1 Near-dupe Classifier. Batch LSH is a powerful way to generate high recall while minimizing computational cost. However, it will generally not produce optimum precision and ranking of candidates. We use a pass over generated candidates via a supervised classifier to select the candidates that are similar enough to be considered near-dupes. The task of the classifier is to select near-dupe images from the candidates produced by LSH and the augmentation. Figure 4 illustrates how this classifier works. To improve learning and convergence for each pair of images, hamming bits (binarized visual embeddings from [13]) are XORed and fed into the input layer, followed by 3 layers of fully connected layer and a sigmoid. The classifier is implemented in TensorFlow with Adam optimizer [15] over a set containing more than one billion distinct pairs

Figure 4: Architecture of the near-dupe classifier.

described in Section 4.2.2. The inference over the trained network happens in the Spark context to efficiently vectorize and reduce overhead. With a network that has almost 10 million parameters, we achieve an average of 1-2ms per prediction on a cluster of r5d machines on Amazon Web Service.

4.2.2 Training Labels for the Classifier. In order to collect a large amount of training labels for the near-dupe classifier, we used detection results from the previous generation of the near-dupe detection system over LSH candidates as well as labels derived from augmentation as described in Section 4.2.3. The previous generation classifier is detailed in Section 3.

4.2.3 Augmentation Labels. Following the Augmentation procedure described in Section 4.1.3, we generate a new training set for the next generation classifier that specifically targets adversarial examples. In the provided example, note that even though Qwas (correctly) recorded into cluster Adue to the transitivity over B, the classifier was not able to handle sim(Q,A) correctly. Learning over such pairs is an effective way to increase accuracy.

## 4.3 Clustering

Finding an ideal partition over the universe of images is mathematically not well-defined, because the near-dupe relation is not transitive and therefore not an equivalence relation. For example, an image of a cat gradually morphs into an image of a dog as shown in Figure 5. Each iteration would fall well within the near-dupe criteria, yet it is not clear how to partition the sequence. We represent images in a graph, where nodes are the images and edges represent image similarity. We first apply the transitive closure algorithm (Algorithm 2) over coherent sub-clusters. Each sub-cluster is a set of images that pass the near-dupe threshold for a given query image.

Figure 5: An image of a cat can morph into an image of a dog through near-dupe hops.

*[Algorithm 2: Transitive Closure Algorithm (listing omitted; see the paper)]*

*[p. 5]*

Following transitive closure, we greedily perform k-cut to correct for any errors that transitive closure may have introduced over several sub-clusters, illustrated in Algorithm 3. We rely on the fact that transitive closure was formed from coherent sub-clusters to develop a greedy approximation algorithm that is tractable.

## 4.4 Incremental Update Algorithm

We design our algorithm to handle a new batch of images on a daily basis. The algorithm works in two stages. In the first stage, we use the new images to query against (a) existing image clusters (NvO job) and (b) new images (NvN job), shown in Table 2. We only run transitive closure for (b) since we want to recognize the new image cluster within the new images. In the second stage, we merge the near-dupe image mappings from the NvO job and the NvN job, with the preference of NvO over NvN. Members of the NvN cluster will be mapped to the old cluster if such a match is found. Otherwise the NvN cluster will be added as a new cluster. This enables us to continue to grow existing image clusters.

Table 2: Two-stage approach to process new images. (a) NvO job: use new images to match existing images (b) NvN job: use new images to match new images. TC: transitive closure.

Name Query set Index set TC + K-cut (a) NvO job New images Existing images No (b) NvN job New images New images Yes

We created a dataset^2 of ~53,000 pairs of images by using a crowdsourcing platform (86% non-near-dupe and 14% near-dupe image pairs). The dataset was evaluated by human judges who have achieved high accuracy against a smaller golden set. For each pair, 5 different human judges were asked whether the pair constitutes near-dupe images. To the best of our knowledge, there is no prior work that has created a human-curated dataset of this scale.

2Available at <https://www.pinterest.com/pin/100275529191933620>.

*[Algorithm 3: Greedy Approximate K-cut Algorithm (listing omitted; see the paper)]*

## 5.2 Classifier

We compared the performance of our near-dupe classifiers with the one in the most recent published web-scale system [14], by evaluating on the gold standard dataset. We implemented the neardupe classifier based on the Jaccard similarity between the visual words described in [14]. The Decision tree classifier is described in Section 3. The Base deep classifier described in Section 4.2.1 is trained with a dataset containing 100 million pairs of images generated based on the approach detailed in Section 4.2.2. We further improved the classifier performance by refining the training set. The Production deep classifier differed from the base deep classifier in increasing the size, diversity, and difficulty of the training examples to over 1 billion pairs of images. Table 3 lists the precision-recall (PR) and ROC AUC of the above classifiers. By comparing the performance of the classifiers using various image features, the visual embedding and the amount of difficult training data play a big role in improving the performance significantly. In the latest generation system, we chose an operating point that maximizes precision while having medium-to-high recall^3.

Table 3: Classifier performance on the gold standard dataset.

PR AUC ROC AUC Kim et al. [14] 0.58 0.83 RANSAC + Decision Tree (Gen. 2) 0.64 0.87 Base Deep Classifier 0.80 0.96 Production Deep Classifier (Gen. 3) 0.85 0.97

## 5.3 Cluster Pinterest Images

We applied the proposed method to a dataset of over 8 billion images crawled from Pinterest, that comes with the visual embedding from [13]. We found ~900 million clusters, containing ~5.4 billion (~64%) near-dupe images^4. The most common cluster size is 2, and average cluster size is 6. The resulting distribution of near-dupe cluster sizes meets the power law distribution, as shown in Figure 6a.

3One application of the system is to remove copyrighted content by finding near-dupe copies of the content. During one operation, human judges reviewed 6,700 detected near-dupe copies of one copyrighted image, and found 0 false positive. 4Prior web-scale works detected ~13%, 27%, 34% in their image corpus [14, 22, 27].

*[p. 6]*

(b) Figure 6: (a) Distribution of near-dupe cluster sizes on Pinterest dataset. (b) Increasing trend of the percentage of nonspam images created on Pinterest that are near-dupe.

We also conducted a longitudinal study on the percentage of near-dupe images on Pinterest by image creation time, and observed an increasing near-dupe ratio over time, in Table 6b.

## 6 APPLICATIONS AT PINTEREST

The proposed near-dupe image detection system processes all Pinterest images and handles new images on a daily basis. We show that this system is helpful in many applications on Pinterest [5, 10, 13, 23], where over 10 billion recommendations are made a day. Table 4 summarizes the lifts in user engagement (e.g. pin click/save) in A/B experiments, after using the near-dupe detection results. De-duplicate Pinterest Feeds. The near-dupe mapping can be used to filter out duplicated content in search result pages and the user home feed, illustrated in Figure 2. Aggregate Pin Meta Data. Pin meta data (e.g. descriptions, board titles) are aggregated among pin images from the same neardupe cluster, and these meta data are used in generating keywords for search engine optimization (SEO) and serving pin recommendations on Pinterest. For graph-based recommendation engines [5, 10], images within the same near-dupe cluster are condensed into one node, and this reduces the size of the graph by 2.5xand greatly increases the connectivity of the graph. Canonical Pin Selection. The near-dupe system can be leveraged to select the most appropriate pin based on the context of users. Given a cluster of near-dupe images, each image may link to a different pin. For example, recommendation systems can select the most localized content in the selected near-dupe cluster.

Table 4: Summary of engagement lift by using near-dupe mapping, measured through A/B experiments on Pinterest.

Applications Engagement Lift Volume in SEO Traffic +33% De-duplicate Search feed +11% De-duplicate Home feed +7% Canonical Image Selection +9%

## 7 LESSONS

Near-dupe Criteria. It is hard to define the exact criteria of neardupe, and we rely on a human-curated gold standard dataset to capture this, since our system aims to be used in recommendation systems. However, human judges have noisy votes - for example,

(b) Figure 7: (a) Ambiguous image pairs. (b) Near-dupe pairs that were incorrectly classified by earlier classifier.

image pairs in Figure 7a are classified as near-dupe by our system but are labeled as non-near-dupe by human judges. Since images with text overlay often provide little value in the recommendation system, such false positives are minor. Image Representation. We find that the semantics of the image is crucial to near-dupe classification. Global and local descriptors from previous systems are pixel-based, while the more powerful visual embedding contains both pixel- and semantic-level signals and best mimics how humans visualize and identify near-dupe pairs. Image pairs shown in Figure 7b are mis-classified by the previous classifier due to the limitation of the pixel-based descriptors, and are correctly classified by the latest system. The high precision of the system is key to enable the real-world applications in Section 6. Design Choice: LSH. For near-dupe detection task, a key metric for measuring the efficacy of LSH candidate generation is not recall@kbut rather recall@distance. Only neighbors that are visually close enough can be considered near-dupe; therefore, the aim is to efficiently find neighbors that fall within a given distance rather than to find all kclosest neighbors regardless of distance. We evaluated LSH recall by classifier selection on 5 billion LSH candidates produced by the near-dupe system; recall of candidate generation is over 99.99% - less than 0.01% of candidates outside of what LSH search generates would be detected as near-dupes by the classifier. Thus, LSH search works well in the near-dupe system. Synthetic Labels. By optimizing for precision in iterations of this system, we were able to build a new generation of the system by bootstrapping over synthetic training labels. The third generation is trained on a large number of examples labeled by the previous classifier. Section 5.2 shows that a combination of synthetic training labels and powerful embedded representation leads to improvements in accuracy characteristics and ability to generalize.

## 8 CONCLUSION

This paper presents the evolution of a web-scale near-dupe image detection system, and shares how we scale it to over 8 billion images with high precision, e.g. the use of visual embedding to decrease computational load for candidate generation and selection, improvement of the near-dupe classifier using synthetic labels, efficient implementation on Spark, and design for incremental update. We also showcase how this system allows recommendation systems to greatly improve relevance and density of content. By sharing our experiences, we hope real-world applications can benefit from de-duplicating and condensing image content on the Web.
