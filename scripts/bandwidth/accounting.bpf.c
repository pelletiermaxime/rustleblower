/* Experimental syscall-payload accounting. See docs/bandwidth-plan.md. */
#pragma clang diagnostic push
/* bpftool emits anonymous kernel declarations that Clang 23 warns about. */
#pragma clang diagnostic ignored "-Wmissing-declarations"
#include "vmlinux.h"
#pragma clang diagnostic pop
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_tracing.h>
#include "accounting.h"

extern struct file *bpf_get_task_exe_file(struct task_struct *task) __ksym;
extern int bpf_path_d_path(const struct path *path, char *buf, size_t size) __ksym;
extern void bpf_put_file(struct file *file) __ksym;

struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct usage_config);
} rb_config SEC(".maps");

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, USAGE_MAX_PROCESSES);
    __type(key, struct usage_identity);
    __type(value, struct usage_bytes);
} rb_counts SEC(".maps");

/* Immutable metadata per process/execution, retained even after a quick exit.
 * Scratch storage avoids putting a 513-byte path on the BPF stack. */
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, USAGE_MAX_PROCESSES);
    __type(key, struct usage_identity);
    __type(value, struct usage_path);
} rb_paths SEC(".maps");
struct {
    __uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, struct usage_path);
} rb_path_scratch SEC(".maps");

struct call_key {
    __u64 tid;
    __u32 receive;
    __u32 padding;
};
struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, USAGE_MAX_INFLIGHT);
    __type(key, struct call_key);
    __type(value, __u32);
} rb_depth SEC(".maps");

/* Failed map insertions are explicit loss, never silently reported as zero. */
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, 1);
    __type(key, __u32);
    __type(value, __u64);
} rb_lost SEC(".maps");

/* Counts only calls passing the requested UID/PID filter, never other apps. */
struct {
    __uint(type, BPF_MAP_TYPE_ARRAY);
    __uint(max_entries, USAGE_DIAGNOSTICS);
    __type(key, __u32);
    __type(value, __u64);
} rb_diag SEC(".maps");

static __always_inline void diagnostic(__u32 field)
{
    __u64 *n = bpf_map_lookup_elem(&rb_diag, &field);
    if (n)
        __sync_fetch_and_add(n, 1);
}

static __always_inline bool selected(__u64 tid)
{
    __u32 zero = 0;
    struct usage_config *c = bpf_map_lookup_elem(&rb_config, &zero);
    if (!c || !c->enabled || (__u32)bpf_get_current_uid_gid() != c->uid)
        return false;
    if (!c->pid_count)
        return true;
    for (int i = 0; i < USAGE_MAX_PIDS; ++i) {
        if (i >= c->pid_count)
            break;
        if (c->pids[i] == tid >> 32)
            return true;
    }
    return false;
}

static __always_inline void loss(void)
{
    __u32 zero = 0;
    __u64 *n = bpf_map_lookup_elem(&rb_lost, &zero);
    if (n)
        __sync_fetch_and_add(n, 1);
}

static __always_inline void identify(struct task_struct *task, struct file *exe,
                                    struct usage_identity *key)
{
    key->pid = bpf_get_current_pid_tgid() >> 32;
    key->uid = (__u32)bpf_get_current_uid_gid();
    key->start_ns = BPF_CORE_READ(task, group_leader, start_boottime);
    key->exec_id = BPF_CORE_READ(task, group_leader, self_exec_id);
    key->inode = BPF_CORE_READ(exe, f_inode, i_ino);
    key->device = BPF_CORE_READ(exe, f_inode, i_sb, s_dev);
    struct vfsmount *vfs_mount = BPF_CORE_READ(exe, f_path.mnt);
    struct mount *mount = (void *)vfs_mount - bpf_core_field_offset(struct mount, mnt);
    key->mount_id = BPF_CORE_READ(mount, mnt_id);
    BPF_CORE_READ_STR_INTO(&key->comm, task, group_leader, comm);
}

static __always_inline int capture_path(struct socket *sock, int previous)
{
    /* Pure observation: preserve any prior LSM denial and never deny a call. */
    if (previous || !selected(bpf_get_current_pid_tgid()))
        return previous;
    unsigned short family = BPF_CORE_READ(sock, sk, __sk_common.skc_family);
    if (family != 2 && family != 10) /* AF_INET / AF_INET6; exclude local IPC */
        return previous;
    struct task_struct *task = bpf_get_current_task_btf();
    struct file *exe = bpf_get_task_exe_file(task);
    if (!exe)
        return previous;
    struct usage_identity key = {};
    identify(task, exe, &key);
    if (key.start_ns && key.inode && key.mount_id &&
        !bpf_map_lookup_elem(&rb_paths, &key)) {
        __u32 zero = 0;
        struct usage_path *path = bpf_map_lookup_elem(&rb_path_scratch, &zero);
        if (path) {
            path->length = bpf_path_d_path(&exe->f_path, path->value, sizeof(path->value));
            bpf_map_update_elem(&rb_paths, &key, path, BPF_NOEXIST);
        }
    }
    bpf_put_file(exe);
    return previous;
}

SEC("lsm/socket_sendmsg")
int BPF_PROG(capture_send_path, struct socket *sock, struct msghdr *msg, int size, int ret)
{
    return capture_path(sock, ret);
}
SEC("lsm/socket_recvmsg")
int BPF_PROG(capture_receive_path, struct socket *sock, struct msghdr *msg, int size, int flags, int ret)
{
    return capture_path(sock, ret);
}

static __always_inline int enter(__u32 receive)
{
    __u64 tid = bpf_get_current_pid_tgid();
    if (!selected(tid))
        return 0;
    diagnostic(USAGE_ENTRIES);
    struct call_key key = {.tid = tid, .receive = receive};
    __u32 *n = bpf_map_lookup_elem(&rb_depth, &key);
    if (n) {
        ++*n;
    } else {
        __u32 one = 1;
        if (bpf_map_update_elem(&rb_depth, &key, &one, BPF_NOEXIST))
            loss();
    }
    return 0;
}

static __always_inline int leave(__u32 receive, __u64 requested, int flags, int result)
{
    struct call_key call = {.tid = bpf_get_current_pid_tgid(), .receive = receive};
    if (!selected(call.tid))
        return 0;
    diagnostic(USAGE_RETURNS);
    __u32 *n = bpf_map_lookup_elem(&rb_depth, &call);
    if (!n) {
        diagnostic(USAGE_MISSING_INFLIGHT);
        return 0;
    }
    if (*n > 1) {
        --*n;
        return 0;
    }
    bpf_map_delete_elem(&rb_depth, &call);
    /* MSG_PEEK=2; datagram MSG_TRUNC may return more than was copied. */
    if (result <= 0 || (receive && (flags & 2)))
        return 0;
    diagnostic(USAGE_POSITIVE_RETURNS);
    __u64 count = (__u64)result;
    if (count > requested)
        count = requested;
    struct task_struct *task = (void *)bpf_get_current_task_btf();
    struct usage_identity key = {};
    identify(task, BPF_CORE_READ(task, mm, exe_file), &key);
    if (!key.start_ns || !key.inode || !key.mount_id) {
        loss();
        return 0;
    }
    struct usage_bytes *value = bpf_map_lookup_elem(&rb_counts, &key);
    if (!value) {
        struct usage_bytes empty = {};
        bpf_map_update_elem(&rb_counts, &key, &empty, BPF_NOEXIST);
        value = bpf_map_lookup_elem(&rb_counts, &key);
    }
    if (!value) {
        loss();
        return 0;
    }
    if (receive)
        __sync_fetch_and_add(&value->received, count);
    else
        __sync_fetch_and_add(&value->sent, count);
    diagnostic(USAGE_ACCOUNTED_CALLS);
    return 0;
}

/* The outermost return owns accounting. In particular, an IPv4-mapped IPv6
 * UDP send may call udp_sendmsg from udpv6_sendmsg; count that result once. */
#define SEND_HOOK(name) \
    SEC("fentry/" #name) \
    int BPF_PROG(begin_##name, struct sock *sk, struct msghdr *msg, size_t len) \
    { return enter(0); } \
    SEC("fexit/" #name) \
    int BPF_PROG(end_##name, struct sock *sk, struct msghdr *msg, size_t len, int result) \
    { return leave(0, len, 0, result); }

#define RECV_HOOK(name) \
    SEC("fentry/" #name) \
    int BPF_PROG(begin_##name, struct sock *sk, struct msghdr *msg, size_t len, int flags) \
    { return enter(1); } \
    SEC("fexit/" #name) \
    int BPF_PROG(end_##name, struct sock *sk, struct msghdr *msg, size_t len, int flags, int result) \
    { return leave(1, len, flags, result); }

/* Optimized kernels can inline tcp_sendmsg into inet_sendmsg. Its locked
 * implementation is also reached by that direct path; hook it once. */
SEND_HOOK(tcp_sendmsg_locked)
RECV_HOOK(tcp_recvmsg)
SEND_HOOK(udp_sendmsg)
RECV_HOOK(udp_recvmsg)
SEND_HOOK(udpv6_sendmsg)
RECV_HOOK(udpv6_recvmsg)

char LICENSE[] SEC("license") = "Dual MIT/GPL";
