# 21 — CFS / 线程 / 信号 / 软中断 / RCU（对标 Linux 2.6.39.4 kernel/）

## 1. Task 与 64 位上下文

- TaskStruct：状态、pid/tgid/tid（TID 在 M8 引入）、内核栈、mm、files/fs/sighand、
  调度实体、时间统计、FPU 区、信号域、per-cpu 归属。
- M1R 上下文：保存 64 位 callee-saved（rbx/rbp/r12–r15）、RSP/RIP、CR3；
  trap 使用 64 位 IDT gate 与 iretq；TSS 仅提供 RSP0/IST，不做硬件任务切换。
- **FPU/SSE 属 M1R gate**：x86_64 用户态基线含 SSE2。先 eager
  fxsave64/fxrstor64（512B、16B 对齐）；M3 可评估 CR0.TS + #NM 惰性优化，
  优化前后均须通过串扰测试。内核编译禁止自动生成 SSE/MMX。
- M1R fork/exec/exit/wait 复用 xv6 语义。
- **M8 Core 仅含 clone/TID/futex 线程基础**：冻结并测试当期支持的 clone flags、TID
  分配与回收、共享 mm/files 的基础行为及 futex wait/wake；完整 CLONE_THREAD 线程组
  语义、robust futex 和 glibc/NPTL 兼容不属于 M8，也不进入 Core gate。
- **M9+ External Parity** 按权威 ABI 清单关闭完整 CLONE_THREAD、线程组信号、线程
  exit 与 robust futex 差距，并用外部 glibc/NPTL 二进制做黑盒验证；RT 信号差距按 §6
  同期关闭。minios 自身测试与 runtime 仍使用 Rust。
- M8 每个 AP 除切换长模式外，必须加载 GDT/TSS/IDT、独立 idle/内核栈与
  per-cpu 数据，初始化 FPU/SSE，配置 syscall 所需 EFER/STAR/LSTAR/SFMASK MSR，
  并设置 IA32_GS_BASE/IA32_KERNEL_GS_BASE、验证 swapgs；不得只复制 BSP 的临时状态。

## 2. CFS（M3 UP，M8 SMP）

- 每任务 `sched_entity{vruntime, load_weight, exec_start, sum_exec}`；
  就绪实体放红黑树，最左节点为下一候选；周期更新 `min_vruntime`。
- nice −20…+19 映射权重；`delta_exec` 按权重折算 vruntime；wakeup/迁移放置
  需避免补偿异常；sched_yield、timeslice/最小粒度参数集中管理。
- SCHED_FIFO/RR 使用独立 RT 队列与优先级，先于 CFS 选择；Core 实现基础语义。
- M3 单 runqueue/单 CPU 验证算法；M8 改 per-cpu runqueue、负载均衡、唤醒迁移。
- 内核抢占：Core 先保持关闭并明确 `preempt_count`/临界区；Parity 开启完整
  CONFIG_PREEMPT 路径并审计 RCU/锁/中断返回点。不得宣称 Core 已有 Linux 2.6.39.4 全抢占。

## 3. 等待、内核线程与延迟工作

- waitqueue 支持可中断/不可中断、exclusive 唤醒与 ERESTARTSYS 语义。
- kthread 共享内核地址空间；workqueue 处理可睡眠延迟工作；softirq/tasklet
  处理中断后半部，ksoftirqd 在压力下接管。
- bdflush/kupdated 在 2.6 已演进为 writeback 模型：Core 提供 per-super 回写线程
  与周期 dirty 扫描，命名/机制以 23 文档为准，不复制早期 writeback 线程模型。

## 4. 时间子系统与 UP/SMP 平台边界

- **M1R/M3 仅使用 PIT + 8259 PIC**：M1R 以 PIT/PIC 提供 jiffies 与定时中断；
  M3 建 clocksource（TSC 校准）和 clockevents 抽象，UP 参考实现仍只用 PIT。
  LAPIC/IOAPIC/IPI 一律到 M8 接线，不得让 UP gate 隐式依赖 QEMU APIC 配置。
- M8 才启用 LAPIC 定时器、IOAPIC 外设路由与 IPI；PIT/PIC 保留早期启动和
  对照路径，切换后屏蔽 8259，避免重复中断。
- hrtimer 以单调时间为基准，支持绝对/相对到期与取消；jiffies timer wheel
  作为低精度兼容接口。alarm/gettimeofday/nanosleep 先接公共时间层。
- NO_HZ、完整高分辨率配置和 CPU 时间统计进入 Parity 差距清单。

## 5. RCU 与同步边界

- M3 UP：读侧临界区标记、quiescent state、callback 队列与 grace period 基础；
  先服务可替换指针/小型配置数据，再用于 dcache/路由等路径。
- M8 SMP：每 CPU quiescent 状态、callback 唤醒和 pressure 测试。
- spin/rwlock/mutex/seqlock/RCU 的选择写入锁层次；Parity 补 lockdep 类检查。

## 6. 信号与线程组

- M1R/M3：pending/blocked/handler、核心信号、SIGCHLD、异常信号；
  x86_64 rt_sigframe 保存 64 位寄存器与 FPU 状态，rt_sigreturn 完整恢复；这里的
  `rt_` 名称不表示实时（RT）信号队列已进入 Core。
- M8 Core：进程组/前台组，以及上述线程基础所需的核心信号与 futex 唤醒交互；不包含
  完整线程组信号、RT 信号队列或 NPTL 验收。
- M9+ External Parity：RT 信号排队/顺序与 siginfo、sigaltstack、SA_*、线程定向信号和
  完整 NPTL 用例；这些差距不作为 M8 Core gate。

## 7. 验收用例

1. M1R：双进程 SSE2 计算与 memcpy 交替不串值；64 位 trap/syscall 寄存器保持。
2. M8 对每个 AP 定向运行用户负载，逐一覆盖 trap、syscall/sysret、FPU/SSE
   保存恢复与 GS base；只在 BSP 通过不算 gate 通过。
3. CFS 的同权 CPU 时间差 **<5%** 只作为 host 确定性调度模型 gate；模型固定输入、
   时钟推进与迁移序列，同时检查 nice 权重比例和 RT 优先正例。
4. QEMU（尤其 TCG SMP）公平性使用 guest-time/调度计数和预先声明的容差，
   记录 vCPU 数与负载；禁止用 host wall-clock 套用 <5%，避免 TCG 抖动造成假绿/假红。
5. 定时器：sleep/nanosleep/alarm 精度矩阵；hrtimer 取消竞态压力。
6. RCU：读写并发与回收压力无 UAF/泄漏；SMP 下 grace period 可前进。
7. M8 Core：futex 等待唤醒、当期支持的 clone flags、TID 生命周期及共享 files/mm
   基础行为；不以 NPTL 程序或 RT 信号用例作为 gate。
8. M9+ External Parity：完整 NPTL 程序、robust futex、线程定向/线程组信号及 RT 信号
   排队、顺序与 siginfo 用例。
