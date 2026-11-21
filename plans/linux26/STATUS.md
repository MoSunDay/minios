# STATUS — x86_64 / Linux 2.6.39.4 实施状态

> 本文件只报告实施与证据状态。**计划文档终审完成不等于实现完成，也不使任何 gate 变绿。**
> x86_64/Rust-only 迁移已启动：M0R 已在净树本地取证与 GitHub CI 首跑双重全绿（见下）；
> 验收规则以 [40-testing-ci.md](40-testing-ci.md) 为唯一权威。

## Rollup

| 范围 | 当前状态 | 判定 |
|------|----------|------|
| Core-2.6（M0R–M8） | 🟢 M0R–M1R done（M2–M8 pending） | M0R 净树取证全绿（HEAD `4c22b7b`）+ CI 首跑 success；M1R 净树电池 9 步全绿（HEAD `ef2a335`，serial 钉值全部复现）+ CI run 44381221846 success；证据见下文 M0R/M1R 现状 |
| Parity-2.6（M9+） | ⬜ **pending** | 未启动；权威差距表、Parity 配置和外部矩阵均未验收 |

最近同步：2026-11-19。唯一实现基线为 Linux 2.6.39.4 / x86_64；不沿用旧完成度或工期。

## 里程碑状态

| 里程碑 | 交付摘要 | 状态 | 当前证据/前置 |
|--------|----------|------|---------------|
| M0R | 工具锁、Multiboot1/ELF64、长模式、高半区、真实 QEMU smoke、CI workflow | 🟢 **done**（2026-11-16 判绿） | 净树 HEAD `4c22b7b` 全套 exit 0 + 三组 sha256 + CI 首跑 success：https://github.com/MoSunDay/minios/actions/runs/43886738739 |
| M1R | syscall/FPU/UserPtr、静态 Rust ET_EXEC 用户态与到期 required 用例 | 🟢 **done**（2026-11-19 判绿） | 净树 HEAD `ef2a335` 电池 9 步 EXIT0（smoke/m1r-suite/m1r-user 35+coverage/m1r-fault/demo-full，serial 钉值 `028048dd…`/`6137aefd…`/`0c39a3e5…`/`1ba8b13a…` 全复现）；CI run 44381221846 success；详见文末 M1R 判绿收口 |
| M2 | 匿名 VMA/brk/mmap/COW 与匿名 OOM | 🟡 **物理页基础已交付**（m2r-s1/s2，2026-11-19） | bootmem/buddy(双 zone)/directmap/SLUB/vmalloc+ioremap + m2 内核套件 9/9（`make test-qemu-m2`，exit 33）；host 26 例纯核单测；CI 接线；证据见文末 m2r 记录；匿名 VMA/brk/mmap/COW 待做 |
| M3 | UP PIT/PIC、CFS、信号、EPIPE/SIGPIPE | 🟡 **调度/时间/底半部已交付**（m3r-s1–s5，2026-11-20） | CFS+RT+waitqueue+时间层+kthread/workqueue/softirq 逐段电池全绿（m3-suite 16/16，`make test-qemu-m3`）；信号（s6）与 RCU/验收（s7）待做；证据见文末 m3r 记录 |
| M4 | VFS/page cache/file-backed mmap、回收、tmpfs、tty、BIO/flush | ⬜ pending | 等 M3 全绿 |
| M5 | 第二盘 ext4 RO 与 host/guest 黄金清单 | ⬜ pending | 等 M4 全绿；M5 起启用 host ext4 管线 |
| M6 | 独立 `^has_journal` 副本 RW、htree/大文件/fsync | ⬜ pending | 等 M5 全绿 |
| M7 | `has_journal` jbd2 ordered/replay 与 200 次有效 crash | ⬜ pending | 等 M6 全绿 |
| M8 | SMP、clone/TID/futex 线程基础、IPC、完整 Core 回归 | ⬜ pending | 等 M7 全绿；1/2/4 vCPU 均必验 |
| M9+ | 权威差距表及 Parity ABI/MM/平台/网络/安全/稳定性 | ⬜ pending | M9 前冻结来源、清单与 Parity 扩展配置 |

M4→M5→M6→M7 不可绕过。M0R 不提前要求 M1R 用户测试或全树 Rust-only；M1R 才启用完整
`test-qemu`、内部 ELF 检查与 `check-rust-only`，未来用例只能记 `not-active`。

## 证据有效性

- 旧 i686/C 的构建、banner、sh、usertests 和 host 结果仅作归档与迁移参照，不计当前完成度。
- 每个 gate 必须在**同一干净 commit** 上，按每个冻结配置及对应 manifest 分别取得完整证据；
  同一配置内不得跨 commit、镜像或 suite 拼接通过结论，多配置矩阵不得互相替代。
- dirty tree 仅供调试；dirty、stale、旧配置、旧镜像或缺失原始结果的证据均不计入 rollup。
  实现、测试、runner、ABI 清单、工具链或输入变化后，按 40 标 stale 并重跑受影响矩阵。
- Core 与 External Parity 分开汇总。Parity 验收前必须重验 Core；in-scope 失败、skip、TCONF
  或 XFAIL 均阻断，out-of-scope 只能引用事先接受的 ADR。

## M0R 现状（2026-11-16，净树取证 + CI 首跑双绿，已判 🟢）

实现与冒烟（正式净树证据见下两节）：

1. 工具链锁定 Rust `1.97.1`（`rust-toolchain.toml`），`x86_64-unknown-none` 已安装，
   构建使用 rust-lld 与 `kernel/x86_64-minios.ld`（ELF64 低 boot block + 高半区）。
2. 静态 gate：Rust `minios-runner check-elf` 校验 Multiboot1 header（文件前 8192 字节、
   4 字节对齐、magic/flags/checksum 和为 0）、ELF64 `ET_EXEC`、低 32 位过渡入口、
   全部 `PT_LOAD` 低 `p_paddr`、存在高半区 VMA；`grub-file --is-x86-multiboot` 亦通过。
3. 真实启动：QEMU 6.2 `-kernel` 不能载 ELF64（ADR-0014），走冻结的 `CORE-UP-GRUB`
   ISO 回退；串口六条 CASE 全 PASS、唯一成功标记、退出码 33，Rust runner/verdict
   判 PASS；verdict 含 40 §2.2 要求的全部故障自测（22 项 host 测试）。
4. `make test-qemu-smoke`/`make m0r-iso`/`make check-layout`/`make test` 已接入
   x86_64 默认路径；early config 固化于 `scripts/m0r-grub-early.cfg`。

待办（阻断 M0R 判绿）：

1. ~~在干净 commit 上重跑全套命令产出正式证据~~ → **已完成**，见下方净树取证记录
   （2026-11-16，HEAD `4c22b7b`，全套退出 0）。
2. ~~CI 首次绿~~ → **已完成**：run #1（push 事件，HEAD `d045142`）结论 success，
   12 个业务步骤 + 上传证据 artifact 全绿：
   https://github.com/MoSunDay/minios/actions/runs/43886738739

### CI 首跑取证记录（2026-11-16，判绿依据之二）

- Run：`core-ci` #1，event=push，HEAD `d045142b79f61716d4000a2c480ff389df7b287f`，
  runner `ubuntu-22.04`，结论 **success**（全部步骤含 Post/Complete 均 success）。
- URL：https://github.com/MoSunDay/minios/actions/runs/43886738739
- 关键步骤在线验证：
  - `Guard: rust-toolchain.toml still pinned to 1.97.1` 与
    `Rust toolchain (pinned 1.97.1, matching rust-toolchain.toml)` 双 success
    ——评审 P0 缺陷（缺 `with:`）修复被线上证实；
  - 冻结快照 apt 安装（四包精确 `=版本` 钉死）success——假设 2（快照 deb 可下载）闭环；
  - `make all`/`make test`/`make test-qemu-smoke` 在 CI 复现本地同判绿（exit 33 +
    协议 verdict PASS）——假设 3（CI target 目录）闭环；`ubuntu-22.04` 可用——假设 1 闭环；
  - kernel ELF/ISO/serial.log/verdict 已作为 artifact `m0r-evidence` 上传（30 天保留）。
- 终态复核：判绿 commit `04c85b7`（判绿后的最终推送态）再触发 run #2，结论亦为
  success（含新增提交过双扫描）：https://github.com/MoSunDay/minios/actions/runs/43887412443
- 判绿合成：净树本地证据（下节）+ CI 外部环境复现 → 40-testing-ci.md 要求的双重环境
  一致取证成立，**M0R 判 🟢**。后续 docs/hardening 轮的 CI 复核以 Actions 历史为准，
  不再逐轮回写本节（避免文档-触发无限递归）。

### 净树取证记录（2026-11-16，正式证据）

- HEAD：`4c22b7b3ab6118a8d13f19397d42272c0fe1aa92`（`git status --porcelain` 输出为空，
  取证当日无 dirty 文件）。
- 命令链（全部退出 0，按 40-testing-ci.md §3/§4 到期集合）：
  `CARGO_HOME=/tmp/minios-cargo-home make fmt-check && make all && make test &&
  make test-qemu-smoke && scripts/check-no-secrets.sh && scripts/check-no-binaries.sh`。
- 协议行：`MINIOS_X86_64_TEST_PASS_V1 suite=m0r run=core-up total=6 passed=6 failed=0
  skipped=0`；`exit_code=Some(33) timed_out=false`；
  `verdict: PASS suite=m0r run=core-up config_id=CORE-UP-GRUB`。
- sha256（`sha256sum` 原样输出；kernel 与此前 dirty 轮一致，符合 40 §1.1 内核不变则
  hash 不变规则；ISO 含 mkisofs 时间戳、逐轮记录）：
  - `8aacf344361b0eab84a26787882950f50c1fb8bf1f4b3113ca96b4a1a26ac5cd  target/x86_64-unknown-none/release/kernel`
  - `1c65e9ba541e756355ef2f2f765d5924f6022660526b65691fa85f30455e1a4e  target/m0r-iso/minios-m0r.iso`
  - `71f9cf197d093eb63fe48fcad93277099e2617b6a20e436eff8ff2f8bd605adf  target/m0r-run/serial.log`
- 本轮修复：`scripts/check-no-binaries.sh` 文本扩展白名单补 `cfg`（首次入库的
  `scripts/m0r-grub-early.cfg` 为 ASCII 文本，原白名单缺口误报）；`.github/workflows/ci.yml`
  工具链步骤补 `with: toolchain/components/targets` + rust-toolchain.toml 漂移守卫
  （评审 P0 缺陷，dtolnay action 不读 toml 且 `toolchain` 为必填输入）。

2026-11-16 本轮增量（工具锁与 CI 接线，均已在本地验证）：

- 工具锁清单补齐（40 §1.1）：QEMU/GRUB/genisoimage/grub-pc-bin 的本地二进制 sha256
  与包版本；CI 侧 `snapshot.ubuntu.com/ubuntu/20261116T000000Z/` 冻结 + 精确版本 +
  deb sha256，快照版本与本地审计逐项一致。
- 新增 `.github/workflows/ci.yml`：全部 `uses` 固定 commit SHA（api.github.com 审计），
  apt 冻结快照装精确版本，跑 fmt-check、双扫描脚本、`make all/test/test-qemu-smoke`，
  上传内核 ELF/ISO/串口日志证据。
- runner 增加 `run-smoke --work-dir`，`make test-qemu-smoke` 将证据固化到
  `<target>/m0r-run/` 并打印 serial.log sha256。
- `make test-host` 改为 `--locked` + host 侧 `RUSTFLAGS='-D warnings'`；内核
  rustflags 亦加 `-D warnings`（.cargo/config.toml）；新增 `make fmt-check`；
  rust-toolchain.toml 显式声明 `x86_64-unknown-none` target。
- 修复史：boot.S 页表循环三处缺陷（`_pte_flags` clobber ECX、PTE 丢物理地址、
  page_dir 基址被覆盖）、高半区曾指向恒等表、TSS 描述符错位；均已在当前构建修正。

M1R 的内部 rust-lld 直链、abi crate、brk allocator、FPU/SSE 与 UserPtr fault fixup 等均是
后续交付，不因方案已定型而标记完成。commit、push、部署和历史改写仍需单独明确授权。

2026-11-17 本轮增量（M1R-S2：trap/fixup/timer 运行时 + 内核态双套件，均已在本地验证）：

- 交付 `m1r-suite`（4 用例：m1r.irq.pit / uaccess.copyin_efault / uaccess.copyout_efault /
  ksym.self，PASS exit 33）与 `m1r-fault`（5 注入，PASS exit 35，FATAL 37），Makefile
  `m1r-iso`/`m1r-fault-iso`/`test-qemu`/`test-expected-fault`，runner verdict 拆分
  （verdict/{mod,m0r,m1r/{mod,fault,tests}}），config_id=CORE-UP-GRUB-R1（两 m1r 目标
  argv 逐字节一致）。
- 证据（`verdict: PASS` + sha256）：
  - `1d33da59cf22185baa617b499fb80223f13a209b1936d2bae4218f32f3084176  target/m1r-suite-run/serial.log`（exit 33，total=4 passed=4）
  - `5ca25fdb7e890dbfcae5ade0458e7ee72d13873bed0e430ab6357f6e93ad6a0c  target/m1r-fault-run/serial.log`（exit 35，total=5 passed=5）
  - M0R 回归：`71f9cf197d093eb63fe48fcad93277099e2617b6a20e436eff8ff2f8bd605adf  target/m0r-run/serial.log` 字节不变；`make test`（58 例）与 fmt-check 全绿。
- 本轮根因修复（QEMU 取证 `-d int`）：
  - CPL0 同特权 `iretq` 只弹 rip/cs/rflags：`.Lreturn_iret` 现将三槽尾上滑越过合成
    rsp/ss，恢复后 rsp 精确等于 trap 时值（首个内核态定时器 IRQ 曾因 rsp-16 在被中断
    函数尾声 `ret` 到垃圾地址 → #UD oops）。
  - boot.S 开启 CR0.WP（PG|WP）使 `_pte_flags` 的 rodata RO 真正生效；boot GDT 迁至
    .boot.data（CPU 段寄存器加载会 RMW 描述符 A 位，GDT 页必须可写）。
  - D-2 偏差记录：QEMU 6.2 TCG 对未知 MSR 的 WRMSR/RDMSR 均静默忽略（`-d int` 零
    #GP 实证），`gp.wrmsr` 不可达；改为 `m1rfault.gp.noncanon`（经 MAGIC_RAX
    0xcafe… 非规范地址取数 → #GP(0)，TCG/真机确定），kind/ec/魔数寄存器契约不变。
  - ksym 前缀符号化：m1rfault.S 头部加 `_m1rf_prologue` 垫片，避免首个注入符号与
    KERNEL_VMA 同址导致 rip 降级符号化；驱动帧 black_box 阻断 LTO 尾跳，backtrace
    头稳定为 `_m1r_fault_driver_frame+`。
  - verdict 侧：MAGIC_RSI 补齐位数（原少一组 0）、KERNEL_CS 更正 0x08（GDT code=8）、
    M1RFault 行解析容忍 GRUB 横幅残留的前导 CR（与 m0r 判定同一工件）。

2026-11-18 闭环增量（M1R-S2 review 硬化，本地全绿后提交）：

- fork/exit 前置修复（S3 点亮用户链前的隐患）：exit() 现按 xv6 语义把孤儿子进程
  reparent 到 INITPROC（userinit 成功时登记；无 init 的内核态引导下孤儿退化为
  不可回收 zombie，绝不保留可复用槽位的悬垂 parent 指针）；fork() 在父/子任一
  tf 为空时干净失败（-1），不再静默调度 rip=0 的零帧。
- uaccess 收紧：新增 USER_TOP=2^47 预检——copyin/copyout/probe 对内核半区指针
  直接 EFAULT（不触碰任何字节），copyinstr 以 USER_TOP-src 封顶扫描（内核半区
  已映射，靠 cap 而非缺页兜底）。m1r-suite 增第 5 用例 m1r.uaccess.kernelptr_efault
  （对 m1r_force_iret 地址 copyin+copyout 均须 EFAULT），runner 清单与自测夹具同步。
- runner 硬化：verdict/mod.rs 抽出共享 base_outcome_errs/scan_forbidden（judge 与
  judge_fault 复用，消除字面量漂移面）；judge_fault 新增「PASS 标记后不得再出现
  M1RFault 行」检查；cr2==rdi 交叉校验改用 req（缺 cr2 即红）；errs sort+dedup
  去除合成判定的重复理由。main.rs 每轮落盘 qemu.version（40 §1.1），临时目录
  命名按 suite 标签，注释改为「两 M1R 目标仅 -cdrom 不同」。
- 每轮独立 root 盘（40 §2.2）：Makefile 改为一次性生成 root-template.raw，各
  target 每轮 rm -rf 后复制进各自 run 目录再挂 IDE，serial+root 双 sha 记账。
- 小修三连：trapcommon.S 帧复制 21→20 qword（精确帧，消除对 trap-time rsp 的
  越界读）；oops kernel_fault/user_fault 统一走 vector_kind()（DE/BP 不再降级
  EX）；fixup::table() 边界检查移到减法前（dev overflow-checks 下防御有效）。
- 死树清理：删除未被 kernel/mod.rs 声明的 kernel/src/kernel/sched/（整目录，
  grep 影子 proc/fork）与 kernel/src/usercall/（影子 syscall/sysarg）。
- 证据（verdict: PASS）：m1r-suite `bd23c5bd…38d4`（5 用例，exit 33，root
  `30e14955…cb58`）；m1r-fault `53b8aac7…5325`（exit 35——与 S2 `5ca25fdb…`
  的差异仅为链接布局位移：_m1rf_rodata_target 0x…8000→0x…9000、crate
  disambiguator 与深层帧偏移；5 条注入的 rip/backtrace 头偏移逐字节不变，D-3
  前缀契约不受影响）；M0R `71f9cf19…5adf` 字节不变；make test 58 例、fmt-check、
  build、check-layout 全绿。

2026-11-18 S3 闭环（用户链点亮：abi/user 两个新 crate + 内核 proc/VM/FPU 运行时，
本地全绿后提交）：

- 新增 abi/：Linux 2.6.39.4 x86-64 syscall 号与共享 ABI 常量（kernel/user/runner
  三方单一来源）。新增 user/：no_std 静态非 PIE ET_EXEC（rust-lld + 专用链接脚本，
  .text@0x400000 起），自带 memset/memcpy/memmove/memcmp，m1r-suite 8 用例
  （write/getpid_fork/fork_wait/execve/execve_enoent/brk/sse_crosstalk/syscall_regs）
  + m1r-exec-child（exec 替身镜像，打印 replaced image 后 exit）。
- 内核：syscall.rs 分发表（write/brk/fork/getpid/execve/exit/wait4/kill，其余
  ENOSYS）；proc.rs fork/wait4（wstatus=(xstate&0xff)<<8）/exit reparent/初始用户
  进程 userinit_named；exec.rs 通用 ELF 装载器（逐段拷贝逐字节校验）；sched.rs
  CPL0→CPL3 首切与 FPU 惰性 save/restore（XRSTOR 状态机，SSE 串扰用例守护）；
  cpu.rs TSS 改 packed（RSP0@offset 4）；userimg.rs + build.rs 构建期把 user ELF
  暂存进内核镜像（MINIOS_USER_IMG_DIR→OUT_DIR/userimg，rerun-if-changed）。
- 套件合并协议：用户态退出码 (total<<4)|failed，内核 proc::exit 在 initproc 收割
  后由 suite::finish_driver 合并打点——单一 PASS 标记 total=13（5 内核 + 8 用户）。
- 排障沉淀（case_brk 根因）：用户 8 字节 store 跨入未映射页（base+0x1fff 起的
  qword 跨 0x406000 页界，#PF CR2 指向缺失页首而非指令基址）——内核 brk/grow/
  mappages/sysretq 全程无辜（gdb 断点+页表行走证实）；用例改触 base+0x1ff8（页内
  最后一个完整 qword）。教训入 40 §5 备注。
- runner：verdict/m1r 清单扩至 13 用例（fixture 含用户 CASE 行与 total=13）；
  新增 userelf.rs（ISO 内嵌 user ELF 的构建期一致性校验）；panic 双前缀 `:0x0x`
  修正。
- 证据（verdict: PASS，exit 33）：m1r-suite `4d08bbf5…cacbe`（total=13 passed=13，
  root `30e14955…cb58`）；M0R `028048dd…92f1`；m1r-fault `5aa34926…7af2`
  （exit 35）；make test-host 58 例、fmt-check、make all 全绿。

2026-11-19 S4a/S4b 闭环（mkfs + 块层上线）：

- S4a Rust mkfs（minios-tools）：tools/ 恢复为 Rust host crate（bin minios-mkfs +
  纯函数引擎 build_image），与 C tools/mkfs.c 在相同输入下**逐字节一致**（实证
  cmp 通过）；根模板从 1MiB 零盘换成真实 xv6 root（1024 块、嵌 m1r-suite/
  m1r-exec-child 两个用户 ELF）。引擎单测 5 例（几何铺盘/位图/间接块/确定性/
  超长名拒绝）进 test-host。
- S4b 内核块层：fs/buffer.rs（bio.c 移植：BCACHE 单锁 + BUSY/VALID/DIRTY 旗标
  + LRU 链，bget 睡后重扫）与 drivers/block/ide.rs（ide.c PIO 移植，偏差记录：
  minios 根盘=drive 0，非 xv6 的 disk1；idewait 有界自旋防仿真卡死；引导期无
  进程上下文时 iderw 用 sti/hlt 轮询代替 sleep）。x86_64 io 补 insl/outsl。
- 两个真实 bug 修复：①PIC 级联——pic::init 全掩码后 slave 线（8..15）永远到不了
  CPU，unmask 现在同时开 master line2（-d int + pic trace 定位）；②Buf.flags
  非原子读被 LTO 提升出轮询循环——中断写了内存但循环不重读，改 AtomicI32(Relaxed)
  （x86 TSO 下仅编译器屏障需求）。
- 套件：新增内核用例 m1r.fs.blockread（bread 读 superblock/root dinode/首块
  dirent 并逐字段核对，只读——root.raw sha 与模板一致即证据）。清单 14 用例。
- 证据（verdict: PASS，exit 33）：m1r-suite `b4d6fb39…12a9`（total=14，root
  `a37262fa…311a` == 模板）；M0R `028048dd…92f1` 字节不变；m1r-fault
  `9814a6d5…fa2e`（exit 35）；test-host 6 组、fmt、make all 绿。CI：S3
  `7e190a4`/S4a `4291272` 均 success。

2026-11-19 S4c 闭环（xv6fs 日志/ inode/namei 层上线）：

- 死树复活：kernel/src/fs/xv6fs 五文件从旧 i686 树就地移植到 x86_64 习惯——
  log.rs/inode.rs 的 sched::{acquire,release,sleep,RawLock} 死 API 换成
  klib::sync::Spinlock<()>（raw_lock/raw_release 对，配 proc::sleep）；log 的
  static mut LOG 全部经 logp() 原始指针访问；log_write 的 `flags |= B_DIRTY`
  改 AtomicI32::fetch_or（S4b 原子旗标延续）；readsb/balloc/bfree/bzero 的
  bread 补 unsafe 块；install_trans 头部拷贝改逐元素（避 implicit autoref）。
  layout::ROOTDEV 1→0（minios 根盘=IDE index 0，非 xv6 的 hdb）。namei 相对
  路径分支在 cwd 落地（S4d 文件表）前按 miss 路径返回 null。写路径
  （balloc/bmap/writei/dirlink/ialloc/stati）与 devsw 以 S4d 标注保留。
- 引导接线：M1rSuite 臂在 ideinit 后加 log::loginit(ROOTDEV)（读 sb、log 居盘尾
  size-nlog=994，恢复事务）+ iinit()。m1r.irq.pit 改快照语义——loginit 引导期
  IDE 轮询（sti/hlt）已合法吃掉首个 tick，用例改为等“新边沿”（死 PIT 仍挂死→
  超时证据，不弱化）。
- 套件：新增内核用例 m1r.fs.inode——namei("/m1r-suite")/("/m1r-exec-child") 解
  析→ilock→readi 头/尾各 64B 与 userimg 内嵌 ELF 逐字节比对（18224B=36 块>12
  直接块，覆盖 bmap 间接路径），/enoent 必须 miss。清单 15 用例。
- 证据（verdict: PASS，exit 33）：m1r-suite `d1ebd8c3…087c`（total=15 passed=15，
  root `a37262fa…311a` == 模板——loginit 恢复重写的头块字节相同，只读性保持）；
  M0R `028048dd…92f1` 字节不变；m1r-fault exit 35；test-host 6 组（runner 44 例）、
  fmt、make all 绿。

2026-11-19 S4d 闭环（文件表/管道/控制台 + 文件系统调用）：

- file.c/pipe.c 移植：fs/file_table.rs（FileState 枚举 Closed/Inode{ip,off}/
  Pipe{pipe,writable} 代替 C tag+union；FTABLE 静态+Spinlock 对；filewrite 按
  LOGSIZE 分块事务）；ipc/pipe.rs（data[512] 环形缓冲，kalloc 块内嵌 Spinlock，
  proc::sleep 通道取 addr_of!(nread/nwrite)，pipeclose drop_in_place+kfree）。
  偏离（文件头已注）：fileread/filewrite 取内核缓冲——sys 层经 uaccess
  copyin/copyout 弹跳，坏用户指针得 EFAULT 而非 readi/writei 内fault；xv6 一律
  -1 改 Linux errno（EBADF/EMFILE/ENOENT/EEXIST/EISDIR/…）。
- sys 层拆两文件：kernel/syspath.rs（copy_path 128B 弹跳、create、
  sys_open/sys_mknod/sys_mkdir/sys_chdir/isdirempty/sys_unlink/sys_link，全部
  begin_op/end_op 包裹）+ kernel/sysfile.rs（sys_read/sys_write 512B 弹跳、
  sys_close/sys_dup/sys_fstat/sys_pipe/fdalloc）。abi 新增 SYS_FSTAT/DUP/CHDIR/
  MKDIR/LINK/UNLINK/MKNOD 与 Linux O_* 旗标。
- 控制台：drivers/char/console.rs 精简版（COM1 RX 中断，INPUT 128B 环+chan；
  CTRL_D/U/H 行规程；devsw CONSOLE=1 注册；traps.rs IRQ_COM1=向量36）。
  mkfs 不建 /console——xv6 语义：init（本树=m1r-suite 驱动 main）先 open，ENOENT
  则 mknod(1,1)，再 dup 0×2 播种 stdin/stdout/stderr；此后 CASE 行经 fd1→file
  表→devsw CONSOLE 打印（对齐 xv6-public init.c，kbd.c 弃——headless QEMU）。
- proc：ofile[*mut File; NOFILE]/cwd: *mut Inode；exit 在 begin_op/end_op 内先
  关全部 fd + iput(cwd)；fork 经 filedup/idup 继承；userinit_named 播种
  cwd=iget(ROOTDEV,ROOTINO)。
- 套件：内核用例 m1r.fs.file（引导上下文 filealloc+FileState::Inode 按 sys_open
  同款所有权交接，fileread 两次共享偏移读比 userimg ELF、filestat/stati、
  fileclose 归零复用）；用户用例 m1r.user.open_read（open /m1r-suite 读 ELF 魔数
  +续读偏移 + 关闭后 EBADF）、m1r.user.pipe（双端往返 + EOF）、m1r.user.dup_close
  （dup 写/关后原句柄仍活）。清单 19 用例（registry+fixture 同步 19）。
- 证据（verdict: PASS，exit 33）：m1r-suite serial `4f412412…48d3`（total=19
  passed=19 failed=0）；运行盘 root `72e55134…c6b0`≠模板——mknod("/console")
  对每轮副本的合法改写（目录块+新 inode）；模板 `6d5aa94e…cd52`（随新 m1r-suite
  ELF 重建）；M0R `028048dd…92f1` 字节不变；m1r-fault `1fbae812…f967` exit 35；
  test-host 6 组（runner 44 例）、fmt、make all 绿。新文件均 ≤400 行。

2026-11-19 S4d-2 闭环（exec 改走磁盘）：

- proc/exec.rs 换流式字节源：exec_elf(p, tf, src: &dyn Fn(&mut [u8], u32) -> i32,
  fsize, argv)——ELF 头/程序头逐个读入栈缓冲，PT_LOAD 经 512B 弹跳块直写目标页
  （xv6 loadseg 语义，不再整文件内核缓冲）；parse_ehdr 的 phdr 表界改对 fsize；
  phdr_at→phdr_from_bytes（纯字节解码）。
- syscall.rs sys_execve：copy_path(128) + namei→ilock→readi 闭包→exec_elf→
  iunlockput（只读不进日志事务）；userimg 嵌入表退役为 pid-1 种子与逐字节比对
  参照（userinit_named 同经 reader 面喂平铺切片）。exec 路径缓冲 64B→MAXPATH 128。
- 套件：新增 m1r.user.execve_diskcopy——fd 级 512B 分块拷 /m1r-exec-child→
  /elfcopy（写路径首个真实使用），fork+execve("/elfcopy")（磁盘独有、不在内嵌表
  的路径），子映像打印标记并 exit 42 证明字节级等价；unlink 后 open 必须 ENOENT。
  清单 20 用例（registry+fixture 同步）。
- 证据（verdict: PASS，exit 33）：m1r-suite serial `6137aefd…e585`（total=20
  passed=20 failed=0；日志含第二行 replaced image=磁盘副本在跑）；运行盘 root
  `9e66abf1…4536`（mknod console + elfcopy 创建/删除痕迹）；M0R `028048dd…92f1`
  字节不变；m1r-fault `1fbae812…f967` exit 35；test-host 6 组（runner 44 例）、
  fmt、make all 绿。

2026-11-19 S4e-1 闭环（15 程序首批 14 个 + 用户堆 + nanosleep）：

- 运行时：lib.rs 注册 #[global_allocator]（alloc.rs BrkAlloc，单线程 Sync 论证
  在模块内）；sys.rs 补 link/mkdir/chdir/fstat/stat(open+fstat+close)/sleep/
  sbrk(brk 包装)；io.rs 补 gets（单字节行读）/print_i64_fd。abi 新增
  SYS_NANOSLEEP=35 与 #[repr(C)] Stat（kernel layout::Stat 改 re-export，用户态
  与内核同源解码）。
- 内核：sys_nanosleep（xv6 sys_sleep 循环：TICK_LOCK+ticks 通道，100Hz；EINVAL
  界检查）；time.rs tick() 在 TICK_LOCK 下唤醒 ticks 通道（xv6 wakeup(&ticks)）。
- 程序（xv6 语义，Rust no_std bins）：echo/cat/kill/mkdir/rm/ln/ls/wc/zombie/
  forktest(N=NPROC=64+ECHILD)/grep(K&P 匹配器)/stressfs(5 进程链 5 文件)/
  init(console 播种+sh 重启+孤儿收割)/sh（枚举 AST 递归下降：;|()<>>>、cd 内建、
  最低 fd 重定向、后台不 wait）。usertests 随下一阶段。
- mkfs：USER_ELFS 16 个 ELF（m1r-suite、m1r-exec-child + 14 程序；dirent 顺序=
  列表序）；根目录 18 项仍一块（layout 文件数无关性成立），m1r.fs.blockread 检查
  不变仅注释更新。
- 证据（verdict: PASS，exit 33）：m1r-suite serial `6137aefd…e585`（20/20 不变）；
  模板 `dff04779…4321`（16 ELF）；运行盘 `49d9ec0e…9860`；M0R `028048dd…92f1`
  字节不变；m1r-fault exit 35；test-host、fmt、make all 绿（user-build 0 警告）。
  sh.rs 376 行、ls.rs 122 行（≤400）。

2026-11-19 S4e-2 闭环（usertests 全量移植 35 例 + QEMU 轮 + 6 处内核缺陷修复）：

- 用户测试体 user/src/utest/（mod/mem/fs_basic/ipc/fs_names/fs_link/fs_big/
  exec/prog，各文件 ≤355 行）：35 例 = m1r_user_spec() 冻结清单——xv6.mem 2、
  xv6.fs 15（含 sharedfd/createdelete/twofiles/bigdir）、xv6.ipc 3、xv6.exec 1、
  xv6.prog 14（run_capture：fork+pipe 重定向+exec_argv+wait4 断言 stdout/退出码）。
  驱动 usertests.rs 以 (total<<8)|failed 编码退出；finish_driver 解码；用户态缺页
  → oops::user_fault → 套件 fatal（exit 37）语义保持。
- 内核修复（对照 link_repos/xv6 逐条）：①traps_dispatch killed 分支补 EOI（抢占
  死锁）；②growproc 提交 mm.sz（brk(0) 陈旧）；③sys_unlink 拒 "."/".."（EINVAL，
  rmdot 自锁）；④create() 收紧为 xv6-public 严格规则——仅 file-on-file 复用
  inode，mkdir/open(O_CREAT) 撞已有目录项一律 EEXIST（fourteen 依赖）；⑤
  USER_STACK_PAGES 2→8（bigwrite 8KB 栈帧溢出）；⑥**filewrite/fileread 共享偏移
  丢失更新**——原在 ilock 外快照 FileState.off、锁内回写整枚举，父/子共享同一
  File 时后写者以陈旧 off 覆盖（sharedfd 症状 nc=10000/np=0、两个 0 起点
  单调流）；改为锁内重读 off、锁窗内回写 off+r（xv6 `f->off += r` 语义）。
- 测试侧缺陷同步修复：createtest 两处 Ok-fd 未 close（fd3/fd4 泄漏随 fork 复
  制）；sharedfd 诊断序 close→fstat 改为 fstat→close。
- 装配：BootMode::M1rUser（userimg 以 usertests 为 pid-1 种子）；scripts/
  m1r-user-grub.cfg；Makefile m1r-user-iso/test-qemu-user；runner Suite::M1rUser
  + 35 例 spec + len==35 fixture。
- 证据（verdict: PASS，exit 33）：m1r-user serial `0c39a3e5…08d3`
  （total=35 passed=35 failed=0，35 个 CASE id 唯一）、root `a5a140d6…c947`；
  m1r-suite `6137aefd…e585` 字节不变、M0R `028048dd…92f1` 字节不变（证明 6 处
  内核修改不扰动既有两套）；test-host 47 例、fmt、make all（user-build 0 警告）绿。

2026-11-19 S4e-3 闭环（用户覆盖矩阵 coverage.toml + runner 派生门禁）：

- 输入清单 tests/user/coverage.toml（394 行，39 条）：35 条 M1R 到期 required
  （21 条 usertests 源 `xv6@2305a8c:usertests:*`，sbrk 标注 subset；12 条程序源
  `xv6@2305a8c:<prog>.c`）+ 4 条未来项（mmap/munmap/cow→M2、sigpipe→M3）——仅静态
  字段（case_id/source/milestone/config/mode/runner_test_name/requirement/adr）。
- runner 新增 coverage 模块（mod 343 行 + tests 91 行，零依赖）：TOML 子集解析
  （未知字段/重复/坏枚举/缺字段均硬错）、按里程碑推导 activation（M0R<M1R<M2…
  <M13）、evidence 仅绑定 active 用例（not-active 有结果即 fail）、手写 JSON 渲染
  派生结果文件 results/<run>/<CONFIG>.json（拒绝覆盖既有证据）。
- 门禁规则（40 §3.1）：active+required 为空、到期 required 缺结果/非 PASS、结果
  匹配不到清单用例均 fail；m1r-user 轮后 Makefile 链式执行，gate fail 即 make 失败。
  完整性交叉校验：清单 M1R 集 ≡ m1r_user_spec() 35 例（防漂移）。verdict 新增
  case_results()（复用 judge 的 CR 修剪语义）供 evidence 派生。
- 证据：m1r-user 35/35 exit 33 + `coverage: PASS run=m1r-user active=35
  not-active=4`；serial `0c39a3e5…08d3` / root `a5a140d6…c947` 与 S4e-2 完全一致
  （门禁纯 host 侧）；m1r-suite `6137aefd…e585`、M0R `028048dd…92f1` 字节不变；
  test-host 6 组（runner 51 例，含 coverage 门禁 4 例）、fmt、make all 绿。
- 备注：S4e-2 推送（33340a0）CI 失败于 snapshot.ubuntu.com 全站 401（外部事故，
  与仓库无关，snapshot 步骤早于任何构建）；本阶段推送即重触发，main tip 覆盖
  S4e-2+S4e-3 两阶段内容。check-rust-only 与遗留 C 删除一并入 S4e-4（先删后启，
  保持每提交可绿）。

2026-11-19 S4e-4 闭环（遗留 C 删除 + rust-only 门禁 + BootMode::Full 演示 + M1R CI 接线）：

- 删除：user 全部 C（19 .c + 11 头 + usys.S + legacy Makefile）、tools/mkfs.c 及 4 头、
  kernel 死代码（io.rs、legacy/i686_main.rs、arch/x86 i686 树、console/{serial,vga}.rs）
  与 user/_*、*.o、fs.img、tools/mkfs 产物；Makefile 去 legacy-user。保留
  user/x86_64-minios-user.ld（Rust 链接脚本）与 kernel/src/arch/x86_64/*.S（6 个
  冻结 asm 桩，白名单）。make all 删除后即绿（无任何引用残留）。
- check-rust-only（runner/src/rustonly.rs 399 行 + 6 单测）：扫 abi/kernel/user/
  tools/host-tests/tests 六根；违规=C 族扩展名（c/h/cc/cpp/cxx/c++/hpp/hh/hxx/i/ii）、
  白名单外 .S/.s、Makefile/scripts//.cargo/Cargo.toml 中 cc/gcc/clang/g++ 等编译器
  调用（basename 精确匹配 + -gcc/-cc 后缀）；缺根亦 fail。实树 PASS：
  roots=6 files=117 asm-allowlist=6 c-sources=0 compiler-invocations=0。
- BootMode::Full（S4e-4 演示）：main.rs Full 臂 init_m1r_fs + userimg 新增 /init 内嵌
  （build.rs 暂存第 4 个 ELF）+ proc::userinit → init(1) mknod /console → fork/exec
  /sh（真实 xv6fs 路径）。runner demo-full（demo.rs 400 行 + 5 单测）：QEMU 唯一
  偏差 -serial stdio（冻结 argv 断言钉死），读者线程镜像 serial.log，6 步会话
  boot→prompt→echo→ls（usertests 先于 console：mkfs 嵌入序，/console 为 init 启动后
  mknod 的最后 dirent）→CTRL-D→init 重启 sh；PASS 印 MINIOS_X86_64_DEMO_FULL_PASS_V1
  markers=6。serial `efa925a4…eb9a`。
- 内核修复 #7（本阶段第 7 处）：consoleintr 缺 EOI —— COM1 首次输入后 PIC in-service
  位悬挂，饿死包括 timer 在内的全部后续 IRQ（会话在第一条命令后冻结）；对齐
  ideintr/time::tick 的"各自应答"约定，eoi 置于 drain 前。
- CI（40 §4.1 M1R 行全部接线）：check-rust-only、test-qemu（m1r-suite）、
  test-qemu-user（35 例+coverage）、test-expected-fault、demo-full 五步插入
  ci.yml；job 更名 M1R core；evidence 上传扩至四 run 目录 + results/m1r-user。
- 证据：全矩阵绿 —— smoke 6/6 `028048dd…92f1`、m1r-suite 20/20 `6137aefd…e585`、
  m1r-user 35/35+coverage PASS `0c39a3e5…08d3`/`a5a140d6…c947`（三者字节不变，证明
  EOI 修复与 Full 臂不扰动既有套件）；m1r-fault 5/5 exit 35，serial 重钉
  `1ba8b13a…4fec`（内核布局位移致符号化回溯偏移变化，语义判定不变；旧钉
  `1fbae812…f967` 作废）；test-host 62 runner 例、fmt、make all 绿。
- 提交注记：首推 5ae9aa7 触发密扫误报（rustonly.rs 词法变量名 token 命中
  `token[:=]` 关键字规则）；扫描器遍历全史 blob，修复只能重写 tip——变量改名
  word 后 amend 为 251883c 并 --force-with-lease（本会话自建提交、CI 红、单作者
  仓库）。CI 17 步全绿（1m42s），demo-full 在 GitHub runner 上通过。

## M1R 判绿收口（2026-11-19，s0）

- 判定对象：净树 HEAD `ef2a335cd920d511107c8d9f7624edb6aa2d67c0`（工作树 clean），同一
  commit 上取得全部证据，无跨 commit 拼接。
- 本地电池（`results/s0-m1r-green/battery.log`，隔离 target 目录，9 步全 EXIT0）：
  make-all / test（host+runner 全过）/ fmt-check / check-rust-only / test-qemu-smoke
  / test-qemu / test-expected-fault / test-qemu-user（35 active + coverage PASS，
  复跑亦绿）/ demo-full。关键 sha256 见 `results/s0-m1r-green/hashes.txt`：
  内核 `ef0c4921…f86da`；serial 钉值 smoke `028048dd…92f1`、m1r-suite `6137aefd…e585`、
  m1r-user `0c39a3e5…08d3`（root.raw `a5a140d6…c947`）、m1r-fault `1ba8b13a…4fec`。
  m1r-user 的 root.raw 为运行后快照，退出 flush 时序可致尾块差异（非 gate；判定键为
  serial 标记 + coverage JSON），复跑钉值以 `a5a140d6…c947` 为准。
- CI：run 44381221846（push，HEAD `ef2a335c`）结论 success：
  https://github.com/MoSunDay/minios/actions/runs/44381221846
- 判定：M0R 冻结面未被扰动（smoke 串口字节不变），M1R 全部到期 gate 于同一净树
  commit 本地 + CI 双绿 → **M1R 🟢**。下一步 M2（物理页基础设施起步）。

## m2r 记录（2026-11-19，S1+S2）

- m2r-s1（内核侧）：mm/bootmem（loader map 消毒：排序/去交/可用表 + 探测顶）、
  mm/buddy（DMA/NORMAL 双 zone，order 0..10，整块分配/合并归还）、mm/slub（11 级
  kmalloc/kfree）、mm/directmap（boot PML4[510] 自 boot.S 256MiB 预映射扩至探测顶）、
  mm/vmalloc+ioremap（0xffff_9000_0000_0000 @ PML4[288] 共享入用户 PML4；常驻 PT 页；
  vmap 调用方页；MMIO RW-NX 精确 PA 位、RAM 拒绝）；frame::init 前先记 CR3。
  关键修复：console VGA 改走直接映射 —— 旧高半区别名 0xffffffff800b8000 实际落在
  内核 .data（高半区映射的是 phys base 0x500000 而非 phys 0），每次打印都以
  (char,0x0f) VGA 对覆写 MEM_MAP（现象：边界随打印量漂移的 .data 损坏）。
- m2r-s2（接线）：host-tests 纯核单测 26 例（bootmem/buddy/slub）；runner m2 verdict
  + Suite::M2Suite（needs_root=false，config_id=CORE-UP-GRUB）；Makefile
  `test-qemu-m2`（`-m 256M` + isa-debug-exit，PASS 印
  `MINIOS_X86_64_TEST_PASS_V1 suite=m2 run=core-up total=9`）；ci.yml 增步 +
  artifact 上传 target/m2-suite-run。
- 证据（净树工作树 = 已验证 scratch 树，patcher 逐字节复现）：m2 套件 9/9
  PASS（zones DMA=2261 NORMAL=61408）；m0r/m1r-suite/m1r-fault/m1r-user serial
  钉值全部不变；make all/test/fmt-check 绿。CI 待本次 push 后确认。

### m2r-s3（匿名 VMA 纯核，2026-11-19）

- 交付：`kernel/src/mm/vma.rs`（308 行纯核）——定容 32 槽排序 VMA 表：
  insert（排序/重叠拒绝/同属性邻接合并）、find/overlap、boundary/split_at、
  remove_range（事务性：容量预检 straddles 需求，失败零突变）、set_prot
  （只读覆盖预检 + 边界分裂 + 重合并）、find_area_topdown（gap 顶= min(gap_top,
  ceiling) 防越 ceiling 溢出）；权限模型 PROT_R/W/X(+RW/RWX) + VMA_ANON/STACK/HEAP，
  VMA_HEAP 允许字节精确 break（brk 语义）。内核接线仅 `mm/mod.rs` +1 行与
  `#[allow(dead_code)] // s4 mm wiring` 项级注记（内核 -D warnings）。
- host：`host-tests/src/vmatests.rs` 12 例（插入合并/重叠拒绝/heap 字节精度/
  边界查找/分裂/满表容量失败/头尾中移除/重保护重合并/覆盖缺失拒绝/顶选址/
  brk 周期回归），22 §6.1「VMA 分裂合并」host gate 落账。
- gate：make all/test/fmt 绿（host-tests 38 例）；QEMU 五套件 verdict 全 PASS
  （m0r 6/6、m1r 20/20、m1r-user 35/35 + coverage active=35、m1r-fault 5/5、
  m2 9/9）。钉值：m0r/m1r-suite/m1r-user 串口与 root 字节不变；
  **m1r-fault 串口重钉 `6a875ab4…4ef2`（root `de5b494a…c61a`）**——vma 模块入链致
  符号位移（rust_main+0x1152），verdict 仍 5/5 PASS（40 §6 按 S4e-4 先例记录）。

### m2r-s4（brk/mmap/munmap/mprotect 全链，2026-11-19）

- 交付：`abi`（SYS_MMAP/MPROTECT/MUNMAP=9/10/11 + PROT_*/MAP_*）；`kernel/src/mm/
  vm.rs` 重写为 `Mm{pml4,vmas,brk}`——alloc/dealloc 全 PTE 位（mappages 仅补
  PTE_P）、字节精确 set_brk（heap_floor 下界 + HEAP_LIMIT 上界 + 堆 VMA 同步）、
  free/copy（fork 纯复制）、map_zero_page（s5 惰性填零原语）；`kernel/src/mm/
  mmap.rs`（新 164 行）——prot_to_pte（P|U，无 PROT_X 即 NX）、mmap_anon
  （顶向选址/MAP_FIXED/PROT_NONE 仅保留地址空间/OOM 回滚 VMA 表）、munmap
  （仅匿名/全覆盖/释放帧+invlpg）、mprotect（set_prot 分裂重合并 + PTE 重授权，
  PROT_NONE 去 U 保留驻留）；`proc/exec.rs` 段映射按 ELF PF 位忠实授权（text
  无 W 非 NX、data W+NX）并插入段/栈 VMA，`new_mm.brk=max_end`；
  `kernel/syscall.rs` sys_brk/mmap/munmap/mprotect（EINVAL/ENOMEM 语义），
  growproc 删除。用户侧：`user/src/sys.rs` 三绑定 + `utest/mem.rs` 三例
  （mmap 顶向不重叠+零填+读写+munmap；munmap 后触访子进程被杀+栈拒绝；
  fork 静态页 COW 隔离——s6 升级真 COW）。
- 套件：m1r-user 35→38（xv6.mem.mmap/munmap/cow，CASE 序插 sbrk 后）；runner
  m1r spec/tests 同步；coverage 完整性过滤改为 rank≤milestone（Makefile 传
  `--milestone M2`）；m2 内核套件 9→12（m2.vma.layout 布局+brk 周期+帧守恒、
  m2.mmap.eager 顶向/零填/PROT_NONE/munmap 帧精确、m2.mprotect.perm 全授权
  矩阵+驻留数据存活+洞拒绝）。
- 修复（本阶段内闭环，QEMU 实测定位）：① set_brk 增长 perm 漏 PTE_U（mappages
  仅 OR PTE_P）→ m1r.user.brk 触访新堆页 ec=0x7 被杀、驱动带崩整个 m1r 套件
  ——补 P|U|W|NX；② `free_user_pml4` 只走 3 层（把 PT 页当数据页释放、叶 PTE
  从不访问）→ 每次进程退出泄漏全部用户数据页（M1R 无帧守恒未暴露）——补第 4
  层循环，数据页/PT/PD/PDPT/PML4 全量回收；③ m2 两例帧计数未计表开销
  （pml4+pdpt+pd+pt=4）——按确定性拓扑改 8/−8/−5/−6。
- gate：make all/test/fmt-check/check-rust-only 绿（host-tests 38 + runner 67 +
  tools 5，m2 self-test 标记变更 total=12 同步）；QEMU 全电池 PASS：m0r 6/6、
  m1r 20/20、**m1r-user 38/38 + coverage active=38 not-active=1（M2 里程碑激活）**、
  m1r-fault 5/5、m2 12/12。
- 钉值：m0r `028048dd…20f1` 与 m1r-suite `6137aefd…e585` 串口字节不变
  （注：历史条目尾码 `…92f1` 为转写笔误，权威全值见 results/s0-m1r-green/
  hashes.txt）；**m1r-user 重钉 serial `b0f5743f…a2c6` / root `9c0f9f98…5f32`、
  m1r-fault 重钉 serial `e531a032…64f7` / root `a44075ba…cea0`**（usertests
  +3 例改 root 模板 + mm 重写致符号位移）；m2-suite serial 首钉
  `e9cb4531…a419`。

### m2r-s5（缺页分类 + 栈增长 + 惰性零填，2026-11-19）

- 交付：`kernel/src/mm/fault.rs`（新 134 行纯函数模块）——#PF 错误码位
  （P/W/U/I）+ `USER_CANON_TOP` canonical 校验 + VMA 权限四重 kill（写只读/
  NX 取指/PROT_NONE/驻留页陈旧 P）+ `FaultAction{Fill,GrowStack,Kill}` 分类；
  apply：惰性匿名零填（map_zero_page 按需一页）与栈 VMA 下扩（guard 地板
  `STACK_GUARD_FLOOR = USER_STACK_TOP-64MiB`，增长窗口即 mmap 让出的 gap，
  取指缺页不增长）；内核来源（err 无 U 位）一律 kill（内核访问用户走 M1R
  fixup，不经此路径）。`traps.rs` 增 `T_PGFLT if from_user` 分派臂：先解析
  （成功则 iretq 返回重执行），未解析才 `oops::user_fault` + kill_cur。
- 套件：m2 12→14——`m2.fault.stack`（单页/多页下扩、VMA 端点与零填 RW|NX
  校验、guard 地板之下/取指/栈顶之上全 kill、帧守恒）、`m2.fault.lazy`
  （PROT_NONE→mprotect RW 后 PTE 仍缺、按需逐页填（暖表后恰 -1 帧）、
  邻页保持 absent、陈旧 P/写只读/NX/非 canonical/内核源/PROT_NONE 七类
  kill、帧守恒）；runner m2 spec/自测标记同步 14。
- gate：make all/test/fmt-check 绿（host-tests 38 + runner 67 + tools 5）；
  QEMU 全电池 PASS：m0r 6/6、m1r 20/20、m1r-user 38/38 + coverage
  active=38、m1r-fault 5/5、m2 14/14。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user serial
  `b0f5743f…a2c6` 字节不变；**m1r-fault 串口重钉 `1fc816dd…58de`（root
  `a44075ba…cea0` 不变）**——fault.rs 入链符号位移，verdict 5/5 不变；
  m2-suite serial 重钉 `3f67e996…c9ba`（14 例）；m1r-user root（运行后像）
  `12c253b4…4646`——内核堆残留字节随布局位移，非语义变化（serial 逐字节
  相同）。

### m2r-s6（COW fork + 内核态用户缺页解析，2026-11-19）

- 交付：
  - `mm/frame.rs`——页帧引用计数：`Page.count` 兼作分配票根+共享数；
    `kfree` 升级为减引用语义（count>1 只递减保留驻留，末引用才 junk-fill
    +回伙伴系统；双释放/坏帧 panic）；新增 `page_ref/page_ref_inc/
    page_ref_dec`（均在 KMEM_LOCK 下，dec 仅 count≥2 否则 panic）。
  - `mm/pgtable.rs` `copy_user_half`——fork 由整页复制改 COW 共享：可写
    叶子双清 W（父侧 PTE 就地改写 + `invlpg`，src 可能是活动 CR3）、
    `page_ref_inc`、子侧同帧映射 bits=P|U|NX；失败路径 `free_user_pml4(dst)`
    经引用计数 kfree 回滚全部已共享帧。
  - `mm/fault.rs`——`FaultAction::Cow{va}`：classify（err&P 合法形态仅
    写缺页+VMA W+PTE 缺 W→Cow，其余 Kill）；apply（ref==1 原地复权：恢复
    W 保 NX + invlpg；ref>1 kalloc+memmove 私有拷贝映射 P|U|W|NX + 旧帧
    page_ref_dec）。
  - **COW 回归修复**（QEMU 驱动）：fork 双清 W 后父进程用户页只读，
    uaccess `copyout`（内核态写用户缓冲，如 sys_read 落 cap 到栈）经
    M1R fixup 直接 EFAULT，`run_capture` read 返回 Err——m1r-user 8 例
    全灭（exectest/echo/cat/grep/wc/ls/forktest/stressfs；产出 0 字节的
    子进程用例不触发故仍绿）。修法：`fault.rs` 新增
    `handle_kernel_user_fault`——内核态 #PF 落用户半区先按当前进程 mm
    解析（合成 U 位走 classify：COW/Fill/GrowStack 可解析则重执行指令），
    不可解析才回落 M1R fixup/oops（真实 EFAULT 语义不变）；traps.rs
    `T_PGFLT if !from_user` 臂接入。
- 套件：m2 14→15——`m2.cow.pagecount`（fork 帧共享：ref=2+双侧 W 清+恰
  4 表帧开销；子写 COW 私有拷贝：ref 回 1+写标记保留；父独占后原地复权；
  teardown 帧守恒精确）；runner m2 spec/自测标记同步 15。
- gate：make all/test/fmt-check 绿（host-tests 38 + runner 67 + tools 5）；
  QEMU 全电池 PASS：m0r 6/6、m1r 20/20、**m1r-user 38/38（COW 回归修复
  后复绿）** + coverage active=38 not-active=1、m1r-fault 5/5、m2 15/15。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user serial
  `b0f5743f…a2c6`、m1r-fault `1fc816dd…58de`（root `a44075ba…cea0`）
  字节不变；**m2-suite serial 重钉 `e0b9a7aa…15d2`（15 例）**；
  m1r-user root（运行后像）`9c0f9f98…5f32`——内核堆残留随布局位移，
  非语义变化（serial 逐字节相同）。

### m2r-s7（匿名 OOM：候选选择 + 终止 + 统计，2026-11-19）

- 交付：
  - `mm/oom.rs`（新 211 行）——纯函数候选选择（`select_oom_victim`：最大
    匿名足迹、平手最低 pid、零足迹不可选）；`alloc_user_page()` 阀门：kalloc
    失败→failed 计数→`collect_candidates`（ptable 锁下快照，UNUSED/EMBRYO/
    ZOMBIE/已 killed/无 mm 一律 rejected 计数，`pgtable::count_user_pages`
    走表数叶子）→选中标记 killed+按 chan 唤醒；自害返回 null 走正常
    fault-kill/mmap 回滚，他害 64 次有界 yield 重试；统计
    scanned/rejected/freed/kills/failed（Spinlock 静态）。策略 ADR-0015。
  - 接线：`vm.rs` `Mm::alloc`（brk/mmap/exec 段 eager 填充）与
    `map_zero_page`（惰性填/栈增长）、`fault.rs` COW 私有拷贝改走阀门；
    fork 页表与 kmalloc 保持礼貌 ENOMEM 不入 OOM。
  - 套件：m2 15→16 `m2.oom.select_stats`（纯选择四断言 + 真实触发：1 GiB
    过量匿名申请拒收、failed/kills/scanned 计数、无人被杀、回滚+teardown
    帧守恒、阀门恢复再服务）；m1r-user 38→39 `xv6.mem.oom`（子进程 1 MiB
    块风暴至枯竭——恰被终止 0xff（最大足迹=自己），驱动存活且 wait4 收割
    后 mmap 写读 munmap 恢复）；runner m2/m1r spec+自测标记同步 16/39；
    coverage.toml 增 `xv6.mem.oom`（M2 required，adr ADR-0015）。
  - 修 bug（QEMU 驱动）：初版 m2.oom 用例复用已 `free()` 的 Mm 做"恢复
    探测"——`walk(NULL)` 内核 GP oops；改用 fresh Mm。另按行数上限把
    m2suite 拆为 `m2suite.rs`（物理页）+ `m2map.rs`（映射）+ `m2mem.rs`
    （缺页/COW/OOM，共享 `pte_of`）。
- gate：make all/test/fmt-check/check-rust-only 绿（host-tests 38 + runner
  67 + tools 5）；QEMU 全电池 PASS：m0r 6/6、m1r 20/20、**m1r-user 39/39 +
  coverage active=39 not-active=1**、m1r-fault 5/5、**m2 16/16**。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585` 字节不变；
  **m1r-user serial 重钉 `58452d31…48f0`（39 例；root 运行后像
  `f3384035…1f87`）**；**m1r-fault 重钉 `d98454df…dbfe`**（oom.rs 入链
  M1RFault 符号位移，verdict 5/5 不变；root `0611a2b6…d4c3`）；**m2-suite
  serial 重钉 `3c1f5b27…70f5`（16 例）**。
- 判定：M2 到期 gate 全绿（s3 VMA 核心 → s4 brk/mmap/munmap/mprotect →
  s5 缺页分类/栈增长/惰性零填 → s6 COW fork → s7 匿名 OOM），本地电池
  全 PASS；同净树 commit `67bc6ca` 推 CI run 44556592736（core-ci）结论
  success → 本地 + CI 双绿 → **M2 🟢**。下一步 M3（proc/sched 深化，
  21 §M3 gate）。

### m3r-s1（CFS 核心：sched_entity/红黑树/vruntime/nice 权重/yield，2026-11-20）

- 交付：
  - `proc/rbtree.rs`（新 381 行）——侵入式红黑树（CLRS 插入/删除修复 +
    Linux 式 leftmost 缓存，等键 FIFO）；结构审计（父链/黑高/红红/序/计数）
    放宿主侧 `rbtreetests.rs`（`#[cfg(test)]` 访问器，内核零审计代码）。
  - `proc/cfs.rs`（新 206 行）——纯策略：Linux 40 档 nice 权重表、
    `update_curr`（sum_exec 墙钟 + vruntime×1024/weight）、`min_vruntime`
    单调地板、`place_new/place_wake`（clamp 到地板）、集中 `granularity =
    max(SCHED_LATENCY/(queued+1), MIN_GRAN)`、`preempt_needed`、
    `yield_entity`。
  - `proc/preempt.rs`（新 44 行）——UP preempt_count + `sched_raw` 前审计
    断言（消费者 s5/s7；见 ADR-0016）。
  - `sched.rs` 重写（174→293 行）——scheduler 扫描改 `pick_next_entity`
    （leftmost 出树 + `exec_start=now`），就绪队列共用 PTABLE_LOCK（无新
    锁序）；`sched_raw` 统一记账出队切片（yield/sleep/exit 全路径）+
    preempt 审计；`entity_tick` 仅用户态定时抢占；`runnable_new/wake`
    覆盖全部 6 个 RUNNABLE 迁移点（fork:267/kill:283/userinit:342/yield/
    wakeup1:63/pick）；`freeproc` 重置实体；swtch/forkret/锁所有权协议
    零改动。
  - `SYS_SCHED_YIELD(24)`（abi + dispatch）；`time::tick` 用户抢占改走
    `entity_tick`。
  - 宿主门（21 §7 确定性模型）：`m3tests.rs`（新 193 行）——Sim 镜像
    sched.rs 胶水（固定 10ms 节拍/单 CPU/无迁移）：同权重 4 实体 4000
    tick 散布 <5%、nice 0:5 → 1024:335（±5%）、两实体交替 ≤2 tick、
    阈值/权重/唤醒 clamp/yield 单测。
- 修 bug（宿主模型先行拦截）：`exec_start==0` 哨兵与开机 `TICKS==0` 冲突
  （首个被 pick 的进程永不记账 → 永不抢占 → 饿死全员）；`granularity`
  分母漏算当前实体（对齐 Linux nr_running 语义）。
- gate：make all/fmt-check/check-rust-only 绿；host-tests 38→**48** +
  runner 67 + tools 5；QEMU 全电池 PASS：m0r 6/6、m1r 20/20、m1r-user
  **39/39**（preempt 金丝雀绿）+ coverage active=39 not-active=1、
  m1r-fault 5/5、m2 16/16。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user
  `58452d31…48f0` 不变；**m1r-fault 重钉 `e9c0ae56…5374`**（rbtree/cfs/
  preempt 入链符号位移；root `0611a2b6…d4c3` 不变）；**m2-suite 重钉
  `c3636c75…0b62`**（16 例）。
- 决策：ADR-0016（抢占显式关闭 + tick 记账 + clamp 简化 + 宿主模型门）。
### m3r-s2（RT FIFO/RR + sched syscall 族 + m3-suite，2026-11-20）

- 交付：
  - `proc/rt.rs`（新 ~170 行）——每优先级 FIFO 双端链（头/尾）+ `u128`
    位图（bit=prio−1）；`push_tail/push_head/unlink/front/front_prio/
    has_higher/is_empty`；宿主 `rttests.rs` 4 例（级序、队头抢占、RR
    旋转、位图 off-by-one——该 bug 由宿主先行拦截）。
  - `Proc` 增 `policy/rt_prio/rt_link/rt_ticks_left`；`SchedPolicy`
    Normal=0/Fifo=1/Rr=2；fork 继承 policy/prio，freeproc 复位。
  - `sched.rs`（293→409 行）——`RT_RQ` + `pick_next_proc`（RT 先于
    CFS）+ 类感知 `enqueue_runnable`（RUNNABLE ⟺ 在本类队列）+ RT
    `entity_tick`（FIFO/Rr/Normal 三臂：任何 runnable RT 抢占 CFS
    当前体；RR 量子 10 tick 旋转）+ `set_sched_policy`（policy 迁移 =
    出旧队入新队，RR 重置量子）。
  - ABI `SYS_SCHED_SETPARAM(142)/GETPARAM(143)/SETSCHEDULER(144)/
    GETSCHEDULER(145)/GET_PRIORITY_MAX(146)/GET_PRIORITY_MIN(147)`；
    `syscall.rs` 六臂 + `sched_target/sched_valid`；锁纪律：PTABLE_LOCK
    内快照，copyout 放锁后。
  - m3-suite 上线：`kernel/m3suite.rs`（6 例策略级电池，boot 上下文，
    cmdline `m3-suite`）+ `BootMode::M3Suite` + `scripts/m3-suite-grub.cfg`
    + Makefile（`m3-suite-iso`/`test-qemu-m3`）+ runner `Suite::M3Suite`
    （verdict/m3.rs，exit 33）。
- 修 bug（QEMU 首跑拦截）：套件曾以裸 offset-0 强转从 `se`/`link` 指针
  回收 `FakeEnt`——`#[repr(Rust)]` 布局在 host debug 与内核 release 下
  字段重排不同，release 下 `se` 不在 0 偏移（RT 例的 `link` 恰在 0），
  CFS 例读错 id、repr(C) 修复后 RT 例越界 oops；终修 = `offset_of!`
  container_of（`ent_of_se`/`ent_of_link`，同 sched.rs `Proc` 惯例），
  任意布局成立。红跑顺带实证 runner FAIL 判定与 kernel-oops FATAL 路径。
- gate：make all/fmt/fmt-check/check-rust-only（roots=6 files=139）绿；
  host-tests 48→**52**（rttests 4）+ runner 72 + tools 5；QEMU 全电池
  **6 目标** PASS：m0r 6/6、m1r 20/20、m1r-user **39/39**（preempt
  金丝雀绿）+ coverage active=39 not-active=1、m1r-fault 5/5、m2 16/16、
  **m3-suite 6/6（首跑）**。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user
  serial `58452d31…48f0` 不变（m1r-user root 运行后像 `61cd7275…6138`）；
  **m1r-fault 重钉 `e0b37b6e…48ea`**（rt/m3suite 入链符号位移；root
  `0611a2b6…d4c3` 不变）；**m2-suite 重钉 `0baa40fb…3c32`**（16 例）；
  **m3-suite 首钉 `ba629b98…4112`**（6 例）。
- 决策：ADR-0017（见上）。

### m3r-s3（waitqueue：可中断/exclusive + ERESTARTSYS + pipe/wait4 迁移，2026-11-20）

- 交付：
  - `proc/waitqueue.rs`（新 158 行，纯策略可宿主单测）——WaitLink
    （幂等 unlink）+ WaitQueue + `enqueue_link`（非 exclusive 头插/
    exclusive 尾插，Linux `__add_wait_queue{,_tail}`）+ `select_wake`
    （one = 全部非 exclusive + 首个 exclusive；all = 全体）。
  - `sleep.rs`（72→152 行）——内核胶水 `wq_sleep`（xv6 sleep 锁协议：
    资源锁跨睡眠释放/复取，PTABLE_LOCK 过 swtch 转移）+
    `wake_up(_locked)`（offset_of 定位 Proc）+ `interrupted`。
  - pipe 迁移：单一 `waitq`（读者非 exclusive/写者 exclusive——读者
    腾空间 wake-one 防惊群，写者落数据 wake-all）；**顺带落地
    pipewrite EPIPE**（读者全关 → -EPIPE；SIGPIPE s6）。
  - wait4 迁移：睡 `PTABLE_WAITQ`（可中断），exit() wake-all；
    kill() 零改动 + 睡眠者幂等自摘链 → 可中断等待以 ERESTARTSYS 破局
    （修复 xv6 已知 wart：被 kill 的管道睡眠者死睡）。
  - `syscall_dispatch` ERESTARTSYS(-512) 回卷 rip-=2（`syscall` 指令
    2 字节）；abi 增 `ERESTARTSYS/ERESTARTSYS_ABS/EPIPE_ABS`。
  - m3-suite 扩 2 例（`m3.wq.wake_policy`/`m3.wq.unlink_middle`，
    FakeWqEnt 用 offset_of 回收——s2 布局教训直接复用），6→8；runner
    spec/自测同步（fixture 计数改动态派生）。
  - 宿主 `waitqtests.rs` 6 例（one/all 纪律、中摘/头尾摘幂等、链接
    标志、chan 相异性）。
- gate：make all/fmt/fmt-check/check-rust-only（files=141）绿；
  host-tests 52→**58** + runner 72 + tools 5；QEMU 电池 6 目标 PASS：
  m0r 6/6、m1r 20/20、m1r-user **39/39（串口钉值跨迁移字节不变——
  pipe/wait/kill 全语义保持）**+ coverage active=39 not-active=1、
  m1r-fault 5/5、m2 16/16、m3-suite **8/8**。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user
  serial `58452d31…48f0`/root `61cd7275…6138`、m1r-fault
  `e0b37b6e…48ea`/root `0611a2b6…d4c3` 全部不变；**m2-suite 重钉
  `537075a3…4c26`**；**m3-suite 重钉 `c4dd5ecc…1a40`**（8 例）。
- 决策：ADR-0018（纯链表层 + Linux 唤醒纪律 + ERESTARTSYS 回卷；
  chan 原语保留，console/ide/bcache/ilock/log 后续机会迁移）。

### m3r-s4（时间层：clocksource/hrtimer/wheel + alarm/gettimeofday/rusage/nanosleep 重写，2026-11-20）

- 交付：
  - `kernel/time/` 模块五件套（原 kernel/time.rs 48 行拆分，755 行）：
    clocksource（TSC + PIT 校准，校准窗口 sti/cli——boot 死锁 #1 修复；
    cycles_to_ns_with 纯函数）/ clockevents（骨架 + ONESHOT 标注 M8）/
    hrtimer（Abs/Rel 有序链，run_expired 显式 now 驱动）/ wheel
    （tv1–tv5 级联，WHEEL_HORIZON=1<<26，past-expiry 停泊语义）/
    mod.rs（init_time + tick()）。
  - **boot 死锁 #2 修复（向量当线号）**：init_time 原以
    `pic::unmask(IRQ_TIMER as u8)` 传向量 32——从片路径回绕
    `1u8<<24`→bit0，从片 IRQ8 被开而 IRQ0 仍屏蔽 → PIT 永不送达 →
    calibrate_tsc 死转，全部 m1r_boot 模式挂死 GRUB 横幅（指纹
    pic0 irr=01 imr=fb）；改 `pic::unmask((IRQ_TIMER - IRQ_BASE) as
    u8)`（ide.rs 惯用法）。
  - syscall：SYS_gettimeofday(96)（16B timeval，NULL 合法）、
    SYS_alarm(37)（Proc.alarm 宿主 LrTimer，alarm_fire→alarm_fired
    +kill_proc；prev 剩余向上取整、WHEEL_HORIZON 钳制、负数
    EINVAL）、SYS_wait4 增 rusage（144B，ru_utime 自 sum_exec_ns，
    exit 前 wait_status 三元组捕获防 freeproc 抹账）、SYS_nanosleep
    重写于栈上 HrTimer（Abs 截止 + nanosleep_wake 先取 TICK_LOCK 防
    丢醒 + 全返回路径取消）。
  - proc：alarm/alarm_fired 字段（freeproc 复位、exit 先 del_timer）、
    kill_proc 抽取、exit 前捕获 exec_ns。
  - m3-suite 扩 3 例（`m3.time.clock_layer_wiring`/
    `m3.time.hrtimer_order_cancel`/`m3.time.wheel_cascade`——真实
    300 tick 级联），8→11；runner 同步。
  - 宿主 timetests 9 例（wheel 阈值/桶位/级联精确触发/del 阻燃/
    past-expiry 重访、hrtimer 顺序/取消竞态/Rel 重排），58→67；
    host-tests 增 time.rs shim（TICKS stand-in + #[path] 引真源码，
    Spinlock/io shim 进 lib.rs——内核零改动可宿主测）。
- gate：make all（0 warning）/fmt/fmt-check/check-rust-only（files=
  147）绿；host-tests 58→**67** + runner 72 + tools 5；QEMU 电池
  6 目标 PASS：m0r 6/6、m1r 20/20、m1r-user **39/39（跨 nanosleep
  重写串口钉值字节不变）**+ coverage active=39 not-active=1、
  m1r-fault 5/5、m2 16/16、m3-suite **11/11**。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user
  serial `58452d31…48f0`/root `61cd7275…6138` 不变；**m1r-fault
  重钉 `909613e4…7ec4f`**（backtrace 符号偏移随代码布局移位，
  root.raw `0611a2b6…d4c3` 不变）、**m2-suite 重钉 `8ad5b441…8a14`**
  （vma.layout 动态地址随布局移位）、**m3-suite 重钉
  `39f1576a…69006`**（8→11 例）；三者均二连跑字节稳定。
- 决策：ADR-0019（三层时间结构 + 过期停泊语义 + 向量/线号教训；
  亚 tick 精度与 RTC wall-clock 延后 M8）。

### m3r-s5（kthread/workqueue/softirq：底半部与内核线程基础，2026-11-20）

- 交付：
  - `proc/preempt.rs`：preempt_count 低 8 位抢占位 + 位 8-15
    softirq/BH 深度（SOFTIRQ_OFFSET/MASK、local_bh_disable/enable、
    in_softirq）——2.6 布局缩影，sched_raw 可审计。
  - `proc/kthread.rs`（新）：KthreadFn=extern "C" fn(usize)->i32；
    kthread_create（重播 allocproc 种子：**slot2=arg(r13)/slot3=fn
    (r12)/slot6=trampoline——swtch 镜像槽序与压栈序相反**，首跑
    fn/arg 反置经 0 间接调用 oops `rip=?0x1`，GDB 核验后修正）；
    naked `_kthread_trampoline`（对齐 + call kthread_body + ud2 兜
    底）；kthread_body（raw_release + sti + fn(arg) + exit(code)）；
    kthread_run（PTABLE_LOCK + runnable_new）/kthread_reap（ZOMBIE
    断言 + freeproc + xstate 回收）。tf=null（无用户态）、mm=None
    （调度器免 CR3 切换，boot PML4 直跑）。
  - `kernel/softirq.rs`（新）：2.6.11 索引 + 静态 vec/data/PENDING；
    open/raise/pending；do_softirq（BH 深度守卫、
    MAX_SOFTIRQ_RESTART=10、升索引快照分发、耗尽唤醒 ksoftirqd）；
    traps_dispatch IRQ 出口钩 irq_exit()（IF 仍屏蔽——UP 简化）；
    tasklet 双表（SCHED 去重、先清位再跑、禁用跳过）；ksoftirqd
    kthread（排空 + wq_sleep 停泊 + interrupted 退出）+ 惰性
    spawn_ksoftirqd；init() 注册 TASKLET/HI（探测用例走 NET_TX/
    NET_RX，不劫持 init 槽位）。
  - `kernel/workqueue.rs`（新）：events 静态队列（FIFO + pending
    去重 + more/done 双等待队列 + 惰性 events worker）；flush_work
    睡 done；schedule_work（M5 writeback 消费，暂 dead_code）。
  - `kernel/time/mod.rs`：tick() 只 raise TIMER_SOFTIRQ，到期分发
    （run_expired + wheel_tick）移入 timer_softirq；**单时基修复：
    open_softirq 提前到 unmask/校准之前**——校准窗 tick 无人分发
    会让 WHEEL.jiffies 永久滞后 TICKS（wheel_tick 空桶即断、每调用
    恰推进 1 拍），wheel_cascade 以 TICKS 装填则永不触发（GDB 指纹：
    timer_softirq 每拍进入 pc=0x100 而 WHEEL.jiffies 不动；注意
    Rust 重排结构体——jiffies 在 WHEEL+0x1000 非 +0）。
  - main.rs：m1r_boot 公共段 softirq::init()；M1rUser/Full 两臂
    spawn_ksoftirqd（m3-suite 自带；smoke/m1r-suite/m2/fault 不
    spawn——钉值稳定，无软中断压力）。sched.rs 增 sched_drive
    （调用者栈上内联调度器，借用 CPU0.scheduler，镜像 pick/switch/
    fpu/switchuvm 窗口；kthread/mm=None 免 CR3 切换）。
  - m3-suite 11→**16** 例：`m3.kthread.run_exit`（run→ZOMBIE
    xstate=42→reap→UNUSED）、`m3.workq.dedup_run`（二连 queue 去
    重、drive 后 RAN==1、重排 RAN==2）、`m3.softirq.order_bh_mask`
    （TX(2) 先于 RX(3)、BH 深度全程持有）、`m3.softirq.tasklet_sched`
    （SCHED 去重/单跑/重排/普通+HI 双表）、`m3.softirq.
    ksoftirqd_pressure`（NET_RX 自增殖风暴→预算耗尽余位+ksoftirqd
    RUNNABLE→停风暴→drive 排空归 SLEEPING）；runner 同步 16 id。
  - 不新增 host 测试：softirq/kthread 语义绑定中断与调度上下文
    （bh 深度、swtch 镜像、调度借用），QEMU m3-suite 承保覆盖。
- gate：make all（0 warning）/fmt/fmt-check/check-rust-only（files=
  150）绿；host-tests **67** + runner 72 + tools 5；QEMU 电池
  6 目标 PASS：m0r 6/6、m1r 20/20、m1r-user **39/39 串口钉值字节
  不变（ksoftirqd 占 PTABLE 槽未扰动 forktest 计数）**+ coverage
  active=39 not-active=1、m1r-fault 5/5、m2 16/16、m3-suite
  **16/16**。
- 钉值：m0r `028048dd…20f1`、m1r-suite `6137aefd…e585`、m1r-user
  serial `58452d31…48f0`/root `61cd7275…6138` 不变；**m1r-fault
  重钉 `f3bf38d2…476e`**（backtrace 符号偏移随 s5 代码布局移位，
  root.raw `0611a2b6…d4c3` 不变）、**m2-suite 重钉
  `d81878e3…8ca8`**（zones 边界/vma.layout 动态地址随内核镜像
  增大移位）、**m3-suite 重钉 `c423adf7…2a9a`**（11→16 例）；
  三者均二连跑字节稳定。
- 决策：ADR-0020（softirq/workqueue/kthread 结构与分歧 + swtch
  槽序教训 + 单时基不变量「softirq 处理器先于首 tick 就位」）。

### m3r-s6（信号：Linux 式信号核心 + EPIPE/SIGPIPE/SIGCHLD，2026-11-21）

- 交付（三段提交）：
  - **s6a `cce75fd`**：m3suite 拆分——`m3suite/{mod,sched,time,bh}.rs`
    按行数规约分叶（纯代码搬移；m3-suite 串口 `c423adf7…2a9a`
    两连跑字节不变）。
  - **s6b `c737232`**：信号核心——`kernel/sigset.rs`（NSIG=64 位图
    代数 + SigAction）；Proc 增 `sig_pending/sig_blocked/
    sig_actions:[SigAction;64]`（freeproc 清零；fork 继承
    actions+blocked 不继承 pending；exec 保留 blocked+SIG_IGN 其余
    回 DFL）；Sigframe 208B（retaddr+0/signum+8/magic+16/tf+24/
    mask+200），`frame_base_for(rsp)=(rsp-344&!0xf)+8`；投递点在
    syscall/IRQ 出口 + fault_cur 同步路径；rt_sigaction/
    rt_sigprocmask/rt_sigpending/kill/alarm 系统调用（SA_RESTORER
    强制、未知位 EINVAL）；m3 16→**19** 例（kthread 信号槽位/
    NODEFER/RESETHAND 三例）、host-tests **67→75**（+8 sigtests；
    host 零依赖原则——abi 交叉断言放内核侧 `const _` 块）。
  - **s6c**：`abi::sig`（编号/SA_*/how/32B UserSigaction，与
    kernel::sigset 编译期互钉）；pipewrite 环满 readopen==0 → 放
    管道锁后 `sig_send(SIGPIPE)` → -EPIPE；exit() 持 ptable 锁
    `sig_send_locked(parent,SIGCHLD)` 先于 wake_up_locked（wait4
    收尸不依赖 disposition）；fault_cur 强制越过 SIG_IGN（IGN 复
    位 DFL 再投递，防缺页死循环）；用户态 sys.rs kill/rt_sig*/
    alarm + `sigreturn_stub`（global_asm `mov rax,15;syscall;ud2`，
    SA_RESTORER 契约）；m1r-user 39→**45** 例（roundtrip/
    mask_defer/ipc.sigpipe/except/chld/alarm）+ coverage +5；runner
    spec 45 id + 冻结断言 39→45；Makefile --milestone M3。
- 调试期真实缺陷（全修，45 例承保）：
  1. **sysfile errno 吞并**：分块 sys_read/write 把负返回改写
    EIO/短读（-512 ERESTARTSYS、-32 EPIPE 被吃）；改原码透传
    （`total>0 ? total : r`，POSIX 部分传输不重启）。
  2. **ERESTARTSYS 回卷毁号**：重启臂写 `tf.rax=-512`，回卷的
    syscall 以 nr=-512 再入 → ENOSYS；Linux orig_ax：入口暂存
    `orig_nr`，重启臂 `tf.rip-=2; tf.rax=orig_nr`。
  3. **stub 地址取错**：`static as u64` 加载首个操作码 0x48 而
    非地址（restorer=0x48 → ifetch 缺页）；`addr_of!` 修正。
  4. （用例侧）sigpipe 父进程关错端（留 fds[0] → readopen 永不
    归零 → 子进程满环死睡 wait4 死锁超时；且写已关 fds[1] 得
    EBADF）；共享 HANDLED 计数跨用例未清（mask_defer 遗留
    SIGUSR2 → alive 误判）。
- gate：make all（0 warning，含 user lib unsafe 清理）/fmt/
  fmt-check/check-rust-only（files=158）绿；host 75/72/5；
  QEMU 电池 6 目标 PASS：m0r 6/6、m1r 20/20、**m1r-user 45/45 +
  coverage active=45 not-active=0**、m1r-fault、m2 16/16、m3 19/19。
- 钉值：m0r `028048dd…20f1`、m1r-suite serial `6137aefd…e585`、
  m1r-fault serial `ec882ba6…0ffc`、m3 `0e1aa635…c889` 均不变；
  **新钉 m1r-user serial `da87aa63…f6f6`/root
  `45cf8a2b…f70`（45 例）**；**重钉 m1r-suite root `341ae68a…
  15ce`、m1r-fault root `f702cdf2…8ca5`**（root 模板随用户 ELF
  变化）、**m2 serial `56015653…7a6a`**（vma.layout 动态地址随
  内核镜像移位）；全部 ×2 字节稳定。
- 决策：ADR-0021（sigframe/生成点三处/orig_ax 重启契约/errno
    透传/越过 IGN/与 2.6 的分歧清单）。
