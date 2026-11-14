# 21 — 进程 / 调度 / 信号 / 软中断 / 内核线程（对标 Linux 2.4 kernel/）

## 1. 进程模型（kernel/sched/）

- `TaskStruct`（对应 task_struct，M1 从 xv6 proc 移植，M3 扩展）：
  state(RUNNABLE/RUNNING/SLEEPING/STOPPED/ZOMBIE/FREE)、pid/ppid、内核栈（KSTACKSIZE 4K）、
  上下文（callee-saved + cr3 + eip）、打开文件表、信号域（M3）、地址空间 mm、
  counter/policy/nice/rt_priority、时间统计（utime/stime/starttime）。
- 状态机与 Linux 2.4 对齐：TASK_{RUNNABLE,RUNNING,INTERRUPTIBLE,UNINTERRUPTIBLE,STOPPED,ZOMBIE}；
  sleep 分可中断/不可中断（信号唤醒 vs 仅 wakeup）。
- 生命周期：fork.rs（复制地址空间→M2 起改 COW）、exec.rs（ELF 装载）、exit.rs（资源回收 +
  父通知 CHLD）、wait.rs（收割）、孤儿过继（M3，对应 reparent_to_init）。
- 线程（kernel clone 语义）为 Stretch：clone(CLONE_VM|CLONE_THREAD) 最小集在 M8 评估。

## 2. 调度器（2.4 O(n) 语义）

- 就绪队列：全局 runqueue + 自旋锁（2.4 即全局 runqueue；per-CPU 仅数据分离，选择仍全局扫描）。
- `schedule()`：遍历任务数组计算 goodness：
  - SCHED_FIFO/RR：rt_priority 优先，RR 耗尽时间片重新入队，FIFO 不抢占同优先级；
  - SCHED_OTHER：counter + priority 亲和微加成（2.4 语义保留但简化 CPU 亲和为固定掩码）。
- 时间片：counter 初值 = priority 基准（HZ=100，时间片 10–200ms 量级），
  全员耗尽后重算（recalc：counter = counter/2 + priority，对应 2.4 的 for_each_task）。
- nice(-20..19) 映射 priority；yield/sched_yield 系统调用。
- idle 任务（每 CPU 一个 kthread，hlt 循环，对应 2.4 init 进程 idle）。

## 3. 等待队列 / 睡眠唤醒（kernel/waitqueue）

- WaitQueue：节点 { task, next }；`wait_event(wq, cond)` 宏（不可中断）与
  `wait_event_interruptible`（信号可打断，返回 ERESTARTSYS 语义）。
- wakeup 分 exclusive/none（2.4 语义），用于 pipe/信号量/IO 完成。
- M1 先按 xv6 sleep(chan)/wakeup(chan) 翻译；M3 平移到 waitqueue（chan 作为 key 保留兼容）。

## 4. 内核线程（kernel/kthread.rs）

- `kthread_create(fn, data, name)`：共享内核地址空间（无 user mm），入队 runqueue。
- 内核线程不对用户暴露 pid 冲突（占 pid 空间，/proc 中 [名] 标注，对应 2.4 显示习惯）。
- 落地实例：ksoftirqd（软中断压制阈值后接管）、bdflush（脏 buffer 刷盘）、
  kupdated（30s 周期 sync）、kswapd（M2 内存回收，先空转打点）、keventd 简化（延迟工作队列，
  仅 event 泛化用，Stretch）。

## 5. 软中断 / tasklet / BH（kernel/softirq.rs）

- softirq 向量表（HI_SOFTIRQ/TIMER/NET_RX/NET_TX/BLOCK/TASKLET），`open_softirq`/`do_softirq`
  在中断返回与 ksoftirqd 两个出口执行；pending 位图 + 本地禁用计数。
- tasklet：同类型串行（per-cpu 状态位），HI 队列优先；M3 用于 console 缓冲 flush、
  块 IO 完成回调（M4）。
- 保留 2.4 的 BH 兼容概念但不实现旧 BH 接口（无历史包袱）。

## 6. 时间与定时器（kernel/timer.rs, arch time.rs）

- jiffies（u64，HZ=100，PIT 通道 0）；timespec/gettimeofday 由 jiffies+TSC 校准（Stretch）。
- 定时器轮：一层 512 槽（10ms 粒度）+ 溢出链；`add_timer/del_timer`，
  用例：睡眠 sys_sleep、管道超时（Stretch）、NAPI 竞争（Stretch）。
- uptime/sys_times 系统调用。

## 7. 信号（kernel/signal.rs）

- 每任务：pending 位集 + blocked 位集 + handler 表（SA_RESTART 等最小标志）。
- 发送路径：kill(-pid)/终端 ^C（SIGINT）/内核异常（SIGSEGV/SIGFPE）/CHLD。
- 投递点：系统调用返回 / 中断返回的用户边界；rt_sigreturn 恢复现场（trampoline 嵌用户页，
  帧布局沿用 Linux i386 sigframe 简化版）。
- 默认动作表：TERM/KILL/Core/Ignore/Stop/Cont 核心集；不可捕获 SIGKILL/SIGSTOP。
- 无实时信号队列（2.4 有 RT 队列，minios 标记 Out/Stretch，理由：无 RT 信号用户）。

## 8. 验收用例（摘）

1. 公平性：8×SCHED_OTHER 计数循环 5s，CPU 时间方差 <5%。
2. RT 抢占：FIFO 高优先级线程空转，OTHER 进程 starvation（预期行为）。
3. 信号：父 SIGCHLD 收割；^C 终止前台；SIGSEGV core 路径（打印后终止）。
4. 定时器：sys_sleep(1) 误差 <20ms；1000 个并发定时器回调无丢失。
5. kthread：bdflush 周期活动日志；ksoftirqd 在洪峰下接管。
