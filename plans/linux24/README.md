# minios — Linux 2.4 水平内核演进计划（linux24）

> 使命：以 **Rust** 从零实现一个 x86 内核 **minios**，以 xv6 为语义基准、以 Linux 2.4 为架构与能力标杆，
> 最终支持 **ext4**（含日志），代码组织完全参照 Linux 目录风格。

## 四项任务与本计划的映射

| # | 任务 | 方案 | 文档 |
|---|------|------|------|
| 1 | 去掉二进制的提交 | minios **全新历史**从零开始，永不入库二进制；xv6 旧仓库如需净化附带已验证流程 | [10-git-history-cleanup.md](10-git-history-cleanup.md) |
| 2 | 内核迭代到 Linux 2.4 的状态 | 8 个里程碑 M0–M8，特性矩阵对标 Linux 2.4.37 | [00-scope-linux24.md](00-scope-linux24.md) / [20-roadmap.md](20-roadmap.md) |
| 3 | 支持 ext4 文件系统 | 四阶段：只读 → 读写（无日志）→ jbd2-lite 日志 → 高级特性 | [30-ext4.md](30-ext4.md) |
| 4 | 组织好代码逻辑，参考 Linux | Linux 风格目录树 + Rust workspace 模块边界 + 编码规范 | [02-architecture.md](02-architecture.md) |

## 文档索引

| 文档 | 内容 |
|------|------|
| [STATUS.md](STATUS.md) | 里程碑进度、验收 gate、阻塞项（活文档，每轮更新） |
| [00-scope-linux24.md](00-scope-linux24.md) | "Linux 2.4 状态"的精确定义：特性矩阵、In/Out Scope |
| [01-current-state-audit.md](01-current-state-audit.md) | 基线审计：xv6 实测数据、构建问题、历史二进制盘点 |
| [02-architecture.md](02-architecture.md) | 目标架构：目录树、模块划分、启动流程、数据流、unsafe 策略 |
| [03-rust-rewrite-strategy.md](03-rust-rewrite-strategy.md) | C(xv6)→Rust 重写方法论与逐子系统翻译次序 |
| [10-git-history-cleanup.md](10-git-history-cleanup.md) | 无二进制历史政策 + xv6 仓库历史净化流程（附录） |
| [20-roadmap.md](20-roadmap.md) | 里程碑路线图 M0–M8：交付物、验收 gate、依赖 |
| [21-proc-sched.md](21-proc-sched.md) | 进程/调度/信号/软中断/内核线程 |
| [22-memory.md](22-memory.md) | 物理页管理/伙伴系统/slab/vmalloc/缺页/交换 |
| [23-vfs-block.md](23-vfs-block.md) | VFS/dcache/page cache/buffer/请求队列/电梯 |
| [24-ipc-smp.md](24-ipc-smp.md) | SysV IPC、管道/FIFO、SMP 与锁层次 |
| [25-drivers-net.md](25-drivers-net.md) | 驱动模型（cdev/bdev/devfs）、tty、网络（stretch） |
| [30-ext4.md](30-ext4.md) | ext4 磁盘格式与四阶段实现细节 |
| [40-testing-ci.md](40-testing-ci.md) | 测试金字塔、e2fsck 一致性闭环、CI |
| [50-risks-adr.md](50-risks-adr.md) | 风险登记册 + 架构决策记录（ADR） |

## 快速上手

```bash
make build   # 构建 i686 内核（Rust, no_std, 零依赖）
make run     # qemu 启动并校验串口 banner
make check   # 快速编译检查
```

环境要求：rustc stable（1.97+，已装 i686-unknown-linux-gnu 目标）、qemu-system-i386、binutils(ld)。
ext4 阶段另需 e2fsprogs（mke2fs/debugfs/e2fsck，开发机已具备）。

## 仓库定位

- **minios（本仓库）**：Rust 内核 + 计划 + 测试，全新历史、无二进制。
- **xv6（MoSunDay/xv6）**：MIT xv6-public C 实现，作为**只读语义参考**，不导入其历史；
  其 848 个提交中含 13.1MB 二进制（pdf/ps/ppt/tar.gz），如需公开该仓库可按附录流程净化。

## 命名

- 仓库/项目：minios；内核镜像：`kernel`（ELF32）；目标架构：i686（32 位，见 ADR-0002）。
