# 00 — "Linux 2.4 状态"的精确定义（Scope）

> 对标本体是 Linux 2.4.37（2.4 系列最终版，2001–2009）的**架构与核心机制**，
> 不是复刻其 3.3MLOC 的驱动/网络/文件系统广度。逐子系统给出 In-Scope（必须）、
> Stretch（尽量）、Out-of-Scope（明确不做）。

## 1. 特性矩阵：xv6 / Linux 2.4 / minios 目标

| 子系统 | xv6 现状 | Linux 2.4 | minios 目标 | 级别 |
|--------|----------|-----------|-------------|------|
| CPU/SMP | mp 启动有但调度弱 | i386 SMP ≤32 CPU、IO-APIC | SMP ≤4 CPU（沿用 xv6 mp 语义） | In |
| 调度 | 单队列轮转 | O(n) goodness、SCHED_{OTHER,FIFO,RR} | O(n) 调度 + 3 策略 + nice | In |
| 内核线程 | 无 | keventd/kswapd/bdflush/kupdated | 内核线程框架 + kswapd/bdflush/kupdated | In |
| 抢占 | 无 | BKL；CONFIG_PREEMPT(后期) | 非抢占（BKL 时代语义） | In/ preempt Out |
| 软中断 | 无 | softirq + tasklet + BH | softirq/tasklet + 延迟处理 | In |
| 定时器 | 单 PIT 节拍 | 定时器轮 + jiffies | jiffies + 定时器轮（简化） | In |
| 物理内存 | kalloc 链表 4KB | bootmem→buddy、highmem/PAE | bootmem→buddy（order 0–10） | In；PAE Out |
| 小对象 | 无专用 | slab / kmem_cache | slab 风格对象缓存 | In |
| 内核虚存 | 固定映射 | vmalloc/percpu/kmap | vmalloc 区 + percpu 区 | In |
| 缺页/COW | 有限 | 完整 demand paging、COW | 匿名 demand paging + COW fork | In |
| 交换 | 无 | kswapd + swap | swap 为 Stretch；先做 OOM 路径 | Stretch |
| VFS | 无（fs.c 直连） | super/inode/dentry/file ops | 完整 VFS ops + dcache/icache | In |
| 页缓存 | 无 | page cache 统一 + LRU | page cache（地址空间模型）+ 简易 LRU | In |
| 缓冲层 | NBUF=10 数组 | buffer_head 挂 page | buffer_head（块<页时嵌页缓存） | In |
| 块设备层 | ide.c 同步轮询 | ll_rw_blk 请求队列+电梯 | 请求队列 + 合并/排序（单电梯） | In |
| 磁盘 FS | xv6fs(512B,log=10块) | ext2/ext3/reiserfs 等 | ext4（RO→RW→journal，用户指定） | In |
| 伪 FS | 无 | proc/devfs/tmpfs/ramfs | procfs + devfs-like；tmpfs Stretch | In |
| SysV IPC | 无 | sem/shm/msg | 三件套 + /proc/sysvipc | In |
| 管道/FIFO | pipe | pipe、FIFO、pty | pipe + FIFO；pty Out | In |
| 信号 | kill 仅终止 | POSIX 信号 + RT 队列 | 核心信号集 + 阻塞掩码（无 RT 队列） | In |
| 系统调用 | 21 个 | ~220 个 | ~55 个（对照表见 02 附录） | In |
| 设备模型 | dev[10] 表 | cdev/bdev、devfs、kmod | cdev/bdev 注册 + /dev 动态节点；动态模块 Out | In |
| tty | console 轮询 | 完整 tty 层 | console + 串口 tty 化（行规程极简） | In |
| 网络 | 无 | TCP/IP + netfilter | socket 层 + loopback + 简化 TCP | Stretch |
| SMP 锁 | spinlock | spin/rwlock/sem/BKL/percpu | spin/rwlock/mutex/sem + percpu + 大锁过渡 | In |
| procfs | 无 | 丰富 | /proc 精简集（cpuinfo/meminfo/uptime/pid/） | In |
| 启动 | BIOS 两阶段 | setup.S + vmlinux、initrd | multiboot 为主 + xv6 式两阶段为辅（ADR-0003） | In |
| 用户态 | C + ulib | glibc | M1 复用 xv6 C 用户程序；小 libc Stretch | In |

## 2. 明确 Out-of-Scope（防范围失控）

- 驱动广度（USB/SCSI/显卡/声卡实物网卡；仅 IDE-PIO + 可选 QEMU 虚拟网卡）
- netfilter/iptables、NFS、隧道、QoS
- 模块动态加载（kmod/insmod）、devtmpfs/udev
- PAE/highmem、NUMA、hugepage、mlock
- 多磁盘文件系统并存（ext4 为唯一磁盘 FS；xv6fs 仅 M1 过渡）
- 2.6 时代特性：O(1)/CFS、preempt、futex、epoll、NPTL、LSM、sysfs/kobject

## 3. 验收口径

"达到 Linux 2.4 状态" = 上表 In 级行全部 ✅ 且通过 40-testing-ci.md 对应验收命令；
Stretch 不阻塞。逐项 gate 见 20-roadmap.md。
