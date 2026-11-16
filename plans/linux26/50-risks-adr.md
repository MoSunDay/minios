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
