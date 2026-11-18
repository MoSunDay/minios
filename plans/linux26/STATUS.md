# STATUS — x86_64 / Linux 2.6.39.4 实施状态

> 本文件只报告实施与证据状态。**计划文档终审完成不等于实现完成，也不使任何 gate 变绿。**
> x86_64/Rust-only 迁移已启动：M0R 已在净树本地取证与 GitHub CI 首跑双重全绿（见下）；
> 验收规则以 [40-testing-ci.md](40-testing-ci.md) 为唯一权威。

## Rollup

| 范围 | 当前状态 | 判定 |
|------|----------|------|
| Core-2.6（M0R–M8） | 🟢 M0R done（M1R–M8 pending） | M0R 净树取证全绿（HEAD `4c22b7b`，内核 sha256 `8aacf344…`）+ CI 首跑 success（run #1，全部 14 步绿）；证据见下文 M0R 现状 |
| Parity-2.6（M9+） | ⬜ **pending** | 未启动；权威差距表、Parity 配置和外部矩阵均未验收 |

最近同步：2026-11-16。唯一实现基线为 Linux 2.6.39.4 / x86_64；不沿用旧完成度或工期。

## 里程碑状态

| 里程碑 | 交付摘要 | 状态 | 当前证据/前置 |
|--------|----------|------|---------------|
| M0R | 工具锁、Multiboot1/ELF64、长模式、高半区、真实 QEMU smoke、CI workflow | 🟢 **done**（2026-11-16 判绿） | 净树 HEAD `4c22b7b` 全套 exit 0 + 三组 sha256 + CI 首跑 success：https://github.com/MoSunDay/minios/actions/runs/43886738739 |
| M1R | syscall/FPU/UserPtr、静态 Rust ET_EXEC 用户态与到期 required 用例 | 🟡 **S1+S2 已交付** | trap/fixup/定时器运行时全绿（m1r-suite 33、m1r-fault 35、M0R 串口字节不变）；S3 用户 ELF（execve）待做；旧 C usertests 不计 |
| M2 | 匿名 VMA/brk/mmap/COW 与匿名 OOM | ⬜ pending | 等 M1R 全绿 |
| M3 | UP PIT/PIC、CFS、信号、EPIPE/SIGPIPE | ⬜ pending | 等 M2 全绿 |
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
