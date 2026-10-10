# #144 soft-checkpoint campaign summary

| run | tree | processed | snapshots/deploy | checkpoint ms/deploy | deploy-unit ms/deploy | fraction |
|---|---|---:|---:|---:|---:|---:|
| run-1.json | `74cda38f14e14ab34f03b75d9989ae1f0c38c94d` | 95/200 | 4.0 | 0.7394734000000001 | 1.9261535157894736 | 38.391% |
| run-2.json | `74cda38f14e14ab34f03b75d9989ae1f0c38c94d` | 96/200 | 4.0 | 0.8718719375 | 2.0681599166666667 | 42.157% |
| run-3.json | `74cda38f14e14ab34f03b75d9989ae1f0c38c94d` | 99/200 | 4.0 | 0.7971648585858586 | 2.0895426565656567 | 38.150% |

## Frozen decision

**VOID**

- run-1.json: processed=95/200
- run-1.json: node log matched consensus-failure void pattern: 'Self-created block #51 (seq 51) failed validation'
- run-2.json: processed=96/200
- run-2.json: node log matched consensus-failure void pattern: 'Self-created block #51 (seq 51) failed validation'
- run-3.json: processed=99/200
- run-3.json: node log matched consensus-failure void pattern: 'Self-created block #51 (seq 51) failed validation'
