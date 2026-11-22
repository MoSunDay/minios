# 50 — 风险登记册与架构决策记录（ADR）

## A. 架构决策记录

### ADR-0001 Rust / no_std / 零第三方内核依赖
- 状态：已接受。跨架构复用；unsafe 仅限硬件与底层原语边界。

### ADR-0002 原目标 i686（32 位）
- 状态：**被 ADR-0011 取代（2026-11-15）**。旧实现和证据仅归档，不计当前完成度。

### ADR-0003 Multiboot1 主引导路径
- 决策：M0R 主路径为 QEMU `-kernel` 直载 Multiboot1 ELF，进入 32 位保护模式入口，
  再建立低恒等/高半区映射并进入 x86_64 长模式；过渡指令不构成 32 位内核或 ABI 支持。
- 回退：GRUB Multiboot ISO 尚未批准；如确有需要，先新增 ADR 并在 40 冻结独立配置、
  工具版本和镜像 hash，再生成新证据，不沿用主路径结论。
- 状态：已接受（按 ADR-0011 修订）。

### ADR-0004 ext4 分阶段交付
- 决策：M5 RO → M6 无日志 RW → M7 jbd2 ordered/replay；M4→M5→M6→M7 不可绕过。
  e2fsprogs 是外部权威校验，后 2.6 扩展不自动进入 Parity-2.6.39.4。
- 状态：已接受；Core 子集与完整基线差距分开验收。

### ADR-0005 minios 全新历史，不导入 xv6 历史
- 状态：已接受；无二进制政策不变。

### ADR-0006 先保持 xv6 可运行语义，再升级机制
- 状态：已接受；M1R 先完成 x86_64 等价，M2+ 对齐 Linux 2.6.39.4 机制。

### ADR-0007 原“用户态先使用 C”方案
- 状态：**被 ADR-0013 取代**。旧 C 用户程序只保留为历史证据，不进入新目标。

### ADR-0008 系统调用直接使用 Linux x86_64 编号
- 决策：M1R 起使用 2.6.39.4 x86_64 原生号段与 syscall/sysret，只实现白名单子集；
  M9 前从冻结的权威源码、syscall 表和 UAPI 输入生成可再生成差距表。编号对齐不等于 ABI 完整。
- 状态：已接受。

### ADR-0009 原 LinuxThreads 最小线程模型
- 状态：**被 ADR-0012 取代**。M8 交付 clone/TID/futex 线程基础；完整 NPTL、线程组信号、
  RT 信号与 robust futex 等由 M9 权威清单关闭，不再永久排除。

### ADR-0010 FPU/SSE 上下文在 M1R 落地
- 决策：x86_64 用户态基线含 SSE2，M1R 先 eager fxsave64/fxrstor64；惰性切换只能作为
  后续可回归优化。内核禁止自动 SSE/MMX，BSP/AP 均须验证控制状态与保存恢复。
- 状态：已接受。

### ADR-0011 唯一目标架构改为 x86_64
- 背景：用户于 2026-11-15 明确不再支持 x86 32 位。
- 决策：target 为 `x86_64-unknown-none`，采用 ELF64、四级页表、syscall/sysret 与 GS-base
  percpu；不交付 i686/ia32 compat。旧 M0/M1 重开为 M0R/M1R，旧证据与工期不沿用。
- 配置：Core QEMU/工具配置自 M0R 起按 40 冻结；M9 只冻结 Parity 扩展配置，不重写 Core
  基线或借用旧证据。
- 状态：已接受（用户批准）。

### ADR-0012 对齐 Linux 2.6.39.4，分 Core 与 Parity 验收
- 决策：M0R–M8 为 Core-2.6，M9+ 为 Parity-2.6；M9 前冻结可再生成权威差距表及 Parity
  配置。全主线全部配置/驱动/文件系统的 1:1 未立项，M8 不得写成全特性完成。
- **Core 关闭定义**：M0R–M8 所有 required 证据有效且 Core 矩阵全绿；不需要外部 glibc、
  LTP 或 rootfs。缺例、空矩阵、无效证据或任何 in-scope FAIL/SKIP/TCONF/XFAIL 均阻断。
- **Parity 关闭定义**：Core 仍全绿，冻结配置的 M9–M13 in-scope 项与 External Parity 矩阵
  全部通过；任何 in-scope FAIL/SKIP/TCONF/XFAIL 均阻断。out-of-scope 只能由失败前已接受的
  ADR 冻结范围、理由和替代测试，禁止事后豁免或以文档记录缺口冒充完成。
- 状态：已接受（用户批准方向；详细 Parity 配置在 M9 前冻结）。

### ADR-0013 实现语言统一为 Rust
- 决策：kernel/user/abi/tools/host-tests/tests 的内核、用户程序、runtime/ulib、镜像工具、
  runner、verdict 与仓库内测试全部使用 Rust。内部用户对象由 rust-lld 按独立脚本直链为
  静态非 PIE ET_EXEC，不经过 cc/gcc，不带 crt/libc/libgcc；汇编只用于必要硬件边界。
- 脚本职责：shell/Python 仅启动进程、传参、设置 timeout 和收集日志，不编写测试用例、
  不解释结果、不自行判绿。M1R 才启用完整 `test-qemu`、内部 ELF 检查与 `check-rust-only`。
- 外部隔离：xv6 C 是只读参考；外部 glibc/LTP/rootfs 及其 C 编译链只在独立 Parity job 和
  隔离目录中，固定来源、版本与 hash，不进入 minios workspace、实现树、Core 镜像或依赖链。
- 状态：已接受（用户批准）。

### ADR-0014 QEMU 6.2 ELF64 直载失败，M0R 冻结 GRUB Multiboot ISO 回退
- 背景：冻结的 QEMU 6.2.0（Debian 1:6.2+dfsg-2ubuntu6.31）对 `-kernel` 加载 ELF64 内核
  返回 1 并报 `Cannot load x86-64 image, give a 32bit one.`；40 的直载主路径在 M0R 不可用。
- 决策：M0R 真实启动走独立配置 `CORE-UP-GRUB`：`grub-mkimage`（GRUB 2.06-2ubuntu7.2，
  `i386-pc-eltorito`，内嵌 early config）+ `genisoimage 1.1.11` 生成 El Torito ISO，
  QEMU 以 `-cdrom` 引导。内核仍保持 Multiboot1 ELF64 低入口，ISO 仅是装载器外壳；
  静态 Multiboot gate 与协议行不变。完整命令、模块表、镜像 hash 按 40 §1.1 冻结。
- 约束：`CORE-UP-GRUB` 只替代装载方式，机器型号、CPU、内存、`-smp`、串口、
  `isa-debug-exit,iobase=0xf4,iosize=0x04`、`-no-reboot` 等参数继承 `CORE-UP` 基线，
  不因回退放宽任何判定；GRUB 串口自身的输出行不属于协议行。
- 状态：已接受（依据 40 §1.1 预留的回退条款；升级 QEMU 后可再评估恢复直载主路径）。

### ADR-0015 M2 匿名 OOM：候选选择 + 终止 + 统计（无 swap）
- 背景：22 §5 M2 gate——匿名页无 swap 不可回收，用户页分配失败只能靠终止
  最大匿名消费者解压；要求验证 OOM 候选选择、终止与统计（扫描/拒绝/释放/
  失败计数），不得误杀无关任务或静默丢内存。
- 决策：`mm/oom.rs`——纯函数候选选择（最大匿名足迹 `count_user_pages`
  胜出，平手取最低 pid，零足迹不可选）；匿名用户页分配统一走
  `oom::alloc_user_page()` 阀门（`Mm::alloc` 的 brk/mmap/exec 段 eager 填充、
  `map_zero_page` 惰性填/栈增长、COW 私有拷贝）：kalloc 失败→failed 计数→
  ptable 扫描（UNUSED/EMBRYO/ZOMBIE/已 killed/无 mm 一律 rejected）→选中者
  标记 killed 并按 chan 唤醒；victim==自害（风暴进程）时阀门返回 null，走
  正常 fault-kill/mmap 回滚路径，他害时 64 次有界 yield 重试等其退出释放。
  统计 scanned/rejected/freed/kills/failed（Spinlock 静态，快照只读导出）。
- 约束：fork 页表分配与 kmalloc 保持礼貌 ENOMEM，不入 OOM；内核态触发
  （m2 引导上下文）无候选只计数不杀；uaccess/fixup 的真实 EFAULT 语义不变；
  COW 共享帧按映射数计为坏度近似（M2 文档化近似，M8 SMP 重审）。
- 状态：已接受（m2.oom.select_stats 内核用例：1 GiB 过量申请拒收+回滚帧
  守恒+计数断言；xv6.mem.oom 用户风暴：最大足迹子进程恰被终止 0xff、驱动
  存活且事后恢复内存服务；coverage M2 required，ADR-0015 即本条）。

### ADR-0016 M3 CFS（UP）：抢占显式关闭 + 简化策略（tick 记账）
- 背景：21 §2——M3 交付 UP CFS（sched_entity/vruntime/红黑树/nice 权重/
  集中 granularity），内核抢占保持关闭并显式审计（preempt_count），不做
  full-PREEMPT 声明；公平性 <5% 仅作宿主确定性模型门（固定输入/节拍/
  无迁移），QEMU 公平性走 guest-time + 预声明容差。
- 决策：`proc/rbtree.rs`（侵入式红黑树，缓存 leftmost，等键 FIFO）+
  `proc/cfs.rs`（纯策略：Linux nice −20..+19 权重表、`update_curr` 加权
  vruntime、`min_vruntime` 单调、集中 `granularity = max(latency/(queued+1),
  min_gran)`，粒度分母含当前实体——对齐 Linux `nr_running` 语义）+
  `proc/preempt.rs`（UP preempt_count，`sched_raw` 前断言可抢占；消费者
  s5/s7 落地）；`sched.rs` 换挑选（ptable/swtch/forkret 所有权协议零改动，
  就绪队列共用 PTABLE_LOCK，无新锁序）；`entity_tick` 仅用户态定时陷阱
  抢占；`SYS_SCHED_YIELD(24)`。
- 简化（相对 Linux，均为有意决策）：记账粒度 = 100 Hz PIT tick（TSC 精化
  s4）；新实体/唤醒实体 clamp 到 `min_vruntime`（无 sleeper bonus、无 fork
  惩罚）；抢占条件 `curr.vruntime − front > granularity`；`exec_start`
  无零哨兵（开机首 pick 合法为 0——首版哨兵与 TICKS==0 冲突致首个进程
  永不记账，宿主模型先行拦截）。宿主门：同权重 4 实体 4000 tick 散布
  <5%、nice 0:5 比值 ≈ 1024:335（±5%）、两实体交替 ≤2 tick。
- 约束：RUNNABLE ⟺ 在队这一不变量覆盖全部 6 个迁移点（fork/kill/
  userinit/yield/wakeup1/pick）；`m1r-user xv6.ipc.preempt` 为金丝雀；
  RT/迁移/组调度不在本 ADR（s2 起）。
- 状态：已接受（host-tests m3tests/rbtreetests 10 例；QEMU 电池 5/5 含
  m1r-user 39/39；本条即 ADR-0016）。

### ADR-0017 M3 RT（UP）：per-prio FIFO 链 + u128 位图，FIFO/RR 语义与 tick 界延迟
- 背景：21 §2 s2——RT（FIFO/RR）先于 CFS 挑选；policy/prio 经
  setscheduler/setparam 族暴露（Linux ABI 142–147）。
- 决策：`proc/rt.rs`——每优先级一条 FIFO 双端链（头尾指针）+ `u128`
  位图（bit = prio−1，`front_prio` = 最高置位位 + 1）；`SchedPolicy`
  Normal=0/Fifo=1/Rr=2（`from_i64` 校验），`rt_prio` 1..=99；RR 量子
  10 tick（100 ms）。`sched.rs`：`pick_next_proc` RT 先于 CFS；类感知
  入队不变量（RUNNABLE ⟺ 在本类队列）；FIFO 被抢占 → `push_head`
  （保持轮次），RR 量子耗尽 → 旋转到尾；tick 见任何 runnable RT 即
  抢占 CFS 当前体。fork 继承 policy/prio；freeproc 复位。
- 简化（有意，相对 Linux）：无 RT 带宽/节流（unbounded RT 可饿死 CFS，
  同 Linux 未开 throttling 的语义，验收文档化）；无 IPI 重调度——重调度
  延迟由 10 ms tick 界定（M8 SMP 前不做）；无 sched_rr_getinterval/
  迁移/组调度；policy 迁移（setscheduler）= 出旧类队 + 入新类队，
  RR 重置量子、FIFO 入尾。
- 锁纪律：6 个 syscall 均在 PTABLE_LOCK 内快照 prio/policy，copyout
  在放锁后（沿 M2 纪律）；两类就绪队列共用 PTABLE_LOCK，无新锁序。
- QEMU 套件（m3suite，cmdline `m3-suite`）：策略级 6 例（pick 顺序/
  tick 公平/RT 级序/FIFO 抢占回队头/RR 旋转/policy 接线）；**首跑抓到
  `#[repr(Rust)]` 布局假设 bug**——套件以裸 offset-0 强转回收容器，
  debug（host）与 release（内核）布局重排不同致读错字段/越界；修复 =
  与 sched.rs `Proc` 相同的 `offset_of!` container_of 惯例
  （`ent_of_se`/`ent_of_link`），任意布局下成立；该红跑顺带实证 runner
  的 FAIL 判定路径与 kernel-oops FATAL 路径。
- 状态：已接受（host rttests 4 例 + rbtreetests/m3tests；QEMU 电池
  6 目标全 PASS 含 m3-suite 6/6、m1r-user 39/39 金丝雀）。

### ADR-0018 M3 waitqueue（UP）：纯链表层 + Linux 唤醒纪律 + ERESTARTSYS 回卷
- 背景：21 §3——waitqueue 承接可中断/不可中断与 exclusive 唤醒；chan
  sleep/wakeup 保留为底层原语（console/ide/bcache/ilock/log 等 xv6 期
  睡眠者仍在 chan 上，后续阶段机会迁移）。
- 决策：`proc/waitqueue.rs` = 纯链表策略（如 rt/cfs 可宿主单测）：
  WaitLink（next/prev/queue 回指/interruptible/exclusive）+ WaitQueue
  （头尾）+ `enqueue_link`（Linux 纪律：非 exclusive 头插、exclusive
  尾插）+ `unlink`（幂等）+ `select_wake`（one = 全部非 exclusive +
  首个 exclusive；all = 全体）；内核胶水（`wq_sleep`/`wake_up`/
  `wake_up_locked`/`interrupted`）在 sleep.rs——锁协议与 chan sleep
  完全一致（资源锁序在前，PTABLE_LOCK 过 swtch 转移所有权）。
- 迁移：pipe 单一 WaitQueue——读者非 exclusive（写者落数据 wake-all）、
  写者 exclusive（读者腾空间 wake-one，防惊群）；wait4 睡 PTABLE_WAITQ
  （可中断），exit() 唤醒全部；kill() 零改动（唤醒任意睡眠者），睡眠者
  醒后幂等自摘链——可中断等待以 ERESTARTSYS 破局。顺带落地 pipewrite
  的 EPIPE（读者全关时写 → -EPIPE；SIGPIPE 信号与用户用例在 s6）。
- ERESTARTSYS：`syscall_dispatch` 见 -512 时 rip -= 2（`syscall` 指令
  2 字节）回卷待重启；s6 前唯一中断源是致命 kill——重入调用因 killed
  返回 -1（xv6 可见契约），进程在下个用户 trap 退出；修复了 xv6 的
  已知 wart（被 kill 的管道睡眠者永远不醒/再睡死循环）。
- 简化（有意）：无 wait_queue_t 回调（default_wake_function 等价 =
  直接 runnable_wake）；无 prepare_to_wait/finish_wait 族拆分（单
  wq_sleep 内联等价）；UP 单 WaitLink（一进程同时至多睡一队）。
- 状态：已接受（host waitqtests 6 例；m3-suite 扩至 8 例含唤醒纪律/
  中摘幂等；m1r-user 39/39 **串口钉值跨迁移字节不变**——pipe/wait/kill
  全契约保持的直接证据）。

### ADR-0019 M3 时间层（UP）：clocksource/hrtimer/wheel 三层 + syscall 集成，亚 tick 精度与 RTC 延后
- 背景：21 §4——时间层为 CFS 记账与 nanosleep/alarm/gettimeofday/wait4
  -rusage 提供地基；PIT 周期 tick 是唯一时钟事件源（LAPIC one-shot
  延后 M8）。
- 结构：`kernel/time/` 模块五件套（原 kernel/time.rs 48 行拆分）：
  - clocksource.rs：TSC + PIT 校准（`calibrate_tsc` 忙等窗口内
    sti/cli——校准循环必须能被 IRQ0 打断退出，否则 boot 死锁 #1）；
    `cycles_to_ns_with(delta, per)` 纯函数 + cycles_to_ns 委托（宿主
    可测）。
  - clockevents.rs：时钟事件设备骨架 + CLOCK_EVT_FEAT_ONESHOT
    （M8: LAPIC one-shot 编程，暂 dead_code 标注）。
  - hrtimer.rs：Abs/Rel 到期有序链（UP 规模 O(n) 插入换实现简单，
    红黑树不必要），`run_expired(now)` 显式 now 驱动（确定性，无需
    IRQ）；Rel 支持重挂载重排。
  - wheel.rs：经典 tv1–tv5 级联轮（256/64/64/64/64 桶，
    WHEEL_HORIZON = 1<<26 jiffies），LrTimer + add_timer/del_timer/
    expires()/jiffies()；**过期停泊语义**：past-expiry 定时器落 tv1
    桶 `jiffies & 255`，索引转回时才触发（≈256 tick 后）而非下一
    tick——Linux 同源行为，host 测试 wheel_past_expiry_runs_on_revisit
    固化。
  - mod.rs：init_time（PIT 周期模式 → unmask → calibrate_tsc）+
    tick()（jiffies 推进 + wheel 级联 + hrtimer run_expired）。
- **关键教训（boot 死锁 #2，指纹 pic0 irr=01 imr=fb）**：
  `pic::unmask(IRQ_TIMER as u8)` 把向量号 32 当 IRQ 线号传入——
  unmask(32) 走从片路径：清主片 bit2（级联）且 `1u8 << (32-8)` =
  1u8<<24 **回绕到 bit0** → 从片 IRQ8 被开、IRQ0 仍被屏蔽 → PIT
  永不送达 → calibrate_tsc 死转，所有 m1r_boot 模式挂死在 GRUB
  横幅。正确惯用法 `pic::unmask((IRQ_TIMER - IRQ_BASE) as u8)`
  （ide.rs 同款）。教训：u8 移位回绕无任何断言，裸线号 API 的
  调用点必须显式做向量→线号换算。
- 简化（有意）：
  - boot_epoch_ns=0（无 RTC/CMOS 读取，wall-clock 延后到 RTC 驱动
    里程碑）；gettimeofday 返回 monotonic 语义 ns。
  - 亚 tick 精度延后 M8（LAPIC one-shot）；hrtimer 到期由 tick()
    驱动——nanosleep 短睡按 tick 粒度对齐。
  - wheel（睡长觉）与 hrtimer（高精度接口）双轨并存，Linux 同构。
- syscall 集成：SYS_gettimeofday(96)（16B timeval，NULL 合法）、
  SYS_alarm(37)（wheel LrTimer 宿主 Proc.alarm，到期 cb alarm_fire
  置 alarm_fired 后 kill_proc；prev 剩余 = expires()-jiffies() 向上
  取整，0 取消，WHEEL_HORIZON 钳制，负数 EINVAL）、SYS_wait4 增
  rusage（144B，ru_utime 自 sum_exec_ns——exit 前 wait_status 三元组
  捕获，防 freeproc 抹账）、SYS_nanosleep 重写于栈上 HrTimer（Abs
  截止，chan=定时器地址一次计算，nanosleep_wake cb 先取 TICK_LOCK
  再唤醒——防丢醒；一切返回路径含 EINTR 均取消定时器）。
- 状态：已接受（host timetests 9 例 58→67；m3-suite 扩至 11 例
  +3 time；m1r-user 39/39 串口钉值跨 nanosleep 重写字节不变——
  对用户态契约无扰动的直接证据）。

### ADR-0020 M3 s5（UP）：softirq/tasklet/ksoftirqd + workqueue + kthread，时间层单时基不变量

- 背景：21 §3——中断底半部与内核线程基础设施：hardirq 只置位
  （raise），重活延后到 IRQ 出口或进程上下文；kthread 是 RCU（s7）
  与 writeback（M5）的执行体地基。
- 结构：
  - `kernel/softirq.rs`：2.6.11 索引（HI=0/TIMER=1/NET_TX=2/
    NET_RX=3/TASKLET=5，NR_SOFTIRQS=6）；静态 vec/data 槽 + PENDING
    原子位图；open/raise/pending；`do_softirq`（in_softirq 深度
    守卫 + local_bh_disable、MAX_SOFTIRQ_RESTART=10、升索引
    run_mask 快照分发、预算耗尽唤醒 ksoftirqd）；traps_dispatch
    在 `vector >= IRQ_BASE` 的 IRQ 出口统一钩 `irq_exit()`。
    tasklet 走 TASKLET/HI 两条表（SCHED 位去重、先清位再跑——
    自重排合法、禁用项跳过）。ksoftirqd 内核线程：排空循环 +
    空转 wq_sleep 停泊（条件是原子位图，waker 不取该锁）+
    interrupted 退出路径。
  - `kernel/workqueue.rs`：events 静态队列（FIFO + WorkStruct
    pending 去重 + more/done 双等待队列 + 惰性 events worker
    kthread）；flush_work 睡 done 至"表空且非运行"；schedule_work
    简写（M5 writeback 消费，暂 dead_code 标注）。
  - `proc/kthread.rs`：kthread_create/run/reap + naked
    `_kthread_trampoline`（对齐后 call kthread_body；body 里
    raw_release + sti 后跑 fn(arg)，返回值走 exit(code)）。
  - `proc/preempt.rs`：preempt_count 低 8 位抢占位 + 位 8-15
    softirq/BH 深度（2.6 布局缩影；sched_raw 审计 preemptible）。
  - `kernel/time/mod.rs`：tick() 只 raise TIMER_SOFTIRQ；到期分发
    （hrtimer run_expired + wheel_tick）移入 timer_softirq 处理器，
    IRQ 出口执行——jiffies 语义与 s4 相同（tick 内递增后才分发）。
- **关键教训 1（swtch 镜像槽序与压栈序相反）**：ctx_sp 内存像
  `[r15][r14][r13][r12][rbx][rbp][resume-ret][next-ret]`——
  swtch.S 压 rbp..r15、弹 r15..rbp、ret 弹槽 6；kthread 种子
  slot2=arg(r13)、slot3=fn(r12)、slot6=trampoline。allocproc 注释
  按 PUSH 序罗列寄存器，易误读为槽序。首跑 fn/arg 反置 →
  kthread_body(0, fn) 经 0 间接调用 → oops `rip=?0x1`
  （指纹：rax=1 写故障）。GDB 断点核验后修正。
- **关键教训 2（单时基不变量：softirq 处理器先于首 tick 就位）**：
  wheel_tick 每次调用恰推进 1 jiffy（空桶即断、无追赶循环——
  Linux 形状）；s5 把到期分发从 tick() 内联挪到 TIMER_SOFTIRQ 后，
  init_time 若在 calibrate_tsc（~5 tick 的开窗忙等）之后才
  open_softirq，校准窗内的 tick raise 了却无人分发 → WHEEL.jiffies
  永久滞后 TICKS 5 拍 → 以 TICKS 为时基装填的 m3.time.wheel_cascade
  在等待窗内永不触发（等待窗 expires+2 早于轮到位）。修正：
  open_softirq 提前到 unmask/校准之前——TICKS 与 WHEEL.jiffies
  自第一拍起 1:1。调试指纹：GDB 证明 timer_softirq 每拍都进
  （入口 pc=0x100=BH 深度）而 wheel_probe 不响、WHEEL.jiffies
  纹丝不动——注意 Rust 会重排结构体字段，jiffies 实际在
  WHEEL+0x1000 而非 +0（读错字段是调试干扰项）。
- 简化（有意，与 Linux 分歧）：
  - do_softirq 全程 IF 屏蔽（Linux 处理期间重开本地中断）——UP
    简化，重启预算兜底活锁；
  - raise_softirq 不从 hardirq 直接唤醒 ksoftirqd（Linux
    wakeup_softirqd 可在 irq 上下文调度唤醒）——仅预算耗尽或
    进程上下文 raise 唤醒，避免 hardirq 路径触碰调度器；
  - 禁用（count>0）tasklet 跳过不重排队（Linux 挂回表尾）；
  - workqueue 惰性单 worker（非 per-cpu pool + manager）；
    flush_work 以"表空且非运行"为条件（无 per-work 引用计数）；
  - ksoftirqd 不做 CPU 亲和/NICE 值配置（UP、无 CFS 用户分级）。
- 状态：已接受（m3-suite 11→16 例：kthread.run_exit /
  workq.dedup_run / softirq.order_bh_mask（探测走 NET_TX/NET_RX，
  不劫持 init 注册的 HI/TASKLET 槽）/ softirq.tasklet_sched /
  softirq.ksoftirqd_pressure（自增殖风暴 → 预算耗尽 → ksoftirqd
  RUNNABLE → 停风暴 → 排空归睡）；不新增 host 测试——softirq/
  kthread 语义绑定中断与调度上下文，QEMU m3 电池承保覆盖；
  m1r-fault/m2 串口因代码布局重钉，m1r-user 39/39 钉值字节不变）。

### ADR-0021 M3 s6（UP）：Linux 式信号——sigframe/rt_sig* 契约、ERESTARTSYS orig_ax、EPIPE/SIGPIPE 与 SIGCHLD

- 背景：21 §6/§7——把 xv6 的 kill/exit(-1) 语义升级为 2.6 式信号：
  per-proc pending/blocked/64 槽 dispositions，帧式投递（handler +
  restorer + rt_sigreturn），同步异常与 EPIPE/SIGCHLD 生成点。
- 结构：
  - `kernel/sigset.rs`（s6b）：NSIG=64 位图代数（sigmask/
    first_pending/MASK_SIGKILL 纯集合语义）；SigAction {handler,
    flags, restorer, mask}；`abi::sig`（编号/SA_*/how/32B
    UserSigaction）与之 `const _: () = assert!` 交叉钉死（host-tests
    零依赖，不能引 abi——编译期断言放内核侧防漂移）。
  - Proc 增 `sig_pending/sig_blocked/sig_actions:[SigAction;64]`
    （freeproc 清零，allocproc 不重播）；fork 继承 actions+blocked
    不继承 pending；exec 保留 blocked+SIG_IGN、其余回 DFL、pending
    清空；生成点规则：IGN 在生成时丢弃、阻塞挂起、DFL 终止类在
    投递点 `exit(-256-sig)`（wstatus `(-x)&0x7f`——xv6.mem.oom 的
    legacy `exit(-1)` 仍走 `(x&0xff)<<8` 通道）；SIGKILL 仍走
    kill_proc（killed=1 → exit(-1)），Stop 类在 M8 前按终止处理。
  - Sigframe 208B（retaddr+0/signum+8/magic+16/tf+24/mask+200），
    `frame_base_for(rsp) = (rsp-344 & !0xf) + 8`（16B 对齐 + 红线
    常量可审）；投递改写 tf（rip=handler、rsp=retaddr 栈），
    rt_sigreturn(15) 校验 magic 后整体还原；SA_NODEFER 投递期免
    自屏蔽、SA_RESETHAND 一次性回 DFL。
  - 生成点三处：pipewrite 环满且 readopen==0 → 先放管道锁再
    `sig_send(SIGPIPE)` → 返回 -EPIPE（POSIX：阻塞/忽略也照返）；
    exit() 持 ptable 锁 `sig_send_locked(parent, SIGCHLD)` 先于
    `wake_up_locked(PTABLE_WAITQ)`（wait4 睡眠者无论 disposition
    都被唤醒收尸）；同步异常 fault_cur **强制越过 SIG_IGN**（IGN
    复位为 DFL 再投递——否则缺页循环死锁），alarm 到期走 timer
    软中断。
  - `interrupted()` = killed || pending&!blocked：可中断睡眠
    （管道读写、wait）被未决未阻塞信号唤醒返回 -ERESTARTSYS。
- 调试期真实缺陷（三项，均已修复并在 m1r-user 45 例承保）：
  1. sysfile.rs 分块 sys_read/sys_write 把负 fileread/filewrite
     返回改写成 EIO/短读——吞掉 -512 ERESTARTSYS 与 -32 EPIPE；
     改为 `total>0 ? total : r` 原码透传（部分传输 POSIX 短读写
     站立、不重启）。
  2. traps.rs ERESTARTSYS 臂写回 `tf.rax=ERESTARTSYS`，回卷的
     `syscall` 再入以 nr=-512 分发 → ENOSYS(-38)；Linux orig_ax
     方案：进入分发前暂存 `orig_nr=tf.rax`，重启臂恢复之。
  3. 用户态 sigreturn_stub 以 `static as u64` 取"地址"实际加载了
     首个操作码字节 0x48（restorer=0x48 → ifetch 缺页）；改
     `core::ptr::addr_of!(sigreturn_stub_addr) as u64`。
  （另两处用例侧缺陷：sigpipe 用例父进程关错端（关 fds[1] 留
  fds[0] → readopen 永不为 0 → 子进程满环死睡 + 父进程写已关 fd
  得 EBADF）；共享 HANDLED 计数器跨用例未清零——mask_defer 留下
  SIGUSR2 使 sigpipe 的 alive 检查误判。）
- 简化（对照 2.6.39.4）：单 pending 位无 RT 队列；无 sigaltstack/
  SA_SIGINFO 四参形；rt_sigaction 不设 SA_RESTORER 时拒绝而非
  代填（glibc 契约显式化）；同步信号内核栈内直接处置，无
  force_sig_info 队列；SIGKILL/Stop 不受掩码语义影响的完整集在
  M8 job control 补齐。
- 增补（s7，ADR-0022）：userret_deliver 头部现承载 RCU 回用户
  QS 上报（用户态不持读临界区；仍对 parked 内核读者检查）——
  信号出口与 RCU 静默判定共用同一漏斗点。
- 状态：已接受（m1r-user 39→45 例：sig.roundtrip / sig.mask_defer
  / ipc.sigpipe（默认死 wstatus&0x7f=13、阻塞/忽略存活返 -32 且
  无 pending）/ sig.except（默认+越过 IGN）/ sig.chld / sig.alarm
  （用户态自旋中经 IRQ 出口投递）；m3 19/19、host 75（+8 sigtests）
  绿；m0r/m1r-suite/m1r-fault/m3 串口钉值不变，m1r-fault/m1r-suite
  root 因用户 ELF 模板重钉，m2 串口因 vma 布局打印重钉——均 ×2
  字节稳定）。

### ADR-0022 M3 s7（UP）：RCU tiny——纯批量引擎 + 五处 QS 上报 + 空转 liveness，读者延迟回收

- 背景：21 §5——UP RCU 基础：读侧临界区标记、quiescent state、
  callback 队列与 grace period；先服务可替换指针/小型配置数据
  （dcache 类消费者 M4+ 落地，m3 用探针注册表先行承保）。
- 结构（两段提交 s7a/s7b）：
  - `kernel/rcu_engine.rs`（s7a，纯函数无内核依赖，host-tests
    include-by-path 直测）：TINY 三链批量管线 nxt→cur→done（
    RcuHead{next,func} 侵入式零分配链）；`queue_cb` 恒入 nxt，
    `advance` 先开后收——cur 空且 nxt 非空时整体接走 nxt（**合并
    批语义：GP 打开前入队的所有回调并成一批**，classic TINY
    nextlist 累积），qs_pending 则本批收进 done；`note_qs(
    readers_active)`：读者在内或无 GP 开 → 无操作 false，否则
    qs_pending 并返回 may_complete（调用方 raise RCU_SOFTIRQ）；
    `take_done` 锁外 FIFO invoke。host 9 例钉死：FIFO 链序/晚入
    队另开 GP/nxt 合并（multiple_gp_boundaries：GP2 打开前
    batch2+batch3 合并成 4 回调一批——正是 gen_swap_pressure 里
    GP 数 < 批数的内核侧根源）/parked 读者抑制、末读者出释放。
  - `kernel/rcu.rs`（s7a 胶水）：ACTIVE_READERS 全局计数 +
    RCU_LOCK（PTABLE_LOCK→RCU_LOCK 唯一文档化嵌套——swtch QS
    站点持 ptable）；**QS 五处上报**：sched_raw 自愿 swtch /
    userret_deliver 回用户（s6 入口现承载，见 ADR-0021 增补）/
    tick `check_callbacks(from_user)`（用户态 tick 是 QS，内核
    tick 只机会性 raise）/ rcu_read_unlock 末读者补报（deferred
    QS）/ **scheduler() 与 sched_drive() 空转 note_idle**——
    全系统 parked 时 GP 不滞死（Linux idle QS 镜像；m3 套件
    liveness 依赖）。RCU_SOFTIRQ=6（2.6.39.4 RCU=9，压缩表紧随
    TASKLET=5，NR=7）。
  - 读端契约（audit 而非强制）：rcu_read_lock/unlock（myproc 非
    空则 per-proc `rcu_read_nesting` 记账）+ `rcu_deref!`/
    `rcu_assign!`（Acquire/Release）；sleep/wq_sleep
    debug_assert 临界区内不可睡眠；与 stock TINY_RCU 分歧：TINY
    依赖 UP 读者不会被换出，我们仍对**任意 parked 读者**抑制
    QS——违规 park 只延迟回收不 UAF；无 expedited/优先级。
  - 同步服务：synchronize_rcu == rcu_barrier（栈上 SyncDone 标记
    + call_rcu + RCU_WAITQ wq_sleep；禁止持 ptable 调用）。
  - m3 消费者 3 例（s7b，m3 19→**22**，runner spec 同步）：
    basic_grace（drain 开 GP→kthread 退出 swtch 真实 QS→FIFO
    4 回调；synchronize_rcu 严格晚一个 GP）；reader_defer（GP
    开后读者 park：swtch QS 被抑制、块 magic 跨 park 存活、
    unlock 补报释放）；gen_swap_pressure（8 代交换 + parked
    读者持旧代跨交换、kmalloc/kmfree 账户经 rcu_barrier 清零、
    barrier 睡者走通 idle-QS liveness 链）。`drive_rcu` 手工播放
    irq_exit/ksoftirqd 交错（drive+drain 轮次）；irq-exit 提前
    drain 与 unlock-QS 窗口使相邻 retire 合并——GP 数是调度依赖
    （锚定 invocations 而非 GP 数），自由滞后只查上界。
- 调试期真实缺陷（两项，均已修复）：
  1. m3 用例在 PTABLE_LOCK 守卫内调 kthread_reap（其自身取
    ptable）→ 自死锁；改作用域先查 ZOMBIE 再放手 reap。
  2. SLUB 对象误用页级 kfree（frame.rs bad-page panic）→
    kmalloc/kmfree 成对。
- 状态：已接受（host 79+72+5=156；m3 22/22；QEMU 电池 6 目标
  PASS ×2 字节稳定：m0r/m1r-suite/m1r-user 串口不变，m1r-fault
  `855f856a…9080`、m2 `d376e5b4…d019`（镜像移位）、m3
  `e7b5a2fc…1b1e`（+3 例）重钉）。

### ADR-0023 M4 s2（UP）：VFS 内核胶水——wrapper/dcache/inode 表引用契约、zombie/RCU 延迟回收与锁序

- 背景：23（VFS/block）——M4 把 s1 纯核心（dcache/icache/namei，host
  17 例）落进内核：dcache 池、根挂载、walk、create/unlink/link 三段式
  （DENTRY_LOCK 下解析钉住 → 放锁跑可睡眠 fs 事务 → 重新加锁一致化）。
- 结构（vfs_glue 五文件，均 ≤400 行；mod/mutate/xv6_adapt/xv6_obj/wrapper）：
  - **wrapper 1:1 dentry**：xv6fs legacy inode cache 的生命周期对 VFS 层
    不可观测，故每名字包装器持一个 iget 引用（InodeCommon 头 + xv6_ip），
    这是 M4 对 50 槽 inode 表的唯一诚实绑定；s1 Icache 本里程碑不消费
    （tmpfs s6 才是首个消费者）。
  - **引用契约**：`kern_namei_d` 返回 (idup 过的 ip, pinned d)——引用停靠
    proc.cwd，由 exit 的 iput 配平；`kern_namei` 直通返回（**双 idup
    教训**：s2b 曾再 idup 一次，每次成功 lookup 净 +1，m1r-user
    createdelete 45 文件恰耗尽 50 槽表）。Err 路径纪律：create_common
    EEXIST 与 xv6_unlink NotEmpty 的 dirlookup 引用必须纯 `iput` 回还
    （ilock/iunlock 已闭合时禁 iunlockput）。
  - **表压力回收**：wrapper 的 iget 引用即槽位占用，idle dentry 的 LRU
    prune（dispose→release_wrapper→iput）是 256 槽 dcache 对 50 槽 inode
    表的泄压阀：`relieve_inode_pressure(need)` 在锁外、每个可能 iget/
    ialloc 的 fs 调用前执行。
  - **zombie**：unlink 使 wrapper 释放时若 nlink==0，末次 iput 可能
    truncate+sleep，排入 zombie 队列由每个 kern_* mutation 入口
    `reclaim_zombies`（事务内 iput）排干；m4suite 的台账钉以
    drained_slots（drain 后 free_slots）为唯一读数。
  - **RCU 延迟回收**：dentry detach 走 retire_enqueue→call_rcu，宽限期
    后回 dentry free list（RETIRE_BUSY 期间直接 recycle 保正确性）。
  - **锁序（钉死）**：DENTRY_LOCK → xv6 ICACHE_LOCK（叶）；
    DENTRY_LOCK → RCU_LOCK（call_rcu 入队）；RCU 回调锁外运行、重新取
    DENTRY_LOCK，无倒置。持 DENTRY_LOCK 下唯一合法 dput 拼法
    `dc().dput(d)`，锁外 `dput_unlocked`。
  - **walk gate 错误传播（s2c 修正）**：中段组件过非目录时
    may_search 返回 NotDir，walk 原样传播；errno_of 把 NotDir 折叠为
    ENOENT（legacy 平凡 miss 拼法，保电池字节稳定），m4suite 在 VfsErr
    层钉 ENOTDIR。s2b 曾把 gate 失败一律改写 Access（syscall 边缘错报
    EACCES，违背折叠意图），m4.namei.file_as_parent 用例首跑即钉出，
    已修为原样传播。
  - **m4suite（s2c）**：boot 上下文内核态套件（无 proc cwd → 只用绝对
    路径；iderw 轮询可做盘 I/O），m4 前缀 / 级名字（≤DIRSIZ）、每例自
    清理 + free_slots 台账钉（s2b 两处泄漏修复的回归类）。9 例：
    namei 4（root_paths/dirsiz_trunc/negative_flip/file_as_parent）+
    fsops 5（create_eexist/dir_dots/link_two_names/unlink_notempty/
    table_pressure 80>NINODE）。
- 状态：已接受（s2c 时点：host 181、m4 9/9、电池 8 目标 PASS；
  m1r-user serial 与 legacy 字节等价钉持续成立）。

### ADR-0024 M4 s3（UP）：page cache 核心——PageDesc 池/xarray 裁剪/映射归属 wrapper 与读路径

- 背景：23（VFS/block）§2——M4 建 address_space 页缓存：page 记
  index/mapping/ref/dirty/LRU/private，read/readpage 与 s4 mmap 共享同一
  缓存，命中/未命中/IO 计数进 gate；脏页在回写成功前禁止回收。
- 结构（纯核心 `fs/vfs/pagecache/` 四文件 mod/page/tree/lru ≤200 行 +
  内核胶水 `vfs_glue/pcache.rs` + m4suite `pc.rs` wave-2 四例）：
  - **描述符与页框分离**：PageDesc 是 VFS 侧视图（index/mapping/ref/
    dirty/uptodate/private），页框由调用方供给（内核 kalloc 一页），
    描述符池为静态 arena（256 槽 = 1MiB 缓存上限，DENTRY_POOL 手法）；
    离缓存页框经 `free_data` 回调归还（内核 kfree；host 测试 None 泄漏
    ——dcache dispose 先例）。核心不含分配，host 直测。
  - **索引裁剪（xarray → 有序链表）**：per-mapping 按 index 有序的侵入
    链表——xv6fs MAXFILE=140 块×512B≈18 页/文件，O(n) 扫描在 M4 规模
    诚实（dcache 线性扫描先例）；真实 radix 仅当文件规模增长时引入。
    此为对 23 §2「xarray 风格（按文件规模裁剪）」的显式取用。
  - **映射归属**：InodeCommon 嵌入 `i_data: AddressSpace`（Linux 形状），
    `i_mapping` 指向自身（tmpfs/shm 换目标留 s6）；wrap_new 对 T_FILE 绑
    `XV6_ASPACE_OPS` 并设 host 回指。映射与 wrapper 1:1（= 与 dentry
    1:1，ADR-0023 契约延伸）：wrapper_free 先 invalidate_mapping 排干
    idle 干净页，残留活页（pinned/dirty）在 M4 不可能（无写路径、读端
    不跨调用持 pin）——assert 失败即响，s4 mmap/s5 回写时点 revisit。
  - **读路径（kern_readpage 族）**：`kern_read_cached` 逐页 lookup（命中
    拷贝+put）；未命中 kalloc→insert(带 pin)→**放锁**派发 a_ops->
    readpage（xv6 readi+bread 轮询盘路径，s2c 已验）→回拷+put。失败路径
    remove+kfree。`kern_page_probe`（探针）/`kern_pc_reclaim`（压力）/
    `kern_readahead`（仅计数钩子，窗口算法属 Parity——23 §1 纪律）。
  - **ref 契约**：lookup 返回即持 pin（find_get_page 语义），put 归零
    上 inactive LRU 尾；LRU 仅 ref==0 页；reclaim FIFO 扫描跳过 dirty
    （23 §2「禁止以回收为名丢数据」）；池满时 insert 自回收最老干净页
    （池压力即 evict-clean 驱动）；全 dirty/活钉池 → Err(NoSpc)。
  - **锁序（钉死）**：DENTRY_LOCK → PC_LOCK → KMEM_LOCK（wrapper 销毁
    嵌套 PC 于 DENTRY 内；reclaim 在 PC 内 kfree）。PC_LOCK 从不在中断
    上下文获取、从不跨可睡眠 fs 调用持有（readpage 的 ilock 可睡眠，
    故 insert 后放锁填页；UP 不可重入竞争）。
  - **sys_read 改道时序**：冻结 legacy fileread（字节钉）；改道需
    m1r-user 字节等价证明（s2b namei 直通手法），s4/s5 择机。m4suite
    wave-2 四例（表 9→13）：hit_miss（二次读 bread 增量为 0、计数器对
    账）、share_handles（同帧共享）、fill_matches_readi（含 EOF 零尾）、
    evict_clean（FIFO 淘汰→重填）。buffer.rs 增 bread 计数器（纯增量）。
- 状态：已接受（s3 时点：host 195、m4 13/13、电池见 STATUS s3 记录）。

## B. 风险登记册

| # | 风险 | P | I | 缓解 | 触发信号 |
|---|------|---|---|------|----------|
| R1 | 把 Core 误报为完整 2.6.39.4 ABI | 高 | 高 | 00/20/STATUS 分开 Core/Parity；M9 先建权威清单 | 无范围 1:1 或提前 done |
| R2 | x86_64 迁移破坏已可用语义 | 高 | 高 | M0R/M1R 分批，只验到期 required；每批回归 | 启动或用例回退 |
| R3 | ext4 写路径损坏镜像 | 中 | 高 | 独立副本、严格离线 e2fsck、语义对账与 crash 注入 | fsck 非零或内容不符 |
| R4 | jbd2 ordered/replay 不完整 | 中 | 高 | flush/revoke 专项与 200 次有效 crash；完整差距入 M10 | replay/卸载/语义闭环失败 |
| R5 | target、链接或引导布局错误 | 中 | 高 | M0R 锁 Rust/linker/QEMU，核验 entry/LMA/VMA 与映射 | 构建或真实 `-kernel` 启动红 |
| R6 | SMP/抢占/RCU 死锁竞态 | 中 | 高 | AP 全状态、锁序图、1/2/4 vCPU 压力；Core 可关抢占 | 挂起、UAF 或旧映射 |
| R7 | 网络/驱动范围拖垮主线 | 高 | 高 | Core 网络 Stretch 不进 M8；M9 冻结 Parity 扩展范围 | 未审计范围持续扩张 |
| R8 | 模块膨胀违反规约 | 中 | 低 | 按职责拆分并检查行数 | 文件超过限制 |
| R9 | 历史或生成物混入二进制 | 低 | 中 | 文件类型/NUL 扫描与无二进制 artifact 政策 | scanner 红 |
| R10 | QEMU 与真实硬件差异 | 中 | 中 | 仅以冻结 QEMU 配置验收；真机另行认证 | 真机特有 failure |
| R11 | FPU/syscall/signal 状态损坏 | 中 | 高 | M1R eager、canonical 返回、fixup 与逐 AP 对照测试 | 寄存器错乱或内核 #GP |
| R12 | 编号相同被误认为 ABI 兼容 | 高 | 高 | UAPI/布局差分、固定外部 fixtures 与 LTP | 外部程序或边界用例失败 |
| R13 | 32 位指针假设残留 | 中 | 高 | newtype、ELF64、溢出/高地址/截断用例 | 高地址崩溃或错误截断 |
| R14 | Parity 范围与工期失真 | 高 | 高 | M9 冻结可再生成清单和配置；滚动估算，不虚报进度 | 未审计先承诺日期 |
| R15 | Rust 用户态重建拖延 M1R | 中 | 高 | rust-lld/abi/brk 先行，按覆盖矩阵迁移；禁止 C 充绿 | 到期 required 缺失或覆盖不足 |
| R16 | 工具或外部输入供应链漂移 | 高 | 高 | 固定来源、版本、密码学 hash/digest；锁 Rust 工具链、QEMU、e2fsprogs、外部源/rootfs，先校验后执行 | 来源/hash 不明、latest 或解析变化 |
| R17 | dirty/stale/拼接证据造成假绿 | 高 | 高 | 同 commit/配置/manifest 取证；输入或工具变化即 stale，跨子系统变更重跑完整 Core | 状态非净、hash 不符或旧证据复用 |

## C. 决策流程

新增决策按序编号，不为文档同步虚增 ADR。范围降级或 out-of-scope 必须在失败前接受 ADR，
并同步 00/20/STATUS；QEMU/证据冲突先修订并冻结 40。破坏性 git 操作、数据库写入、部署等
仍需用户明确批准，不因架构方向已批准而自动获得授权。
