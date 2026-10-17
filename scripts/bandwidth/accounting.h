/* Shared ABI for the isolated measurement prototype. No packet payloads. */
#ifndef RUSTLEBLOWER_ACCOUNTING_H
#define RUSTLEBLOWER_ACCOUNTING_H
#define USAGE_MAX_PROCESSES 4096
#define USAGE_MAX_INFLIGHT 8192
#define USAGE_PATH_SIZE 513
#define USAGE_PROGRAMS 14
#define USAGE_MAX_PIDS 8

enum usage_diagnostic {
    USAGE_ENTRIES,
    USAGE_RETURNS,
    USAGE_MISSING_INFLIGHT,
    USAGE_POSITIVE_RETURNS,
    USAGE_ACCOUNTED_CALLS,
    USAGE_DIAGNOSTICS
};

struct usage_config {
    __u32 enabled;
    __u32 uid;
    __u32 pid_count; /* zero selects all processes of uid */
    __u32 pids[USAGE_MAX_PIDS];
};
struct usage_identity {
    __u64 start_ns;
    __u64 inode;
    __u64 exec_id;
    __u32 device;
    __u32 pid;
    __u32 uid;
    __u32 mount_id;
    char comm[16];
};
struct usage_bytes {
    __u64 sent;
    __u64 received;
};
struct usage_path {
    int length; /* includes trailing NUL; negative means capture unavailable */
    char value[USAGE_PATH_SIZE];
};
#endif
