# Cross-model comparison

## Median total tokens and accuracy

| task | deepseek-v4p1-flash<br>control | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>control | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|
| `diff-001` | 35,608<br>5/5 | 54,320<br>5/5 | 35,077<br>5/5 | 386,450<br>1/5 | 210,226<br>1/5 | 203,658<br>2/5 |
| `imports-001` | 14,245<br>5/5 | 6,534<br>5/5 | 15,577<br>5/5 | 27,487<br>4/5 | 56,834<br>3/5 | 8,326<br>5/5 |
| `outline-001` | 9,872<br>5/5 | 10,789<br>5/5 | 23,109<br>5/5 | 84,876<br>5/5 | 101,900<br>3/5 | 121,319<br>4/5 |
| `symbol-001` | 9,550<br>5/5 | 11,796<br>5/5 | 14,687<br>5/5 | 14,389<br>2/5 | 6,202<br>5/5 | 7,572<br>5/5 |
| `workflow-001` | 882,406<br>5/5 | 928,377<br>5/5 | 605,986<br>5/5 | 366,803<br>5/5 | 606,263<br>3/5 | 505,470<br>5/5 |

## Mean cost USD and accuracy

| task | deepseek-v4p1-flash<br>control | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>control | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|
| `diff-001` | $0.0059<br>5/5 | $0.0067<br>5/5 | $0.0052<br>5/5 | $0.0102<br>1/5 | $0.0062<br>1/5 | $0.0043<br>2/5 |
| `imports-001` | $0.0010<br>5/5 | $0.0007<br>5/5 | $0.0013<br>5/5 | $0.0012<br>4/5 | $0.0016<br>3/5 | $0.0003<br>5/5 |
| `outline-001` | $0.0018<br>5/5 | $0.0009<br>5/5 | $0.0012<br>5/5 | $0.0023<br>5/5 | $0.0026<br>3/5 | $0.0089<br>4/5 |
| `symbol-001` | $0.0010<br>5/5 | $0.0007<br>5/5 | $0.0011<br>5/5 | $0.0003<br>2/5 | $0.0001<br>5/5 | $0.0002<br>5/5 |
| `workflow-001` | $0.0438<br>5/5 | $0.0427<br>5/5 | $0.0308<br>5/5 | $0.0082<br>5/5 | $0.0268<br>3/5 | $0.0081<br>5/5 |

## Token reduction vs control (median, higher is better)

| task | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|
| `diff-001` | -53% | +1% | +46% | +47% |
| `imports-001` | +54% | -9% | -107% | +70% |
| `outline-001` | -9% | -134% | -20% | -43% |
| `symbol-001` | -24% | -54% | +57% | +47% |
| `workflow-001` | -5% | +31% | -65% | -38% |

## Cost reduction vs control (mean, higher is better)

| task | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|
| `diff-001` | -13% | +11% | +39% | +58% |
| `imports-001` | +30% | -36% | -36% | +77% |
| `outline-001` | +52% | +32% | -16% | -296% |
| `symbol-001` | +35% | -4% | +68% | +50% |
| `workflow-001` | +3% | +30% | -228% | +0% |
