#define _GNU_SOURCE
#include <linux/types.h>
#include <bpf/bpf.h>
#include <bpf/libbpf.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <limits.h>
#include <signal.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>
#include "accounting.h"

#define MAX_SNAPSHOT (2 * 1024 * 1024)
static volatile sig_atomic_t stopped;
static void stop(int signal_number) { (void)signal_number; stopped = 1; }

static uint64_t monotonic_ms(void)
{
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) {
        perror("clock_gettime");
        exit(1);
    }
    return (uint64_t)t.tv_sec * 1000 + (uint64_t)t.tv_nsec / 1000000;
}

static void hex(char *out, const unsigned char *in, size_t length)
{
    const char *digits = "0123456789abcdef";
    for (size_t i = 0; i < length; ++i) {
        out[2 * i] = digits[in[i] >> 4];
        out[2 * i + 1] = digits[in[i] & 15];
    }
    out[2 * length] = 0;
}

/* The map key binds this path to one kernel-observed process execution.
 * Never resolve an old record using a potentially recycled /proc entry. */
static bool captured_executable(const struct usage_path *path, char *encoded)
{
    if (path->length <= 1 || path->length > USAGE_PATH_SIZE || path->value[0] != '/' ||
        strnlen(path->value, sizeof(path->value)) != (size_t)path->length - 1)
        return false;
    hex(encoded, (const unsigned char *)path->value, (size_t)path->length - 1);
    return true;
}

static bool snapshot(int counters, int paths, int losses, int diagnostics, uint64_t started, uint64_t sample)
{
    struct usage_identity key, next;
    struct usage_bytes bytes;
    uint32_t zero = 0;
    uint64_t lost = 0;
    if (bpf_map_lookup_elem(losses, &zero, &lost))
        return false;
    uint64_t diagnostic[USAGE_DIAGNOSTICS] = {};
    for (uint32_t field = 0; field < USAGE_DIAGNOSTICS; ++field) {
        if (bpf_map_lookup_elem(diagnostics, &field, &diagnostic[field]))
            return false;
    }
    /* Complete bounded JSON lines, no packet data and no unescaped OS strings. */
    size_t size = 256, rows = 0;
    bool truncated = false;
    printf("{\"version\":1,\"started_ms\":%llu,\"sample_ms\":%llu,\"rows\":[",
        (unsigned long long)started, (unsigned long long)sample);
    void *previous = NULL;
    while (bpf_map_get_next_key(counters, previous, &next) == 0) {
        key = next;
        previous = &key;
        if (rows >= USAGE_MAX_PROCESSES) {
            truncated = true;
            break;
        }
        if (bpf_map_lookup_elem(counters, &key, &bytes)) {
            fprintf(stderr, "Cannot read counters: %s\n", strerror(errno));
            return false;
        }
        char path[1025], comm[33], row[1600];
        struct usage_path captured;
        bool retained = bpf_map_lookup_elem(paths, &key, &captured) == 0 && captured_executable(&captured, path);
        const char *reason = retained ? NULL : "kernel-executable-path-unavailable";
        bool known = reason == NULL;
        hex(comm, (unsigned char *)key.comm, strnlen(key.comm, sizeof(key.comm)));
        int length = snprintf(row, sizeof(row),
            "%s{\"pid\":%u,\"uid\":%u,\"start_ns\":%llu,\"inode\":%llu,"
            "\"device\":%u,\"mount_id\":%u,\"exec_id\":%llu,\"comm_hex\":\"%s\",\"executable_hex\":%s%s%s,"
            "\"executable_source\":%s,\"executable_unavailable_reason\":%s%s%s,"
            "\"sent\":%llu,\"received\":%llu}",
            rows ? "," : "", key.pid, key.uid,
            (unsigned long long)key.start_ns, (unsigned long long)key.inode,
            key.device, key.mount_id, (unsigned long long)key.exec_id, comm, known ? "\"" : "", known ? path : "null", known ? "\"" : "",
            known ? "\"kernel\"" : "null",
            known ? "" : "\"", known ? "null" : reason, known ? "" : "\"",
            (unsigned long long)bytes.sent, (unsigned long long)bytes.received);
        if (length < 0 || (size_t)length >= sizeof(row) || size + (size_t)length > MAX_SNAPSHOT - 256) {
            truncated = true;
            break;
        }
        fwrite(row, 1, (size_t)length, stdout);
        size += (size_t)length;
        ++rows;
    }
    if (errno != ENOENT && !truncated) {
        fprintf(stderr, "Cannot enumerate counters: %s\n", strerror(errno));
        return false;
    }
    printf("],\"lost_operations\":%llu,\"truncated\":%s,\"diagnostics\":{"
        "\"entries\":%llu,\"returns\":%llu,\"missing_inflight\":%llu,"
        "\"positive_returns\":%llu,\"accounted_calls\":%llu}}\n",
        (unsigned long long)lost, truncated ? "true" : "false",
        (unsigned long long)diagnostic[USAGE_ENTRIES],
        (unsigned long long)diagnostic[USAGE_RETURNS],
        (unsigned long long)diagnostic[USAGE_MISSING_INFLIGHT],
        (unsigned long long)diagnostic[USAGE_POSITIVE_RETURNS],
        (unsigned long long)diagnostic[USAGE_ACCOUNTED_CALLS]);
    return fflush(stdout) == 0;
}

static unsigned number(const char *value, unsigned min, unsigned max)
{
    char *end;
    errno = 0;
    unsigned long n = strtoul(value, &end, 10);
    if (errno || *end || !*value || *value == '-' || n < min || n > max) {
        fprintf(stderr, "Invalid numeric argument: %s\n", value);
        exit(2);
    }
    return (unsigned)n;
}

static bool connect_output(const char *path, unsigned uid)
{
    struct sockaddr_un address = {.sun_family = AF_UNIX};
    if (path[0] != '/' || strlen(path) >= sizeof(address.sun_path)) {
        fprintf(stderr, "Collector receiver needs a short absolute Unix socket path\n");
        return false;
    }
    memcpy(address.sun_path, path, strlen(path) + 1);
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0)
        return false;
    bool success = false;
    struct ucred peer;
    socklen_t length = sizeof(peer);
    struct timeval timeout = {.tv_sec = 2};
    if (connect(fd, (struct sockaddr *)&address, sizeof(address)) ||
        getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &length)) {
        fprintf(stderr, "Cannot connect usage receiver: %s\n", strerror(errno));
    } else if (length != sizeof(peer) || peer.uid != uid) {
        fprintf(stderr, "Usage receiver does not belong to the selected UID\n");
    } else if (setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) || dup2(fd, STDOUT_FILENO) < 0) {
        fprintf(stderr, "Cannot prepare usage stream: %s\n", strerror(errno));
    } else {
        success = true;
    }
    close(fd);
    return success;
}

int main(int argc, char **argv)
{
    const char *object = NULL;
    const char *receiver = NULL;
    bool have_uid = false;
    unsigned seconds = 30, interval = 1000;
    struct usage_config config = {};
    static const struct option options[] = {
        {"help", no_argument, NULL, 'h'},
        {"object", required_argument, NULL, 'o'},
        {"uid", required_argument, NULL, 'u'},
        {"pid", required_argument, NULL, 'p'},
        {"seconds", required_argument, NULL, 's'},
        {"interval-ms", required_argument, NULL, 'i'},
        {"connect", required_argument, NULL, 'c'},
        {NULL, 0, NULL, 0}
    };
    int option;
    while ((option = getopt_long(argc, argv, "", options, NULL)) != -1) {
        switch (option) {
        case 'h':
            puts("Usage: collector --object accounting.bpf.o --uid UID [--pid PID ...] [--seconds 1..3600] [--interval-ms 100..5000] [--connect /path/to/receiver.sock]");
            return 0;
        case 'o': object = optarg; break;
        case 'u': config.uid = number(optarg, 0, UINT_MAX - 1); have_uid = true; break;
        case 'p':
            if (config.pid_count >= USAGE_MAX_PIDS) {
                fprintf(stderr, "At most %d owned PID filters are supported\n", USAGE_MAX_PIDS);
                return 2;
            }
            config.pids[config.pid_count++] = number(optarg, 1, INT_MAX);
            break;
        case 's': seconds = number(optarg, 1, 3600); break;
        case 'i': interval = number(optarg, 100, 5000); break;
        case 'c': receiver = optarg; break;
        default: return 2;
        }
    }
    if (!object || !have_uid || optind != argc) {
        fprintf(stderr, "Usage: collector --object accounting.bpf.o --uid UID "
            "[--pid PID ...] [--seconds 1..3600] [--interval-ms 100..5000] [--connect /path/to/receiver.sock]\n");
        return 2;
    }
    signal(SIGINT, stop);
    signal(SIGTERM, stop);
    signal(SIGPIPE, SIG_IGN);
    if (receiver && !connect_output(receiver, config.uid))
        return 1;
    struct bpf_object *obj = bpf_object__open_file(object, NULL);
    if (!obj || libbpf_get_error(obj)) {
        fprintf(stderr, "Cannot open BPF object\n");
        return 1;
    }
    struct bpf_link *links[USAGE_PROGRAMS] = {};
    size_t linked = 0;
    int code = 1;
    if (bpf_object__load(obj)) {
        fprintf(stderr, "Collector unavailable: BPF load failed. Requires privileged "
            "BPF access, compatible BTF and tracing support. No zero-traffic snapshot emitted.\n");
        goto cleanup;
    }
    struct bpf_program *program;
    bpf_object__for_each_program(program, obj) {
        if (linked >= USAGE_PROGRAMS)
            goto cleanup;
        struct bpf_link *link = bpf_program__attach(program);
        if (!link || libbpf_get_error(link)) {
            fprintf(stderr, "Cannot attach %s; all owned probes will detach\n", bpf_program__name(program));
            goto cleanup;
        }
        links[linked++] = link;
    }
    if (linked != USAGE_PROGRAMS)
        goto cleanup;
    int counters = bpf_object__find_map_fd_by_name(obj, "rb_counts");
    int paths = bpf_object__find_map_fd_by_name(obj, "rb_paths");
    int losses = bpf_object__find_map_fd_by_name(obj, "rb_lost");
    int configuration = bpf_object__find_map_fd_by_name(obj, "rb_config");
    int diagnostics = bpf_object__find_map_fd_by_name(obj, "rb_diag");
    uint32_t zero = 0;
    config.enabled = 1;
    uint64_t started = monotonic_ms();
    if (bpf_map_update_elem(configuration, &zero, &config, BPF_ANY))
        goto cleanup;
    uint64_t end = started + (uint64_t)seconds * 1000;
    do {
        if (!snapshot(counters, paths, losses, diagnostics, started, monotonic_ms()))
            goto cleanup;
        uint64_t now = monotonic_ms();
        if (now >= end)
            break;
        uint64_t delay = end - now < interval ? end - now : interval;
        struct timespec pause = {.tv_sec = delay / 1000, .tv_nsec = (delay % 1000) * 1000000};
        while (nanosleep(&pause, &pause) && errno == EINTR && !stopped) {}
    } while (!stopped);
    code = 0;
cleanup:
    for (size_t i = 0; i < linked; ++i)
        bpf_link__destroy(links[i]);
    bpf_object__close(obj);
    return code;
}
