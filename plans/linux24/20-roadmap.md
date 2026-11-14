# 20 — 里程碑路线图 M0–M8

> 原则：每个里程碑结束必须"可启动、可演示、可回归"；gate 全绿才能进入下一个。
> 工期为单人投入的乐观估计（每周 10–15h），按 1.5 倍浮动规划。

## 依赖图

```
M0 ─► M1(xv6等价) ─► M2(内存) ─► M3(调度/内核基础) ─► M4(VFS/块层) ─► M5(ext4 RO)
                                                        │                │
                                                        └──(procfs/devfs)┴► M6(ext4 RW) ─► M7(jbd2-lite)
                                                                          │
M8(SMP/IPC/收尾) ◄───────────────────────────────────────────────────────┘（可与 M5–M7 部分并行）
```

## M0 — Rust 引导骨架 ✅（已完成 2026-11-14）

交付：multiboot 启动、VGA+COM1 双输出、multiboot 内存信息解析、panic handler、
Makefile(build/run/check)、workspace 与链接脚本定型。
Gate（已验）：make build 零警告；qemu 串口出现 `minios v0.1.0 (x86, rust)`、`boot ok`；
cargo fmt 通过；文件全部 ≤400 行。

## M1 — xv6 功能等价（Rust 版 xv6）｜🚧 核心已完成（2026-11-15）

交付（批次见 03-rust-rewrite-strategy.md）：
- 中断体系：GDT/IDT/TSS、8259、异常处理框架、PIT 100Hz、`jiffies`。
- 内存：entrypgdir→完整内核映射、kalloc 等价页分配器、用户地址空间 exec 装载。
- 进程：proc 表、上下文切换（swtch 等价）、fork/exec/exit/wait、sleep/wakeup、首个 shell。
- 系统调用：xv6 21 个全套；console/键盘中断输入；IDE PIO；xv6fs 过渡栈
  （fs/log/bio/file 翻译）；pipe。
- user/：xv6 C 用户程序全集（gcc -m32）+ usertests 移植通过。
Gate：`make run` 进入 sh；usertests 全绿；zombie/stressfs 通过；
新增 `scripts/run-qemu.sh` 自动回归；cargo test（host 单测：string/位图/ELF 解析）绿。

**2026-11-15 进度**：内核/用户态功能等价已落地 —— `make run` 进入 sh，
usertests 输出 `ALL TESTS PASSED`，zombie/stressfs 无 panic，debug/release 零警告。
剩余 gate：`scripts/run-qemu.sh`、host `cargo test`（详见 STATUS.md）。

## M2 — 内存管理现代化｜约 2–3 周

交付：bootmem（memblock 风格区间登记）→ buddy（order 0–10，每 CPU 单页缓存）；
slab 对象缓存（通用 + 专用 cache：inode/dentry/bh/bio）；vmalloc 区（4M 之上）；
percpu 区与每 CPU 变量宏；缺页处理完善 + COW fork；kmem 统计 /proc/meminfo 数据源。
Gate：buddy/slab host 单测（分配释放/碎片注入）；vmalloc 映射自检；
usertests 回归不退化；新增压力测试（并发 fork+exec 500 轮）。

## M3 — 调度与内核基础设施｜约 3 周

交付：waitqueue 体系；O(n) 调度器（goodness 简化：counter+priority，SCHED_OTHER/FIFO/RR，
nice ±19，时间片轮转）；内核线程框架（kthread_create/run），落地 ksoftirqd、bdflush、
kupdated（先空转打点）；softirq/tasklet（HI/普通两类）；jiffies 定时器轮（一层）；
信号核心集 + rt_sigreturn 框架；panic 栈回溯（frame pointer）。
Gate：调度公平性脚本（同优先级 N 进程 CPU 占比 ±5%）；FIFO 饿死 OTHER 的正例；
信号收发用例；定时器 10ms 精度验证；usertests 回归。

## M4 — VFS + 块层 + procfs/devfs｜约 3–4 周

交付：superblock/inode/dentry/file 四 ops trait；dcache（LRU+hash）与 icache；
page cache（地址空间模型，read/write/mmap 共享）；buffer_head（block<page）；
block_device_ops + 请求队列 + 电梯（排序+邻接合并）；cdev/bdev 注册与主次设备号；
procfs v1（/proc/version|cpuinfo|meminfo|uptime|pid/{status,cmdline}）；devfs-like /dev；
mount 表与 rootfs 挂载；xv6fs 改写为首个 VFS 客户端（回归载体）。
Gate：xv6fs 经 VFS 路径 usertests 全绿；page cache 命中率打印验证；
电梯排序 host 单测；/proc 内容快照测试；两个块设备（hd0/hd1）共存挂载。

## M5 — ext4 只读｜约 2–3 周（详见 30-ext4.md）

交付：ext4 super/GD/inode 解析、extent 树读取、目录线性+htree(dx_root) 读取、
filetype/symlink(fast/普通)、挂载选项（ro、bs=1k/2k/4k）、mke2fs -t ext4 镜像挂载。
Gate：host mke2fs 生成镜像（含大文件/深目录/多 group），qemu 挂载后
ls/cat/cp 全对（与 debugfs 导出清单 diff 为空）；read-only 强制写拒绝。

## M6 — ext4 读写（无日志）｜约 3–4 周

交付：块/Inode 位图分配（组内 first-fit→buddy 化）、extent 追加/分裂、目录插入与
htree 分裂（简化为 node 增长）、truncate、link/unlink/rename、空闲统计维护、
fsync 路径（无日志即直写）。host 侧 e2fsck -fn 全程零错误。
Gate：e2fsck 闭环（创建/删除/追加/截断/重命名 各 ≥1k 次后 e2fsck -fn 干净）；
并发写压力（多进程 fork 写文件）；与 debugfs 内容对账。

## M7 — jbd2-lite 日志｜约 3 周

交付：journal 区域布局（journal inode #8、描述/提交块）、事务 API（handle：start/stop）、
ordered 模式（数据先落，元数据入日志）、checkpoint、挂载回放、JBD2 特性位最低集；
崩溃注入框架（qemu NMI/kill 循环 + e2fsck 判定）。
Gate：崩溃循环 ≥200 次后 e2fsck -fn 干净（或仅 known-safe 提示）；
回放后 1s 内可再次读写；性能记录（小文件创建吞吐 vs M6）。

## M8 — SMP/IPC/2.4 parity 收尾｜约 3–4 周

交付：SMP 启动（AP trampoline）、每 CPU runqueue 数据（仍 O(n) 全局挑选 + 负载均衡简化）、
锁纪律审计（无锁竞争死锁检测脚本）、SysV sem/shm/msg + /proc/sysvipc、FIFO、
procfs 补全、（Stretch）socket/loopback/简化 TCP、（Stretch）xv6 式 BIOS 两阶段启动。
Gate：2–4 vCPU 全量回归（usertests+ext4 套件）；shm 双进程通信用例；
sem 竞争用例；00-scope In-Scope 清单逐项 ✅ 审计表（终验）。

## 里程碑与四任务的对应

任务1（无二进制历史）：M0 起持续政策 ✅；xv6 净化为可选附录。
任务2（2.4 状态）：M1–M4、M8 主体。
任务3（ext4）：M5–M7。
任务4（代码组织参考 Linux）：M1 起目录即按 02-architecture.md 演进，M4 完成全树收敛。
