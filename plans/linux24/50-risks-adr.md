# 50 — 风险登记册 与 架构决策记录（ADR）

## A. 架构决策记录

### ADR-0001 内核语言选择 Rust（而非继续 C）
- 背景：用户指定 Rust；生态有 Redox/Theseus 先例。
- 决策：内核 crate 全 Rust（no_std、零第三方依赖），asm 仅限 global_asm!/asm! 内联。
- 后果(+)：内存/并发错误类别大幅前移到编译期；类型化状态机。
- 后果(−)：i686 裸机非官方一级目标（用 i686-unknown-linux-gnu 目标 freestanding 链接，
  M0 已验证可行）；少量底层代码 unsafe 摩擦。
- 状态：已接受（2026-11-14）。

### ADR-0002 目标架构 i686（32 位）
- 理由：对齐 xv6 血统与 Linux 2.4 主战场（i386 两级页表），简化迁移对照；qemu 冒烟链路最短。
- 放弃：x86-64（如果未来切换，mm/arch 边界已隔离，预计影响 <15% 文件）。
- 状态：已接受。

### ADR-0003 启动方式：multiboot 为主，xv6 式 BIOS 两阶段为辅
- 理由：M0 已用 multiboot（qemu -kernel / GRUB）打通；调试迭代快。
- xv6 式 bootasm/bootmain 作为 M8 Stretch（保留 16 位引导技能线与真机可能性）。
- 状态：已接受。

### ADR-0004 ext4 分四阶段（RO → RW 无日志 → jbd2-lite ordered → 高级特性）
- 理由：与 VFS/块层里程碑解耦，每阶段均有 e2fsprogs 权威校验闭环。
- 明确只做 ordered 数据模式；writeback/journal_checksum 为 Stretch。
- 状态：已接受。

### ADR-0005 minios 全新历史，不导入 xv6 提交历史
- 理由：任务 1 要求去除二进制提交；xv6 历史含 13.1MB 二进制且对 Rust 重写无引用价值；
  xv6 保留为只读参考。MoSunDay/xv6 的净化为独立可选项（10 号文档附录）。
- 状态：已接受。

### ADR-0006 M1 先"忠实等价翻译 xv6"，M2 起机制升级
- 理由：保住可运行语义基准（usertests 回归），避免一步到位的理想化重写失控。
- 状态：已接受。

### ADR-0007 用户态先行保持 C（xv6 用户程序），Rust 用户库为 Stretch
- 理由：ABI 稳定（int 0x40）、gcc -m32 开发环境可用；把精力集中在内核侧。
- 状态：已接受。

## B. 风险登记册（概率×影响，缓解措施）

| # | 风险 | P | I | 缓解 | 触发信号 |
|---|------|---|---|------|----------|
| R1 | 范围失控（2.4 parity 清单越做越大） | 高 | 高 | 00-scope In/Out 表冻结；Stretch 不阻塞验收 | 里程碑 gate 连续两次延期 |
| R2 | 单人产能不足，M4+ 弃坑 | 高 | 高 | 每 M 独立可演示；文档先行便于断点续作 | 单 M 超 1.5×工期 |
| R3 | ext4 写路径 bug 导致镜像损坏 | 中 | 高 | e2fsck 闭环每轮必跑；镜像模板重置制 | e2fsck 报错类目新增 |
| R4 | jbd2-lite 回放不完整 | 中 | 高 | 只做 ordered；revoke 提前实现；崩溃注入统计 | 循环测试出现非干净恢复 |
| R5 | i686 目标编译怪癖（red zone 无、eh_personality 等） | 低 | 中 | M0 已趟平并记录；锁死 rust-toolchain | toolchain 升级后构建红 |
| R6 | SMP 死锁/竞态 | 中 | 高 | 锁序文档+断言宏；先 BKL 后收缩；4 vCPU 回归 | 回归偶发挂起 |
| R7 | 网络 Stretch 拖期主线 | 中 | 中 | 网络仅 M8 后评估，未开工前不进 STATUS | — |
| R8 | 文件行数/模块膨胀违反仓库规约 | 中 | 低 | CI 行数检查；超限即拆（见 40 §5.3） | wc -l 审计违例 |
| R9 | git 历史再次混入二进制 | 低 | 中 | 四道防线（10 §1） | check-no-binaries 红 |
| R10 | qemu 与真实硬件差异（PIC/IDE 时序） | 低 | 低 | 以 qemu 为准；Stretch 真机线未承诺 | — |

## C. 决策流程

新决策：提交 PR 修改本文件（编号递增 ADR-000N）；重大方向变更需先在 STATUS.md 挂
"待批准"并征得仓库所有者确认后方可实施（破坏性 git 操作亦同，见 10 附录）。
