# Cross-model comparison

## Median total tokens and accuracy

| task | deepseek-v4p1-flash<br>control | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>control | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|
| `diff-001` | 53,139<br>5/5 | 46,810<br>5/5 | 55,459<br>5/5 | 386,450<br>1/5 | 210,226<br>1/5 | 203,658<br>2/5 |
| `imports-001` | 7,429<br>5/5 | 10,058<br>5/5 | 12,766<br>5/5 | 27,487<br>4/5 | 56,834<br>3/5 | 8,326<br>5/5 |
| `outline-001` | 12,641<br>5/5 | 10,931<br>5/5 | 22,987<br>5/5 | 84,876<br>5/5 | 101,900<br>3/5 | 121,319<br>4/5 |
| `symbol-001` | 8,219<br>5/5 | 11,796<br>5/5 | 14,678<br>5/5 | 14,389<br>2/5 | 6,202<br>5/5 | 7,572<br>5/5 |
| `workflow-001` | 599,943<br>5/5 | 1,115,278<br>5/5 | 566,463<br>5/5 | 366,803<br>5/5 | 606,263<br>3/5 | 505,470<br>5/5 |

## Mean cost USD and accuracy

| task | deepseek-v4p1-flash<br>control | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>control | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|---:|---:|
| `diff-001` | $0.0079<br>5/5 | $0.0064<br>5/5 | $0.0057<br>5/5 | $0.0102<br>1/5 | $0.0062<br>1/5 | $0.0043<br>2/5 |
| `imports-001` | $0.0008<br>5/5 | $0.0006<br>5/5 | $0.0018<br>5/5 | $0.0012<br>4/5 | $0.0016<br>3/5 | $0.0003<br>5/5 |
| `outline-001` | $0.0023<br>5/5 | $0.0008<br>5/5 | $0.0018<br>5/5 | $0.0023<br>5/5 | $0.0026<br>3/5 | $0.0089<br>4/5 |
| `symbol-001` | $0.0009<br>5/5 | $0.0005<br>5/5 | $0.0009<br>5/5 | $0.0003<br>2/5 | $0.0001<br>5/5 | $0.0002<br>5/5 |
| `workflow-001` | $0.0411<br>5/5 | $0.0555<br>5/5 | $0.0274<br>5/5 | $0.0082<br>5/5 | $0.0268<br>3/5 | $0.0081<br>5/5 |

## Token reduction vs control (median, higher is better)

| task | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|
| `diff-001` | +12% | -4% | +46% | +47% |
| `imports-001` | -35% | -72% | -107% | +70% |
| `outline-001` | +14% | -82% | -20% | -43% |
| `symbol-001` | -44% | -79% | +57% | +47% |
| `workflow-001` | -86% | +6% | -65% | -38% |

## Cost reduction vs control (mean, higher is better)

| task | deepseek-v4p1-flash<br>gmem | deepseek-v4p1-flash<br>gmem-guided | nemotron-lightning-3p5-30b-a3b<br>gmem | nemotron-lightning-3p5-30b-a3b<br>gmem-guided |
|---|---:|---:|---:|---:|
| `diff-001` | +19% | +29% | +39% | +58% |
| `imports-001` | +33% | -113% | -36% | +77% |
| `outline-001` | +63% | +20% | -16% | -296% |
| `symbol-001` | +39% | +1% | +68% | +50% |
| `workflow-001` | -35% | +33% | -228% | +0% |
