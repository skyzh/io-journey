run_queue_case() {
    local policy=$1 depth=$2 size=$3 output=$4
    local major minor dev_number event
    udevadm info --wait-for-initialization=10 --query=name --name="$device" >/dev/null
    udevadm settle --timeout=10

    IFS=: read -r major minor < "$sys_block/dev"
    dev_number=$((major * (1 << 20) + minor))
    create_trace_instance "/sys/kernel/tracing/instances/${run_dir##*/}"
    printf '0\n' > "$trace_instance/tracing_on"
    printf 'mono\n' > "$trace_instance/trace_clock"
    printf '2048\n' > "$trace_instance/buffer_size_kb"
    for event in block_rq_issue block_rq_complete; do
        printf 'dev == %s\n' "$dev_number" > "$trace_instance/events/block/$event/filter"
        printf '1\n' > "$trace_instance/events/block/$event/enable"
    done
    printf '%s\t%s\t%s\t%s,%s\n' "$policy" "$depth" "$size" "$major" "$minor" > "$output/case.tsv"

    udevadm lock --device="$device" --timeout=10 -- bash -euo pipefail -c '
        source "$1"
        shift
        capture_queue_case "$@"
    ' queue-case "$adapter_dir/queue-case.sh" "$binary" "$socket" \
        "$device" "$depth" "$size" "$output" "$trace_instance"
    cat "$trace_instance/trace" > "$output/kernel.trace"
    cat "$trace_instance"/per_cpu/cpu*/stats > "$output/trace-stats.txt"
    rmdir "$trace_instance"
    [[ ! -e "$trace_instance" ]] || fail 'Trace instance survived the run.'
    trace_instance=
}

create_trace_instance() {
    mkdir -- "$1" || return
    trace_instance=$1
}

capture_queue_case() {
    local binary=$1 socket=$2 device=$3 depth=$4 size=$5 output=$6 instance=$7
    local -a fio_args=(
        --name=io-journey --filename="$device" --allow_file_create=0
        --ioengine=io_uring --direct=1 --invalidate=0 --rw=randrw --rwmixread=70
        --size=16M --bs="$size" --iodepth="$depth" --iodepth_batch_submit="$depth"
        --iodepth_batch_complete_max="$depth" --randrepeat=1 --randseed=20261120
        --end_fsync=0 --lat_percentiles=1 --eta=never --output-format=json
    )
    timeout 30 fio "${fio_args[@]}" --io_size=2M --output="$output/warm.json"
    "$binary" control "$socket" profile-start >/dev/null
    printf '1\n' > "$instance/tracing_on"
    timeout 30 fio "${fio_args[@]}" --io_size=16M --output="$output/fio.json"
    printf '0\n' > "$instance/tracing_on"
    "$binary" control "$socket" profile-save > "$output/profile-rows.txt"
}
