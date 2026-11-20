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
