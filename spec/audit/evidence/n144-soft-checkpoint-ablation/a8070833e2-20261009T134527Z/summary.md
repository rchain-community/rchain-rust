# #144 paired soft-checkpoint ablation

| pair | baseline ms/deploy | ablation ms/deploy | reduction | baseline snapshots | ablation snapshots | baseline checkpoint fraction | ablation checkpoint fraction |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1.969679 | 1.925970 | 2.219% | 4.0 | 3.0 | 42.291% | 35.309% |
| 2 | 2.248612 | 1.715502 | 23.708% | 4.0 | 3.0 | 38.671% | 33.137% |
| 3 | 2.093248 | 1.682811 | 19.608% | 4.0 | 3.0 | 38.869% | 38.986% |

## Frozen decision

Median paired deploy-unit reduction: **19.608%**.
Worst pair: **2.219%**.

**Decision: ablation-earns-merge.**
