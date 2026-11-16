# 24 — IPC / futex / SMP / RCU（对标 Linux 2.6.39.4）

## 1. IPC 与事件机制

- SysV sem/shm/msg：ids/key/ipc_perm、权限、RMID、阻塞唤醒、/proc/sysvipc；
  shm 接 VMA/page cache 与 COW/reclaim，UNDO、完整限制在 Parity 逐项核对。
- pipe/FIFO：页缓冲、O_NONBLOCK、SIGPIPE、poll；M8 支持跨进程与 SMP 唤醒压力。
- futex（M8 基础）：按用户地址哈希等待队列、FUTEX_WAIT/WAKE、copyin 值校验、
  与 mm/clone 生命周期协同；M9 补 FUTEX_REQUEUE/PI/robust list 等 NPTL 所需差距。
- select/poll 与基础 epoll 同为 **M8 Core gate**：统一 file.poll 就绪模型；
  select/poll 覆盖等待、超时与唤醒，基础 epoll 包含兴趣列表、就绪队列、LT 语义
  与并发关闭。ET、完整 epoll 唤醒/并发言义在 Parity 验收。
- POSIX mqueue、eventfd/signalfd/timerfd 与共享匿名 mmap 进入 Parity 清单；
  前三者不得作为 M8 基础 epoll 的依赖或测试替身。

## 2. x86_64 SMP 启动与 per-cpu

- BSP 解析 MP/ACPI 表（Core 可先 MP 表），AP 经 SIPI 进入低页 trampoline，
  依次完成 16→32→64 模式并跳入高半区；每 CPU 建 idle/kthread、GDT/TSS/IDT 与栈。
- 每 AP 还须初始化 FPU/SSE、syscall EFER/STAR/LSTAR/SFMASK MSR、GS base 与
  swapgs 状态；逐 AP trap/syscall/FPU 测试按 21 §7 执行，不能仅验证 BSP。
- LAPIC/xAPIC 本地定时器与 IPI，IOAPIC 路由外设 IRQ；x2APIC、拓扑与热插拔在
  Parity 扩展。8259 服务 M1R/M3 UP，M8 切换后屏蔽。
- per-cpu 使用 IA32_GS_BASE/IA32_KERNEL_GS_BASE MSR 与 swapgs，不使用 32 位
  GDT 基址方案；current/cpu_number/preempt_count/软中断与 RCU 状态放 per-cpu。
- AP trampoline、startup IPI 与所有 MSR 写入必须有 host 可审的常量定义和 qemu 证据。

## 3. TLB、调度与 RCU 的 SMP 化

- 每 mm 记录运行 CPU 位图；unmap/mprotect/exit_mm 对其他 CPU 发 TLB flush IPI；
  考虑进程正在切换 CR3 的竞态，IPI 返回/统计用于压力验证。
- CFS 改 per-cpu runqueue，负载均衡和唤醒迁移遵循统一 runqueue 锁层次。
- RCU 每 CPU quiescent state/callback；调度点、中断返回和 CPU 离线边界必须可前进。
- preempt_count 同时覆盖抢占、软中断、硬中断与 RCU 临界区；Core 可关闭内核抢占，
  Parity 开启后重新审计所有睡眠与 per-cpu 访问。

## 4. 锁依赖图与死锁防控

锁序不再用单一链表近似；按实际“持锁 A 时再获取 B”记录有向边，最低包含：

```text
mm ───────────────► page_cache
inode/dcache ─────► page_cache ─► BIO/request ─► IRQ completion
IRQ completion ───► waitqueue ──► runqueue       # 完成唤醒
timer/pipe/futex ─► waitqueue ──► runqueue       # wakeup → runqueue
page_cache ───────► buffer/page
```

- 中断路径统一 irqsave/irqrestore；IRQ 完成不得持 request 锁进入可睡眠的 VFS/mm，
  waitqueue wakeup 获取 runqueue 的边必须进入同一图，禁止反向持锁等待 IRQ 完成。
- M8 从真实锁获取/wakeup 路径生成 host 可审的依赖表并做环检测，qemu 锁序断言
  也使用这张图；代码新增边必须同步审查，不能继续断言旧线性链就算通过。
- 禁止在中断/RCU/atomic 上下文睡眠。Core 保留上述轻量断言，Parity 才扩展完整
  lockdep 风格状态与抢占测试。

## 5. 验收

1. 2–4 vCPU 启动、每 CPU idle/CFS/定时器可运行；逐 AP trap/syscall/FPU 通过；
   IPI 与 TLB 压力无旧映射。
2. M8 同跑 select/poll、基础 epoll、clone/futex/SysV 双进程与多线程用例；
   关闭/退出竞态无 UAF，eventfd/signalfd/timerfd 不计入 Core 通过率。
3. RCU/SMP 读者压力与 callback 回收；真实锁依赖图无未审环，锁序断言零触发。
4. Parity：外部 NPTL/glibc 黑盒程序、LTP ipc/syscall、完整 epoll/抢占与热插拔
   （若纳入配置）矩阵；仓库内用例保持 Rust。
