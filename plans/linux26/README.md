# minios — x86_64 / Linux 2.6.39.4 演进计划

> 技术基线：**x86_64 长模式 + Linux 2.6.39.4 核心机制 + ext4/jbd2**。
> xv6 仅作早期语义参照；实际实现状态以 [STATUS.md](STATUS.md) 为准。

## 目标边界

- **Core-2.6（M0R–M8）**：交付可启动、可演示、可回归的核心机制，不等于完整 Linux ABI。
- **Parity-2.6（M9+）**：依据可再生成的权威差距表关闭冻结范围内的 syscall/ABI 与子系统
  差距，并以外部黑盒负载验收；全主线全部配置、驱动和文件系统的 1:1 未立项。
- **Rust-only**：内核、用户态、runtime/ulib、工具及仓库内测试全部使用 Rust；汇编仅用于
  必要硬件边界，脚本只负责启动、传参、timeout 与日志收集，不实现测试或 verdict。
- **外部黑盒边界**：xv6 C 只读参考；glibc/NPTL、LTP 和发行版 rootfs 仅作隔离的 Parity
  验证负载，不进入 minios 实现、Cargo workspace 或 Core 构建依赖。外部 C 不替代 Rust 交付。
- 不交付 i686 内核、ELF32 用户态或 ia32 compat；引导过渡指令不构成 32 位 ABI。
- **当前代码尚未迁移到本目标**，仍是原 i686/C 用户态实现；旧证据只归档，不计 Core 或
  Parity 完成度。计划中的接口、job 与配置均为待交付契约，不表示当前已经可用。

## 构建与验证权威

- [40-testing-ci.md](40-testing-ci.md) 是 **QEMU 配置、测试判定、CI 与证据验收唯一权威**；
  本文不复制易漂移的 QEMU 命令、退出协议或完成门禁。
- [04-rust-userspace.md](04-rust-userspace.md) 固定内部 Rust 用户 ELF、rust-lld 直链、
  镜像流水线与外部动态 ELF 边界；[20-roadmap.md](20-roadmap.md) 给出分阶段 gate。
- Core 与 Parity 分开报告；dirty、stale、旧架构或跨配置拼接的证据均不能判绿。

## 文档索引

| 文档 | 内容 |
|------|------|
| [STATUS.md](STATUS.md) | Core/Parity rollup、里程碑状态与实施依赖 |
| [00-scope-linux26.md](00-scope-linux26.md) | Linux 2.6.39.4 基线、Core/Parity 范围矩阵 |
| [01-current-state-audit.md](01-current-state-audit.md) | xv6 与原 i686 实现的历史审计 |
| [02-architecture.md](02-architecture.md) | x86_64 架构、目录边界与 Rust 约束 |
| [03-rust-rewrite-strategy.md](03-rust-rewrite-strategy.md) | 迁移批次、翻译规约与回退 |
| [04-rust-userspace.md](04-rust-userspace.md) | Rust 用户态、共享 ABI、链接与镜像流水线 |
| [10-git-history-cleanup.md](10-git-history-cleanup.md) | 无二进制历史政策与可选净化附录 |
| [20-roadmap.md](20-roadmap.md) | M0R–M8 Core 与 M9+ Parity 路线图 |
| [21-proc-sched.md](21-proc-sched.md) | CFS、线程、信号、定时器与 RCU |
| [22-memory.md](22-memory.md) | 页表、物理内存、VMA/COW、回收与 OOM |
| [23-vfs-block.md](23-vfs-block.md) | VFS、page cache、BIO 与块层 |
| [24-ipc-smp.md](24-ipc-smp.md) | IPC、futex、SMP、锁与事件等待 |
| [25-drivers-net.md](25-drivers-net.md) | 驱动模型、tty、平台与网络边界 |
| [30-ext4.md](30-ext4.md) | ext4/jbd2 格式、阶段与离线校验闭环 |
| [40-testing-ci.md](40-testing-ci.md) | QEMU、测试、CI 与证据唯一权威 |
| [50-risks-adr.md](50-risks-adr.md) | 架构决策与风险登记册 |

## 四项任务映射

| # | 任务 | 对应方案 |
|---|------|----------|
| 1 | 无二进制提交 | 全新历史与持续检查，见 10 |
| 2 | x86_64 / Linux 2.6.39.4 | M0R/M1R 迁移，M2–M8 Core，M9+ Parity |
| 3 | ext4/jbd2 | M5 RO → M6 无日志 RW → M7 ordered/replay，见 30 |
| 4 | Linux 风格代码组织 | Rust 模块边界与职责拆分，见 02 |

minios 使用全新历史；xv6 参考副本不导入其历史。commit、push、部署和历史改写均不因本计划
自动获得授权。
