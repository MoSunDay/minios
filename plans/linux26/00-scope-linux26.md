# 00 — Linux 2.6.39.4 / x86_64 目标定义（Scope）

> 对标本体：**Linux 2.6.39.4（2.6 系列最终稳定版）/ x86_64**。x86_64、NPTL、
> CFS、epoll、sysfs 与 ext4 均落在该基线内；本目录是唯一有效的演进计划。
> 内核、用户态、ulib/runtime、工具与仓库内测试全部用 Rust 编写；C 仅存在于
> 只读 xv6 参考和外部验证负载，不参与 minios 构建（ADR-0013）。

## 1. 完成口径：Core 不等于 1:1

| 口径 | 里程碑 | 验收含义 |
|------|--------|----------|
| Core-2.6 | M0R–M8 | 下表 Core 项全绿；核心机制可演示、可回归；不是完整 Linux ABI |
| Parity-2.6 | M9+ | 2.6.39.4 x86_64 syscall/ABI 与约定子系统差距关闭；LTP + 发行版 rootfs 验收 |
| 全主线 1:1 | 未立项 | 全部配置/驱动/文件系统的源码级广度；当前计划不承诺，需另做资源评估 |

“达到 Linux 2.6.39.4 状态”在 M8 只能写作 **Core-2.6 完成**。只有 Parity gate 全绿后，
才可对约定范围声明 parity；不得把 syscall 编号对齐等同于 ABI 兼容。

## 2. 特性矩阵

| 子系统 | Linux 2.6.39.4 / x86_64 参照 | Core-2.6（M0R–M8） | Parity 后续 |
|--------|------------------------------|--------------------|-------------|
| 架构 | 长模式、四级页表、ELF64、syscall/sysret | 全部基础路径 | 完整异常/ABI/vDSO/vsyscall 细节 |
| SMP | per-cpu runqueue、APIC、热插拔等 | ≤4 vCPU、APIC、TLB shootdown | 更多 CPU/拓扑/热插拔 |
| 调度 | CFS、RT 类、组调度、内核抢占 | CFS + FIFO/RR + 非抢占先行 | 组调度、完整抢占与负载均衡 |
| 线程 | clone 全标志、TID、futex、NPTL | clone 最小集 + futex 基础 | NPTL 全语义、robust futex；外部 glibc 黑盒验证 |
| 信号 | RT 信号、siginfo、sigaltstack | 核心信号 + 64 位 sigframe | RT 队列与完整语义 |
| 定时器 | clocksource/clockevents/hrtimer/NO_HZ | jiffies + timer wheel + hrtimer 基础 | 高精度/NO_HZ 全路径 |
| 物理内存 | zones、buddy、SLAB/SLUB、percpu | buddy + SLUB 风格 + percpu | NUMA/热插拔/hugepage |
| 用户虚存 | VMA、COW、mmap、mlock、swap、OOM | M2 匿名 VMA/brk/匿名 mmap/COW/OOM；M4 file-backed mmap；无 swap | 完整 VMA/madvise/mlock/swap |
| 内核虚存 | vmalloc、vmap、固定映射、percpu | vmalloc + 直映 + GS-base percpu | 完整 vmap/调试设施 |
| 同步 | spin/rwlock/mutex/seqlock/RCU/lockdep | 锁层次 + RCU 基础 + 断言 | preempt/lockdep/RCU 全语义 |
| VFS | super/inode/dentry/file、RCU walk | M4 四 ops + dcache/icache | RCU path walk/完整 mount 语义 |
| 文件 IO/通知 | 原生 AIO、零拷贝、文件通知 | 仅既有 read/write/mmap/poll；不加入本行 Parity API | Parity M10：原生 Linux AIO（io_*）、sendfile/splice/tee/vmsplice、inotify/fanotify/dnotify（F_NOTIFY） |
| 页缓存 | address_space、readahead、回写 | M4 page cache + LRU + 干净/脏页回收与回写 | readahead/完整 writeback |
| 块层 | BIO、request、CFQ/deadline/noop | BIO + 队列 + 电梯基础；M4/M6 基础 CACHE FLUSH | Parity M10：全电梯、完整 barrier/FUA/统计 |
| 磁盘 FS | ext2/ext3/ext4 等多 FS | ext4 RO→RW→jbd2；xv6fs 仅过渡 | 其余 FS 按 parity 范围另列 |
| 伪 FS | proc/sysfs/tmpfs/ramfs/devtmpfs | procfs + sysfs 基础 + 最小 tmpfs | 完整属性/设备语义 |
| IPC | SysV/POSIX IPC、pipe、eventfd 等 | SysV 三件套 + pipe/FIFO | POSIX mqueue、eventfd/signalfd/timerfd |
| 事件等待 | select/poll/epoll | M8 select/poll + 基础 epoll | 完整 epoll 边缘触发与并发言义 |
| 设备模型 | kobject/sysfs、模块、PCI/ACPI | cdev/bdev + sysfs 基础；模块后置 | kmod/initramfs/PCI/ACPI/热插拔 |
| tty | termios、pty、行规程、job control | M4 最小行规程；M8 前台组信号基础 | 完整 termios/pty/job control |
| 网络 | IPv4/IPv6、TCP/UDP、netfilter、netlink | Stretch：loopback/UDP/最小 TCP；不进 M8 gate | Parity 必须：完整约定网络栈 |
| 安全 | credentials、capabilities、LSM、审计接口 | 单用户；最小凭据/capability 检查 + mode/caps 基础 | Parity M12：完整 credentials/capabilities 算法、LSM、审计 |
| 容器 | namespaces、cgroups | 不做，留接口边界 | Parity M12：ns/cgroup 约定集 |
| 可观测性 | oops、ftrace、perf、kprobes | printk/oops/栈回溯 | ftrace/perf/kprobes 约定集 |
| 系统调用 | 以 2.6.39.4 syscall_64.tbl 为准 | 白名单核心子集 | 从权威表生成差距清单并逐项关闭 |

安全边界固定为：Core 只有单用户的最小凭据表示、DAC mode 与少量 capability 检查；
完整 credentials 生命周期/变更语义、capabilities 算法和 LSM 均在 Parity M12。
eventfd/signalfd/timerfd 也始终是 Parity 项，不得借基础 epoll 带入 Core。

## 3. 明确不做或另立项

- i686/x86 32 位内核、ELF32 用户态、ia32 compat ABI。
- 在 minios 实现树中新增 C 源文件或以 gcc/clang 构建 minios 内核、用户程序、ulib、
  工具；外部参考/测试负载不属于此例。
- 非 x86_64 架构、真实硬件认证、私有驱动。
- Core 阶段不承诺完整 Linux 用户态兼容；Parity 以真实用户态测试为准。
- “全主线 1:1”未立项前，不把 USB/SCSI/全部网卡/全部 FS/全部配置写成已交付。
- bug-for-bug 兼容仅在影响公开 ABI 或文件系统一致性时纳入，不复制内部实现细节。

## 4. 验收原则

1. 每个里程碑可启动、可演示、可回归，gate 未绿不得进入下一阶段。
2. ext4 以 e2fsprogs 为权威外部校验；崩溃测试不得以“能再次启动”代替 e2fsck。
3. Parity 以未修改的 x86_64 Linux 用户态、LTP 和 ABI 差分测试为证据。
4. 所有范围降级必须记录 ADR，并同步修改 STATUS 的完成口径。
