#!/usr/bin/env bash
# #148's absent-validator probe. The frozen protocol is `n148-preregistration.md`; this only sets the
# environment that protocol names and calls the campaign harness unchanged, so the n127 artifact keeps its
# own defaults and its own meaning.
#
# Arm A (the issue's shape):  REPO=<dev worktree> spec/audit/evidence/n148-run.sh
# Arm B (attribution control, only if A reproduces — no kill):
#                             KILL_AT=9999 REPO=<dev worktree> spec/audit/evidence/n148-run.sh
#
# `REPO` must point at a checkout of the tree under test: `tools/devnet.sh build` builds `rnode:local`
# from the current directory and the harness `cd`s to `$REPO`, so the image and the tree must agree.
set -u

REPO=${REPO:-$(git rev-parse --show-toplevel)}
ATTEMPTS=${ATTEMPTS:-3}
CAP=${CAP:-8g}
# 420 s, not the campaign's 300 s: the single deploy lands at T+180, so this leaves 240 s after it — wider
# than the ~120 s in which #148 saw 351 blocks.
WINDOW_S=${WINDOW_S:-420}
KILL_AT=${KILL_AT:-120}

# No deploy before the kill; exactly one after it. `DEPLOY_AT=0`/`DEPLOYS=0` is the idle-before shape #148
# describes; `DEPLOYS_AGAIN=1` is the single deploy that re-arms it.
DEPLOY_AT=0
DEPLOYS=0
DEPLOY_AGAIN_AT=180
DEPLOYS_AGAIN=1

export REPO ATTEMPTS CAP WINDOW_S KILL_AT DEPLOY_AT DEPLOYS DEPLOY_AGAIN_AT DEPLOYS_AGAIN

echo "==> arm: kill at T+${KILL_AT}s (window ${WINDOW_S}s$([ "$KILL_AT" -gt "$WINDOW_S" ] && echo ' — no kill'), one deploy at T+${DEPLOY_AGAIN_AT}s"
exec spec/audit/evidence/n127-liveness-run.sh
