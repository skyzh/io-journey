#!/usr/bin/env bash
set -euo pipefail

adapter_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$adapter_dir/../mount-state.sh"

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $(id -u) == 0 && -c /dev/ublk-control ]] || fail 'Run as root in a guest with ublk.'
require_private_mount_namespace
[[ $(findmnt -n -o FSTYPE -T /run) == tmpfs ]] || fail '/run must be tmpfs for ublk metadata.'

mode=ext4
mount_filesystem=ext4
profile_args=()
build_args=()
if (( $# != 0 )); then
    mode=$1
    case "$mode" in
        learner)
            [[ $# == 1 ]] || fail 'Use smoke.sh learner from your course checkout.'
            build_args=(--features learner)
            ;;
        queue-case)
            [[ $# == 5 ]] || fail 'Use queue-bench.sh for a benchmark.'
            [[ "$2" == fifo || "$2" == read-first ]] || fail 'Invalid scheduling policy.'
            [[ "$3" == 1 || "$3" == 8 ]] || fail 'Invalid benchmark depth.'
            [[ "$4" == 4096 || "$4" == 16384 ]] || fail 'Invalid benchmark request size.'
            [[ -d "$5" && $(findmnt -n -o FSTYPE -T "$5") == tmpfs ]] ||
                fail 'Benchmark output must be in guest tmpfs.'
            ;;
        fuse-case)
            [[ $# == 2 || ( $# == 3 && "$3" == learner ) ]] || fail 'Use linux/fuse/smoke.sh for the FUSE lab.'
            if (( $# == 3 )); then build_args=(--features learner-queue); fi
            fuse_binary=$(realpath -- "$2")
            [[ -x "$fuse_binary" ]] || fail 'Expected executable at '"$fuse_binary"
            mount_filesystem=fuse
            ;;
        append-case)
            [[ $# == 5 && ( "$2" == ext4 || "$2" == fuse ) &&
               ( "$3" == sync || "$3" == uring ) && ( "$4" == 4 || "$4" == 32 ) ]] ||
                fail 'Use linux/append/benchmark.sh for the append benchmark.'
            [[ -d "$5" && $(findmnt -n -o FSTYPE -T "$5") == tmpfs ]] ||
                fail 'Append benchmark output must be in guest tmpfs.'
            mount_filesystem=$2
            ;;
        *)
            fail 'Unknown lab workload.'
            ;;
    esac
fi

run_parent=/var/tmp
if [[ "$mode" == queue-case || "$mode" == append-case ]]; then
    run_parent=/dev/shm
fi
require_private_mount_parent "$run_parent"
run_dir=$(mktemp -d "$run_parent/io-journey-ublk.XXXXXX")
mount_dir="$run_dir/mount"
mount_name="${run_dir##*/}"
socket="$run_dir/control.sock"
log="$run_dir/server.log"
binary="$run_dir/target/debug/io-journey-ublk"
server_pid=
server_exit_status=0
device_id=
node_identity=
device=
trace_instance=
mkdir "$mount_dir"
source "$adapter_dir/build-env.sh"
source "$adapter_dir/cleanup.sh"
if [[ "$mount_filesystem" == fuse ]]; then
    source "$adapter_dir/../fuse/mount.sh"
fi
if [[ "$mode" == queue-case || "$mode" == append-case ]]; then
    binary=$(realpath "${IO_JOURNEY_ADAPTER_BIN:?}")
    [[ -x "$binary" ]] || fail 'Expected executable at '"$binary"
fi
if [[ "$mode" == queue-case ]]; then
    profile_args=("$2" "$5/server.tsv")
fi

shopt -s nullglob
initial_devices=(/sys/class/block/ublkb*)

read_allocation() {
    if [[ -f "$log" ]]; then
        local allocation
        allocation=$(awk -v pid="$server_pid" '
            $1 == "allocated" && $2 ~ /^[0-9]+$/ && $3 == pid &&
            $4 ~ /^[0-9]+:[0-9]+:[0-9]+$/ {print; exit}
        ' "$log")
        read -r _ device_id _ node_identity <<< "$allocation"
    fi
}

server_exited() {
    [[ ! -d "/proc/$server_pid" ]] ||
        grep -Eq '^State:[[:space:]]+Z' "/proc/$server_pid/status" 2>/dev/null
}

reap_server() {
    local attempt
    for attempt in {1..50}; do
        if server_exited; then
            wait "$server_pid" 2>/dev/null || server_exit_status=$?
            return 0
        fi
        sleep 0.1
    done
    return 1
}

device_remains() {
    [[ -n "$device_id" ]] || return 1
    local metadata
    printf -v metadata '/run/ublksrvd/%04d.json' "$device_id"
    [[ -e "/sys/class/block/ublkb$device_id" || -e "/dev/ublkb$device_id" ||
       -e "/sys/class/ublk-char/ublkc$device_id" || -e "/dev/ublkc$device_id" ||
       -e "$metadata" || -L "$metadata" ]]
}

trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ "$mode" == ext4 || "$mode" == fuse-case || "$mode" == learner ]]; then
    build_adapter "$adapter_dir" "$run_dir" "${build_args[@]}"
fi
"$binary" serve "$socket" "${profile_args[@]}" > "$log" 2>&1 &
server_pid=$!
for attempt in {1..150}; do
    read_allocation
    if [[ -n "$device_id" && -b "/dev/ublkb$device_id" ]] &&
        grep -qxF "ready $device_id" "$log"; then
        device="/dev/ublkb$device_id"
        break
    fi
    kill -0 "$server_pid" 2>/dev/null || fail 'The ublk server exited before readiness.'
    sleep 0.1
done
[[ -n "$device" ]] || fail 'The ublk server did not report a ready block device.'
sys_block="/sys/class/block/ublkb$device_id"
for prior in "${initial_devices[@]}"; do
    [[ "$prior" != "$sys_block" ]] || fail 'The allocated device was present before this run.'
done
"$binary" check-owned "$device_id" "$server_pid" "$node_identity" "$socket"
[[ $(blockdev --getsize64 "$device") == 33554432 &&
   $(blockdev --getss "$device") == 4096 ]] || fail 'The allocated device has unexpected geometry.'
! findmnt -rn -S "$device" >/dev/null || fail 'The allocated device is already mounted.'
[[ $(cat "$sys_block/queue/write_cache") == 'write back' &&
   $(cat "$sys_block/queue/fua") == 0 ]] || fail 'The ublk cache features differ from the model.'
printf 'owned device %s uses one ublk queue, write-back cache and separate flushes\n' "$device"

control() {
    "$binary" control "$socket" "$1"
}

if [[ "$mode" == queue-case ]]; then
    source "$adapter_dir/queue-case.sh"
    run_queue_case "$2" "$3" "$4" "$5"
    exit 0
fi
if [[ "$mode" == fuse-case ]]; then
    bash "$adapter_dir/../fuse/workload.sh" "$fuse_binary" "$binary" "$socket" "$server_pid" \
        "$node_identity" "$device_id" "$mount_dir" "$mount_name"
    exit 0
fi
if [[ "$mode" == append-case ]]; then
    udevadm info --wait-for-initialization=10 --query=name --name="$device" >/dev/null
    udevadm settle --timeout=10
    udevadm lock --device="$device" --timeout=10 -- bash "$adapter_dir/../append/case.sh" \
        "$2" "$3" "$4" "$5" "$binary" "$socket" "$server_pid" "$node_identity" "$device_id" \
        "$mount_dir" "$mount_name"
    exit 0
fi

mkfs.ext4 -q -F -b 4096 -m 0 -E nodiscard,lazy_itable_init=0,lazy_journal_init=0 "$device"
mount -t ext4 "$device" "$mount_dir"
before=$(control stats)
read -r _ before_writes before_flushes <<< "$before"
"$binary" append "$mount_dir"
if [[ "$mode" == learner ]]; then
    grep -m 1 '^learner ftl    map LBA ' "$log" || fail 'The learner FTL did not report a page mapping.'
fi
saved=$(control checkpoint)
read -r _ saved_writes saved_flushes <<< "$saved"
(( saved_flushes > before_flushes && saved_writes > before_writes )) ||
    fail 'The application sync did not reach a new FTL checkpoint.'
grep -F "flush $saved_flushes published FTL checkpoint" "$log"
printf 'saved workload checkpoint at flush %s before unmount\n' "$saved_flushes"

"$binary" tail "$mount_dir"
umount "$mount_dir"
mount_absent "$mount_dir" || fail 'Could not confirm unmount before reopening the checkpoint.'
blockdev --flushbufs "$device"
later=$(control stats)
read -r before_reads _ later_flushes <<< "$later"
printf 'unmount and cache invalidation left the server at flush %s\n' "$later_flushes"
[[ $(control cut) == "$saved" ]] || fail 'The server did not reopen the captured checkpoint.'
mount -t ext4 -o ro "$device" "$mount_dir"
"$binary" verify "$mount_dir"
after=$(control stats)
read -r after_reads _ _ <<< "$after"
(( after_reads > before_reads )) || fail 'The remount did not issue reads to the reopened device.'
umount "$mount_dir"
