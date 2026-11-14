#!/usr/bin/env bash
# run-qemu.sh: scripted minios boot regression.
#
#   scripts/run-qemu.sh                 smoke: expect banner + shell prompt
#   scripts/run-qemu.sh --usertests     boot, run usertests, judge ALL TESTS PASSED
#
# QEMU's serial console is a FIFO in this script (not stdio), so the test
# command is injected only after the shell prompt appears. The script polls the
# serial log and exits as soon as the expected line arrives; no fixed sleep
# long enough to hide a hang.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE=smoke
TIMEOUT=15
DO_BUILD=1
KERNEL=""

usage() {
    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
    printf 'options: --usertests --smoke --timeout SECS --no-build --kernel PATH\n'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --usertests) MODE=usertests; TIMEOUT=180; shift ;;
        --smoke) MODE=smoke; shift ;;
        --timeout) TIMEOUT="${2:?--timeout needs a value}"; shift 2 ;;
        --no-build) DO_BUILD=0; shift ;;
        --kernel) KERNEL="${2:?--kernel needs a path}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

command -v qemu-system-i386 >/dev/null || { echo 'qemu-system-i386 not found' >&2; exit 1; }

if [ "$DO_BUILD" = 1 ]; then
    make -s build
    if [ "$MODE" = usertests ]; then
        # usertests creates usertests.ran; mkfs must start from a clean image.
        rm -f user/fs.img
    fi
    make -s fs.img
fi

if [ -z "$KERNEL" ]; then
    TARGET_DIR="$(cargo metadata --format-version 1 --no-deps \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    KERNEL="$TARGET_DIR/i686-unknown-linux-gnu/release/kernel"
fi
[ -f "$KERNEL" ] || { echo "kernel not found: $KERNEL" >&2; exit 1; }

LOG="${MINIOS_QEMU_LOG:-/tmp/minios-qemu.log}"
FIFO="$(mktemp -u /tmp/minios-qemu-in.XXXXXX)"
mkfifo "$FIFO"

FEEDER=""
QPID=""
cleanup() {
    [ -n "$FEEDER" ] && kill "$FEEDER" 2>/dev/null || true
    [ -n "$QPID" ] && kill "$QPID" 2>/dev/null || true
    rm -f "$FIFO"
}
trap cleanup EXIT INT TERM

: > "$LOG"
if [ "$MODE" = usertests ]; then
    # Wait for the shell, then inject. Keeping the FIFO write end open avoids
    # an early serial EOF while usertests is still running.
    (
        for _ in $(seq 1 200); do
            grep -q 'init: starting sh' "$LOG" && break
            sleep 0.1
        done
        sleep 1
        printf 'usertests\n'
        exec sleep $((TIMEOUT + 10))
    ) > "$FIFO" &
else
    ( exec sleep $((TIMEOUT + 10)) ) > "$FIFO" &
fi
FEEDER=$!

qemu-system-i386 -kernel "$KERNEL" \
    -drive file=user/fs.img,index=1,if=ide,format=raw \
    -m 256M -serial stdio -display none -no-reboot -monitor none \
    < "$FIFO" > "$LOG" 2>&1 &
QPID=$!

START=$SECONDS
VERDICT=""
while kill -0 "$QPID" 2>/dev/null; do
    if grep -q 'ALL TESTS PASSED' "$LOG"; then VERDICT=pass; break; fi
    if grep -Eq 'panic:|kernel panic|PANIC' "$LOG"; then VERDICT=panic; break; fi
    if [ $((SECONDS - START)) -ge "$TIMEOUT" ]; then VERDICT=timeout; break; fi
    sleep 0.5
done
[ -n "$VERDICT" ] || VERDICT=exited

# Terminate qemu before judging, so wait cannot block on a passing run.
kill "$QPID" 2>/dev/null || true
wait "$QPID" 2>/dev/null || true
QPID=""

case "$MODE" in
    usertests)
        if grep -q 'ALL TESTS PASSED' "$LOG"; then
            echo 'run-qemu: usertests PASS (ALL TESTS PASSED)'
            exit 0
        fi
        echo "run-qemu: usertests FAIL ($VERDICT)" >&2
        ;;
    smoke)
        if grep -q 'minios v' "$LOG" && grep -q 'init: starting sh' "$LOG"; then
            echo 'run-qemu: smoke PASS (banner + shell)'
            exit 0
        fi
        echo "run-qemu: smoke FAIL ($VERDICT)" >&2
        ;;
esac
echo "serial log: $LOG" >&2
tail -40 "$LOG" >&2
exit 1
