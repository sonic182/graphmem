# Cross-model comparison

## Run settings and limit stops

Accuracy and spend include all attempts, including limit stops. Resource
limits may differ across models; these are not controlled model-quality rankings.
Generation values are ceilings; a remaining token budget can lower a call's cap.

| model | attempts | generation ceiling | temperature | jobs | matrix cost ceiling | run token caps | limit stops | other errors |
|---|---:|---:|---:|---:|---:|---|---:|---:|
| `deepseek-v4p1-flash` | 75 | 16384 | 0.0 | 8 | not recorded | not recorded | 0 | 0 |
| `glm-5p3-flash` | 75 | 16384 | 0.0 | 8 | $1.14900851 | enabled | 4 | 0 |
| `nemotron-lightning-3p5-30b-a3b` | 75 | 16384 | 0.0 | 8 | not recorded | not recorded | 0 | 0 |

## Median total tokens and accuracy

| task | deepseek-v4p1-flash<br>control | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | glm-5p3-flash<br>control | glm-5p3-flash<br>gmem | glm-5p3-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>control | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `diff-001` | 35,170<br>5/5 | 37,317<br>5/5 | 45,720<br>5/5 | 28,768<br>5/5 | 17,296<br>5/5 | 34,884<br>5/5 | 219,897<br>0/5 | 130,536<br>1/5 | 172,188<br>0/5 |
| `imports-001` | 14,243<br>5/5 | 10,188<br>5/5 | 13,153<br>5/5 | 7,536<br>5/5 | 8,469<br>5/5 | 14,094<br>2/5 | 47,149<br>5/5 | 25,402<br>4/5 | 9,142<br>5/5 |
| `outline-001` | 9,621<br>5/5 | 11,516<br>5/5 | 23,788<br>5/5 | 8,696<br>5/5 | 13,321<br>5/5 | 21,441<br>5/5 | 71,437<br>4/5 | 99,003<br>4/5 | 67,050<br>5/5 |
| `symbol-001` | 9,227<br>5/5 | 11,790<br>5/5 | 14,734<br>5/5 | 7,842<br>4/5 | 9,245<br>5/5 | 13,462<br>5/5 | 49,643<br>3/5 | 6,202<br>5/5 | 7,493<br>5/5 |
| `workflow-001` | 583,517<br>5/5 | 1,161,091<br>5/5 | 967,132<br>5/5 | 215,887<br>5/5 | 427,251<br>4/5 | 348,693<br>5/5 | 272,629<br>5/5 | 797,354<br>4/5 | 453,126<br>3/5 |

## Mean cost USD and accuracy

| task | deepseek-v4p1-flash<br>control | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | glm-5p3-flash<br>control | glm-5p3-flash<br>gmem | glm-5p3-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>control | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `diff-001` | $0.0060<br>5/5 | $0.0061<br>5/5 | $0.0059<br>5/5 | $0.0046<br>5/5 | $0.0020<br>5/5 | $0.0036<br>5/5 | $0.0193<br>0/5 | $0.0050<br>1/5 | $0.0043<br>0/5 |
| `imports-001` | $0.0007<br>5/5 | $0.0008<br>5/5 | $0.0018<br>5/5 | $0.0007<br>5/5 | $0.0008<br>5/5 | $0.0015<br>2/5 | $0.0020<br>5/5 | $0.0030<br>4/5 | $0.0004<br>5/5 |
| `outline-001` | $0.0020<br>5/5 | $0.0016<br>5/5 | $0.0016<br>5/5 | $0.0010<br>5/5 | $0.0014<br>5/5 | $0.0021<br>5/5 | $0.0019<br>4/5 | $0.0025<br>4/5 | $0.0029<br>5/5 |
| `symbol-001` | $0.0018<br>5/5 | $0.0009<br>5/5 | $0.0012<br>5/5 | $0.0009<br>4/5 | $0.0006<br>5/5 | $0.0011<br>5/5 | $0.0011<br>3/5 | $0.0001<br>5/5 | $0.0004<br>5/5 |
| `workflow-001` | $0.0477<br>5/5 | $0.0481<br>5/5 | $0.0503<br>5/5 | $0.0236<br>5/5 | $0.0345<br>4/5 | $0.0240<br>5/5 | $0.0058<br>5/5 | $0.0327<br>4/5 | $0.0077<br>3/5 |

## Token reduction vs control (median, higher is better)

| task | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | glm-5p3-flash<br>gmem | glm-5p3-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|
| `diff-001` | -6% | -30% | +40% | -21% | +41% | +22% |
| `imports-001` | +28% | +8% | -12% | -87% | +46% | +81% |
| `outline-001` | -20% | -147% | -53% | -147% | -39% | +6% |
| `symbol-001` | -28% | -60% | -18% | -72% | +88% | +85% |
| `workflow-001` | -99% | -66% | -98% | -62% | -192% | -66% |

## Cost reduction vs control (mean, higher is better)

| task | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | glm-5p3-flash<br>gmem | glm-5p3-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|
| `diff-001` | -1% | +2% | +55% | +22% | +74% | +78% |
| `imports-001` | -15% | -153% | -15% | -107% | -53% | +79% |
| `outline-001` | +20% | +17% | -36% | -109% | -31% | -48% |
| `symbol-001` | +48% | +33% | +34% | -21% | +90% | +62% |
| `workflow-001` | -1% | -6% | -46% | -2% | -460% | -31% |
