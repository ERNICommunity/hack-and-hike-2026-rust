# Labeled Faces in the Wild (LFW)

The standard face verification benchmark, used by `facekit eval` to
measure the accuracy of the whole pipeline against a published number.
The photos (266 MB) are not part of the repository; `pairs.txt` is.
[`../README.md`](../README.md) has the steps for getting the photos,
together with the other photos facekit needs.

| File | What | From |
| --- | --- | --- |
| `lfw_funneled/` | 13,233 photos of 5,749 people, 250x250, one folder per person | <https://ndownloader.figshare.com/files/5976015> (scikit-learn's mirror of the UMass dataset) |
| `pairs.txt` | the 6,000-pair protocol: 10 folds of 300 matched and 300 mismatched pairs | <https://ndownloader.figshare.com/files/5976006> |

To download the photos, from the repository root:

```bash
cd tools/facekit/data/lfw
curl -L -o lfw-funneled.tgz https://ndownloader.figshare.com/files/5976015
tar xzf lfw-funneled.tgz && rm lfw-funneled.tgz
```

The original site, <https://vis-www.cs.umass.edu/lfw/>, did not answer
when this was written; the figshare files above are the mirror that
scikit-learn's `fetch_lfw_pairs` uses.

## The `pairs.txt` format

The first line is `10<tab>300`: ten folds of 300 matched and 300
mismatched pairs, 6,000 pairs in all. Then, per fold, 300 lines of

```
Name  i  j        both images are of Name: Name/Name_000i.jpg and _000j.jpg
```

followed by 300 lines of

```
NameA  i  NameB  j    two different people
```

The published protocol is ten-fold cross-validation: choose the
threshold on nine folds, measure accuracy on the tenth, repeat, and
report the mean and the standard deviation.
