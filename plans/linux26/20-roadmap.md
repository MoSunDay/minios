# 20 — 路线图：x86_64 迁移 → Core-2.6 → Parity-2.6

> 唯一对标基线为 **Linux 2.6.39.4 / x86_64**。M0R–M8 属于 **Core-2.6**，
> M9+ 属于 **Parity-2.6**；二者分开验收，不把 Core 写成完整 Linux ABI。
> QEMU、测试判定与证据规则以 [40-testing-ci.md](40-testing-ci.md) 为唯一权威。

## 阶段与历史证据

```text
M0R → M1R → M2 → M3 → M4 → M5 → M6 → M7 → M8 → M9 → M10 → M11 → M12 → M13
|____________ Core-2.6 ____________| |____________ Parity-2.6 ____________|
```

- 前一里程碑 gate 未绿不得进入下一阶段；**M4→M5→M6→M7 不可绕过或并行冒进**。
- 旧 i686 内核与 C 用户态证据仅归档于 [01-current-state-audit.md](01-current-state-audit.md)，
  不计当前完成度。只可复用可移植 Rust 逻辑和测试方法，架构、ELF、用户 ABI 与证据必须重做。
- 每个 gate 须在同一干净 commit 上，按各冻结配置及对应 manifest 分别完整取证；
  同一配置内不得跨 commit、镜像或 suite 拼接，dirty、stale 与旧配置证据均无效。

## M0R — x86_64 长模式骨架｜Core

**交付**

- 固定已审计的 Rust 工具版本、`rust-toolchain.toml` 与 Cargo lock，构建均使用 `--locked`；
  按 [40 的 CORE-UP](40-testing-ci.md) 固定 QEMU patch 版本和包/镜像 digest。
- ELF64 高半区链接脚本、Multiboot1 头与 32 位保护模式入口；过渡入口建立四级页表，
  同时提供过渡所需低半区恒等映射和内核高半区映射，再进入长模式。
- M0R/M1R 内核 codegen 均固定 `-C force-frame-pointers=yes`，并静态验证帧指针未被省略，
  为 M1R 可验证栈回溯保留元数据。
- 基础 64 位 GDT/TSS/IDT、早期串口/VGA、早期 panic 与 Rust smoke/verdict。

**Gate**

1. 静态检查完整 Multiboot1 header 位于文件前 8192 字节内、起始偏移 4 字节对齐，
   magic 为 `0x1BADB002` 且 magic+flags+checksum 的 32 位和为 0；header 位于可加载段。
   ELF64 `e_entry` 必须是可用低地址空间内的 32 位保护模式过渡入口，不得指向高半区
   `rust_main`；所有由 Multiboot 装载的 `PT_LOAD`，其 LMA/`p_paddr`（含过渡代码和页表）
   均须位于可用低地址空间，高 VMA/`p_vaddr` 可保留。页表和跳转测试覆盖低恒等映射、
   高半区执行与 NX，并保存 header/ELF 检查命令及输出。
2. 真实 QEMU `-kernel` 直载完成启动，串口标记、完整用例、`isa-debug-exit` 预期退出码 33
   与 host verdict 同时通过；banner 或正常关机本身不算通过。
3. 若需 GRUB 回退，先新增 ADR 并在 40 冻结独立配置、工具链和镜像 hash；不得沿用
   `-kernel` 或旧配置证据。M0R 不提前要求 M1R 用户测试或全树 Rust-only，仅启用到期的
   host 项与 `test-qemu-smoke`。

## M1R — 64 位 xv6 等价与 Rust 用户态｜Core

**交付**

- 64 位 trap/context、PIT/PIC、fork/exec/exit/wait、IDE/xv6fs/pipe 迁移。
- M1R 在异常/陷阱路径承接 oops：受控 fault 输出寄存器、故障 RIP 与符号化栈回溯；
  沿用 M0R 的 force-frame-pointers 并验证调用链。这只是内部诊断，不引入后 2.6 ABI。
- syscall 入口切换至**本任务内核栈**，正确执行 swapgs，初始化 EFER/STAR/LSTAR/FMASK
  与 GS 相关 MSR，保存 RCX/R11/RFLAGS 等现场；仅 canonical 且安全的返回使用 `sysretq`，
  其余走 `iretq`，覆盖异常返回与 GS 窗口。
- eager FPU/SSE 保存恢复及 x87/SSE 控制状态；`UserPtr`/copyin/copyout 使用可恢复
  fault fixup，将非 canonical、越界、未映射和跨页失败按约定转为 `EFAULT`。
- 共享 `abi` crate；Rust no_std 用户 runtime、`_start` 与 brk allocator。内部用户程序由
  rust-lld 直链为静态非 PIE `ET_EXEC`，不经过 C driver、不带 crt/libc/libgcc；详见
  [04-rust-userspace.md](04-rust-userspace.md)。

**Gate**

- 只运行 M1R 及以前的到期 required Rust 用例，覆盖 syscall 返回路径、SSE 串扰、
  fault fixup、内部 ELF 布局与到期 xv6 语义；活动集为空、缺例、失败或 required skip 均为红。
- 独立 expected-fault 用例注入受控 #PF/#GP，核对 fault 类型、寄存器/RIP 和预期符号化
  调用链；意外 fault、寄存器不符或回溯失败均为红，该专项不得代替普通到期用例。
- mmap/COW 在 M2、SIGPIPE 在 M3 才激活；未来项记 `not-active`，不得记 skip-success。
- M1R 才启用完整 `test-qemu`、内部 ELF 检查与 `check-rust-only`；旧 C/usertests 不替代本 gate。

## M2 — 物理直映 / vmalloc / 匿名内存与 OOM｜Core

**交付**：bootmem、zones/buddy、SLUB/kmalloc；完整物理直映及 phys/virt 转换，vmalloc/vmap
的非连续物理页映射、权限与释放；匿名 VMA、brk、匿名 mmap/munmap/mprotect 最小集、
栈增长、COW fork；无 swap 的匿名 OOM 选择与统计。

**Gate**：host 验证分配守恒和 VMA 分裂合并；QEMU 验证直映 phys↔virt 往返及边界、
直映权限和 usable/reserved 区（预留页不得进入 buddy 分配），并验证 vmalloc 非连续页
连续映射、只读/NX 权限、释放后映射失效及物理页只归还一次。再验证 brk/mmap、NX/权限、
COW 页计数、fork+exec 压力及匿名 OOM，不误杀、不丢内存；M1R 全回归。

## M3 — UP CFS / 信号 / 内核基础｜Core

**交付**：UP PIT/PIC 参考路径、CFS 与 FIFO/RR 基础、waitqueue/kthread/workqueue、
softirq/timer/hrtimer/RCU UP；核心信号与 64 位 sigframe，管道写端收到 `EPIPE` 与 `SIGPIPE`。
内核抢占保持配置关闭并显式审计。

**Gate**：CFS 确定性模型、RT、定时器、信号帧/恢复、EPIPE/SIGPIPE 与 RCU 用例通过；
QEMU 公平性采用 guest-time 和预声明容差；M1R–M2 全回归。

## M4 — VFS / page cache / BIO / 最小设备语义｜Core

**交付**

- VFS 四 ops、dcache/icache、proc/sysfs 基础；file-backed mmap 与 page cache 共用页生命周期。
- active/inactive LRU：干净文件页可回收，脏页写回成功后才释放；无 swap 时匿名页和仍含
  唯一数据的 tmpfs 页不得以回收为名丢弃。交付最小 tmpfs 与统一 OOM 统计。
- 单用户 DAC mode 与基础 caps；最小 tty 回显、退格、EOF、TCGETS、winsize。
- BIO/request/电梯基础、IDE 中断路径和真实 CACHE FLUSH，传播完成、超时与 I/O 错误。
- cdev/bdev 主次设备号注册、驱动/设备绑定、按设备号进行 file ops 分发，以及 sysfs
  driver↔device 链接；字符 `/dev` 打开与块设备到 BIO 的路径均接通。

**Gate**：VFS 路径、file-backed mmap、tmpfs 生命周期、LRU 干净/脏回收及无 swap 压力通过；
mode/caps 允许与拒绝、最小 tty 逐项用例通过；BIO 并发和 flush 顺序/错误注入通过；
主次设备号注册与冲突拒绝、驱动/设备绑定及 sysfs 双向链接正确，字符 `/dev` open 按
设备号路由到绑定 file ops，块设备 open/read/write 经 bdev 到达 BIO；M1R–M3 全回归。
完整 credentials/LSM、pty 和 Parity FileOps 不在此提前扩张。

## M5 — ext4 只读｜Core

**交付**：按 [30-ext4.md](30-ext4.md) P1 实现特性白名单、extent、线性/htree 目录与
symlink。自 M5 起启用 host 管线，以固定 mke2fs/debugfs 输入创建独立第二盘 ext4 RO 模板，
保持 xv6 root；`dumpe2fs` 核对实际 features、块大小和 geometry。

**Gate**：guest 与 host 生成同字段、稳定排序的黄金清单，规范化路径/属性、链接与内容
SHA-256 完全一致；1k/2k/4k、htree、未知特性拒绝和只读写拒绝通过，离线 e2fsck 闭环有效。

## M6 — ext4 无日志读写｜Core

**交付**：始终保持 xv6 root，仅将独立 ext4 第二盘的未挂载 `^has_journal` 副本以 RW
开始测试；实现分配、extent、目录、rename/truncate 等。已索引目录必须正确 split 并更新 dx，
或完整重建为线性目录并清除 INDEX_FL/dx；大文件同步维护 64 位 `i_size`、`i_blocks` 与
huge_file 元数据。fsync 等待相关 BIO 和真实 flush。

**Gate**：大目录 split/重建、大文件/截断与高位字段、分配回滚、真实 flush 错误注入通过；
guest sync 并干净卸载后，host 严格离线 `e2fsck -fn` 退出 0，且内容 hash、目录/链接语义和
操作计数一致。不得以 `noload`、已有 M5 镜像或非零 fsck 代替本 gate。

## M7 — jbd2 ordered / replay｜Core

**交付**：在独立 `has_journal` 副本上实现事务、ordered、commit/checkpoint/replay/revoke；
数据、日志与 CACHE FLUSH 的持久化顺序符合 30，未知 journal feature 明确拒绝。

**Gate**：在 UP 参考配置 `CORE-UP-EXT4-CRASH` 完成 **200 次有效写中 crash**，执行
`make test-ext4-crash CONFIG_ID=CORE-UP-EXT4-CRASH`。按 30 §5 预声明的唯一主类别分别要求
`needs_recovery/nonempty-replay=40`、`revoke=30`、`commit-before=20`、
`commit-after=20`、`flush=20`、`abort=20`、`checkpoint=25`、
`journal-reuse-wrap=25`；各类 expected=actual 且均非零，合计恰为 200。每个计数样本
均须留下真实 `needs_recovery` 与非空 replay 记录，空 replay 或无效注入不计数；逐轮记录
config_id、完整命令、seed/hash、类别、操作序列及 expected/actual 计数。每轮都在刚崩溃的
同一镜像重启 replay，完成业务语义校验并干净卸载，QEMU 停止后严格离线 e2fsck 为 0，
第二轮 debug-exit/verdict 也通过。不得重建镜像绕过 replay；仅启动成功或结构无错不算通过。

## M8 — SMP / clone/TID/futex 线程基础 / IPC / Core 收尾｜Core

**交付**

- LAPIC/IOAPIC/IPI、AP trampoline；每个 AP 完整初始化 GDT/TSS/IDT、独立 idle/内核栈、
  per-cpu、FPU/SSE、syscall MSR、GS base/swapgs，而非只复制 BSP 临时状态。
- per-cpu CFS、TLB shootdown、RCU SMP；clone 必要标志、TID/futex 基础、SysV IPC/FIFO、
  select/poll 与基础 epoll；tty 前台组及 ^C/^Z 基础。
- Core 网络仅为 Stretch，不进入 M8 gate；可选网络结果不得冲抵 Core required 项。

**Gate**：按 40 将 `{1,2,4}` vCPU 与 `{EXT4-RO,EXT4-RW,EXT4-CRASH}` 展开为
`CORE-SMP{1,2,4}-EXT4-{RO,RW,CRASH}` 九个唯一复合 config_id；M8 manifest 的九项
均为 required，各自 expected/actual 用例数非零且相等，各自保留完整 QEMU argv、镜像、
suite/run 与 evidence ID/path/result，不得共用一个 SMP ID 的结果。非磁盘及其他 Core
required 项也逐 case 绑定明确的 SMP/复合配置，由 `verify-core` 反查 00，不得静默漏项。

逐 AP 验证 trap/syscall/FPU/GS；RO 配置执行 M5 只读用例，RW 配置执行 M6 无日志 RW
与干净卸载，CRASH 配置执行适用的 M7 jbd2 非崩溃用例及 SMP crash 专项；三者均运行
manifest 分配的跨 CPU VFS/page cache/BIO、select/poll/基础 epoll、tty 前台组信号用例。
RW 在卸载后做严格 fsck 与语义对账。三个 CRASH 配置各运行 manifest 冻结的**非空 SMP
crash 专项**，逐类别核对 expected/actual，并完成 40 的 replay/卸载/fsck/语义闭环。最终 Core 同时需要
M7 的 UP 200/200 完整活动和 M8 的三个 SMP crash 专项证据，二者不得互相替代或重复计数。
并发 fault/unmap/writeback/reclaim/OOM 审计无 UAF、旧映射、双重释放或统计失衡。

## M9 入口 — 可再生成权威差距表与 Parity 配置

开始 M9 实现前，必须用可重跑的生成器冻结并保存：

1. Linux 2.6.39.4 权威源码的 tag、完整 commit 与源码归档 hash，x86_64 syscall 表、
   实际使用的 UAPI 头及配置输入；记录生成器版本、输入 hash 和确定性输出。
2. 差距表覆盖 syscall/flags/结构体、auxv、ioctl、proc/sysfs/sysctl、netlink、mount 与
   文件系统 features。每行均含实现状态与证据、计划里程碑、配置排除理由、事先 ADR
   out-of-scope 引用及测试 ID/结果；不适用字段显式标记，配置排除须有冻结配置证据。
3. **只冻结 Parity 扩展配置**，Core QEMU/测试配置仍沿用 M0R 起冻结的 40；不得借 M9
   改写 Core 证据。记录缺口只是建账，不等于该差距已关闭或完成。

## M9+ — Parity-2.6 差距关闭

| 里程碑 | 范围与前置 | Gate |
|---|---|---|
| M9 ABI/静态外部验证 | 先使用固定来源、版本和 hash 的静态外部 glibc/LTP fixtures；关闭清单分配的线程、RT 信号、vDSO/vsyscall 等差距。mmap/socket 在此只冻结 ABI 清单与布局，不宣称 M10/M12 语义完成 | ABI 差分与静态外部黑盒矩阵全绿，Core 仍绿 |
| M10 MM/VFS/Block/FS | 按清单关闭 swap/完整回写等差距；明确交付原生 AIO（io_*）、sendfile/splice/tee/vmsplice、inotify/fanotify/dnotify、eventfd/signalfd/timerfd 与完整 epoll | LTP mm/fs/io、通知/零拷贝/完整 epoll、故障压力及 ext4/jbd2 约定项全绿 |
| M11 进程/平台/完整 tty/发行版 | 关闭清单分配的进程语义、kmod/initramfs、设备模型、完整 termios/pty/job control；若使用动态 rootfs，先实现 ET_DYN/PT_INTERP 映射及含 AT_BASE 的 loader auxv | 模块/设备/tty 与发行版启动、黑盒用例全绿 |
| M12 网络/安全/容器 | 关闭约定网络栈与 socket 语义、netlink/netfilter，以及完整 credentials/capabilities/LSM 和清单内 ns/cgroup | 派生网络配置互操作、安全与容器矩阵全绿，Core 默认配置不被改写 |
| M13 稳定性与终验 | 冻结范围 soak/crash/故障注入、性能基线和全量范围审计，不以文档化未实现项充绿 | 完整 Core+External Parity 重跑及 `verify-parity` 全绿 |

其余 `*at`、vectored IO、xattr、TLS/arch_prctl、timer/rlimit/sched、ptrace、NUMA
config-off 等均由 M9 差距表分配里程碑、配置证据或事先 ADR，不在此无限扩写。
后 2.6 的 ext4 扩展不自动成为 Parity-2.6.39.4 必备项，只有显式立项并冻结扩展配置才验收。

## 完成口径（严格遵循 [40 §6](40-testing-ci.md)）

- **Core done**：M0R–M8 所有 required 项在同一目标上证据有效且 Core 矩阵全绿；
  不需要外部 glibc、LTP 或 rootfs。
- **Parity done**：Core 仍全绿，M9–M13 冻结配置的 in-scope 项全部关闭且 External Parity
  全绿；任何 in-scope FAIL/SKIP/TCONF/XFAIL 均阻断。out-of-scope 只能由失败前已接受的 ADR
  冻结范围、理由和替代测试，不能事后豁免；单次演示或文档记录不等于完成。
