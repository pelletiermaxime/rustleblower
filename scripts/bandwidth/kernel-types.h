/* Minimal CO-RE declarations, not a kernel ABI or a generated build-host header.
 * libbpf relocates every accessed field against the running kernel's BTF.
 * Missing hooks, fields or kfuncs make loading fail explicitly. */
#ifndef __VMLINUX_H__
#define __VMLINUX_H__
typedef unsigned char __u8;
typedef signed char __s8;
typedef unsigned short __u16;
typedef signed short __s16;
typedef unsigned int __u32;
typedef signed int __s32;
typedef unsigned long long __u64;
typedef signed long long __s64;
typedef __u16 __be16;
typedef __u32 __be32;
typedef __u32 __wsum;
typedef unsigned long size_t;
typedef _Bool bool;
#define true 1
#define false 0
enum { BPF_MAP_TYPE_HASH = 1, BPF_MAP_TYPE_ARRAY = 2,
       BPF_MAP_TYPE_PERCPU_ARRAY = 6, BPF_NOEXIST = 1 };
#pragma clang attribute push (__attribute__((preserve_access_index)), apply_to = record)
struct dentry;
struct msghdr;
struct super_block { __u32 s_dev; };
struct inode { struct super_block *i_sb; __u64 i_ino; };
struct vfsmount { struct dentry *mnt_root; };
struct path { struct vfsmount *mnt; struct dentry *dentry; };
struct file { struct path f_path; struct inode *f_inode; };
struct mm_struct { struct file *exe_file; };
struct task_struct {
    struct mm_struct *mm;
    struct task_struct *group_leader;
    __u64 start_boottime;
    char comm[16];
    __u64 self_exec_id;
};
struct mount { struct vfsmount mnt; int mnt_id; };
struct sock_common { unsigned short skc_family; };
struct sock { struct sock_common __sk_common; };
struct socket { struct sock *sk; };
#pragma clang attribute pop
#endif
