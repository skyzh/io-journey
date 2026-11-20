#!/usr/bin/env bash
set -euo pipefail

adapter_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $(id -u) == 0 && -c /dev/ublk-control ]] || fail 'Run as root in a guest with ublk.'
[[ $(readlink /proc/self/ns/mnt) != "$(readlink /proc/1/ns/mnt)" ]] ||
    fail 'Run in a private mount namespace using unshare --mount --propagation private.'
for ram_dir in /dev/shm /run; do
    [[ $(findmnt -n -o FSTYPE -T "$ram_dir") == tmpfs ]] || fail "$ram_dir must be tmpfs."
done

run_dir=$(mktemp -d /dev/shm/io-journey-ublk.XXXXXX)
mount_dir="$run_dir/mount"
socket="$run_dir/control.sock"
log="$run_dir/server.log"
binary="$run_dir/target/debug/io-journey-ublk"
server_pid=
device_id=
node_identity=
device=
mkdir "$mount_dir" "$run_dir/tmp"
export CARGO_HOME="$run_dir/cargo"
export CARGO_TARGET_DIR="$run_dir/target"
export TMPDIR="$run_dir/tmp"
export CARGO_INCREMENTAL=0

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
            wait "$server_pid" 2>/dev/null || true
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

cleanup() {
    local status=$? cleanup_failed=0 stop_status=0
    trap - EXIT
    set +e
    if mountpoint -q "$mount_dir"; then
        if [[ -n "$device" && $(findmnt -n -o SOURCE -M "$mount_dir") == "$device" ]]; then
            umount "$mount_dir" || cleanup_failed=1
        else
            cleanup_failed=1
        fi
    fi
    if [[ -n "$server_pid" ]] && ! mountpoint -q "$mount_dir"; then
        if [[ -z "$device_id" ]]; then
            read_allocation
        fi
        if ! server_exited; then
            timeout 7 "$binary" control "$socket" stop > "$run_dir/stop.log" 2>&1 || stop_status=$?
            if (( stop_status != 0 )); then
                printf 'socket stop failed. Reaping the owned server before deletion\n'
                if [[ $(readlink "/proc/$server_pid/exe") == "$binary" ]]; then
                    kill -TERM "$server_pid" || cleanup_failed=1
                fi
                if (( stop_status == 124 )); then cleanup_failed=1; fi
            fi
        fi
        if reap_server; then
            if device_remains; then
                if timeout 7 "$binary" remove-owned "$device_id" "$server_pid" "$node_identity" "$socket" >> "$run_dir/stop.log" 2>&1; then
                    printf 'deleted the run-owned ublk device after reaping its server\n'
                else
                    cleanup_failed=1
                fi
            fi
        else
            cleanup_failed=1
        fi
    fi
    if device_remains; then cleanup_failed=1; fi
    if mountpoint -q "$mount_dir"; then cleanup_failed=1; fi
    if [[ -n "$server_pid" && -d "/proc/$server_pid" ]]; then cleanup_failed=1; fi
    if (( status == 0 && cleanup_failed == 0 )); then
        printf 'verified cleanup of ublk device %s, metadata, mount, and server\n' "$device_id"
        rm -rf -- "$run_dir"
    else
        printf 'ublk smoke failed (cleanup status %s). Guest diagnostics at %s\n' "$cleanup_failed" "$run_dir" >&2
        if [[ -f "$log" ]]; then tail -n 20 "$log" >&2; fi
        if [[ -f "$run_dir/stop.log" ]]; then tail -n 10 "$run_dir/stop.log" >&2; fi
        if (( status == 0 )); then status=1; fi
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ -z "${LIBCLANG_PATH:-}" ]] && ! ldconfig -p | grep -F libclang >/dev/null; then
    [[ $(dpkg-query -W -f='${db:Status-Status}' libllvm21 2>/dev/null) == installed ]] ||
        fail 'Set LIBCLANG_PATH to an available libclang shared library directory.'
    mkdir "$run_dir/clang"
    chmod 0711 "$run_dir"
    install -d -m 0700 -o _apt "$run_dir/debs"
    (
        cd "$run_dir/debs"
        apt-get download libclang1-21
        for archive in libclang1-21_*.deb; do
            dpkg-deb -x "$archive" "$run_dir/clang"
        done
    )
    chmod 0700 "$run_dir"
    export LIBCLANG_PATH="$run_dir/clang/usr/lib/llvm-21/lib"
fi

cargo build --locked --quiet --manifest-path "$adapter_dir/Cargo.toml"
"$binary" serve "$socket" > "$log" 2>&1 &
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

mkfs.ext4 -q -F -b 4096 -m 0 -E nodiscard,lazy_itable_init=0,lazy_journal_init=0 "$device"
mount -t ext4 "$device" "$mount_dir"
before=$(control stats)
read -r _ before_writes before_flushes <<< "$before"
"$binary" append "$mount_dir"
saved=$(control checkpoint)
read -r _ saved_writes saved_flushes <<< "$saved"
(( saved_flushes > before_flushes && saved_writes > before_writes )) ||
    fail 'The application sync did not reach a new FTL checkpoint.'
grep -F "flush $saved_flushes published FTL checkpoint" "$log"
printf 'saved workload checkpoint at flush %s before unmount\n' "$saved_flushes"

"$binary" tail "$mount_dir"
umount "$mount_dir"
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
