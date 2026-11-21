# STATUS — x86_64 / Linux 2.6.39.4 实施状态

> 本文件只报告实施与证据状态。**计划文档终审完成不等于实现完成，也不使任何 gate 变绿。**
> x86_64/Rust-only 迁移已启动：M0R 已在净树本地取证与 GitHub CI 首跑双重全绿（见 [STATUS-history/m0r-m1r.md](STATUS-history/m0r-m1r.md)）；
> 验收规则以 [40-testing-ci.md](40-testing-ci.md) 为唯一权威。

## Rollup

| 范围 | 当前状态 | 判定 |
|------|----------|------|
| Core-2.6（M0R–M8） | 🟢 M0R–M3 done（M4–M8 pending） | M0R/M1R 见 [STATUS-history/m0r-m1r.md](STATUS-history/m0r-m1r.md)；M2 判绿 HEAD `67bc6ca` + CI run 44556592736 success（[STATUS-history/m2.md](STATUS-history/m2.md)）；M3 判绿（m3 22/22、host 156、电池 6×2 字节稳定，HEAD `d4bc6a7` 树）+ `b5dfbaf` 推送触发 CI（[STATUS-history/m3.md](STATUS-history/m3.md)） |
| Parity-2.6（M9+） | ⬜ **pending** | 未启动；权威差距表、Parity 配置和外部矩阵均未验收 |

最近同步：2026-11-21。唯一实现基线为 Linux 2.6.39.4 / x86_64；不沿用旧完成度或工期。

## 里程碑状态

| 里程碑 | 交付摘要 | 状态 | 当前证据/前置 |
|--------|----------|------|---------------|
| M0R | 工具锁、Multiboot1/ELF64、长模式、高半区、真实 QEMU smoke、CI workflow | 🟢 **done**（2026-11-16 判绿） | 净树 HEAD `4c22b7b` 全套 exit 0 + 三组 sha256 + CI 首跑 success：https://github.com/MoSunDay/minios/actions/runs/43886738739 |
| M1R | syscall/FPU/UserPtr、静态 Rust ET_EXEC 用户态与到期 required 用例 | 🟢 **done**（2026-11-19 判绿） | 净树 HEAD `ef2a335` 电池 9 步 EXIT0（smoke/m1r-suite/m1r-user 35+coverage/m1r-fault/demo-full，serial 钉值 `028048dd…`/`6137aefd…`/`0c39a3e5…`/`1ba8b13a…` 全复现）；CI run 44381221846 success；详见 [STATUS-history/m0r-m1r.md](STATUS-history/m0r-m1r.md) |
| M2 | 匿名 VMA/brk/mmap/COW 与匿名 OOM | 🟢 **done**（m2r-s1–s7，2026-11-20 判绿） | 物理页基础 + VMA/brk/mmap/COW fork + 匿名 OOM 全链交付：本地电池 + CI run 44556592736（`67bc6ca`）双绿；m2 套件 16/16、m1r-user 39/39；证据见 [STATUS-history/m2.md](STATUS-history/m2.md) |
| M3 | UP PIT/PIC、CFS、信号、EPIPE/SIGPIPE、RCU | 🟢 **done**（m3r-s1–s7，2026-11-21 判绿） | CFS+RT+waitqueue+时间层+kthread/workqueue/softirq+信号+RCU 全链交付：m3-suite 22/22、m1r-user 45/45+coverage、host 156、QEMU 电池 6 目标 ×2 字节稳定（HEAD `d4bc6a7` 树）；证据见 [STATUS-history/m3.md](STATUS-history/m3.md) |
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

## 历史里程碑归档（M0R–M3）

已判绿里程碑的详细过程记录迁至 [STATUS-history/](STATUS-history/)（纯文档搬运）：
[M0R/M1R](STATUS-history/m0r-m1r.md)、[M2 m2r S1–S7](STATUS-history/m2.md)、[M3 m3r S1–S7](STATUS-history/m3.md)。
本文件保留里程碑总表与当前里程碑滚动窗口；后续里程碑判绿后按同一方式归档。

## m4r 记录（M4 滚动窗口：VFS / page cache / BIO / 最小设备语义）

### m4r-s0（开工治理：M3 归档 + Rollup/里程碑表同步，2026-11-21）

- P1：推送 `c4acd59`+`b5dfbaf` 至 origin 触发 CI（M4 开工基线）。
- P2：① m3r s1–s7 记录归档至 [STATUS-history/m3.md](STATUS-history/m3.md)
  （滚动窗口规约首次对 M3 执行，纯文档搬运）；② Rollup 行修正
  「M0R–M1R done（M2–M8 pending）」→「M0R–M3 done（M4–M8 pending）」并
  同步「最近同步」日期；③ 里程碑表 M2 行 🟡 → 🟢（m2r-s7 已判绿，表未同步，
  上轮遗留观察闭环）。
- 纯文档变更，零代码风险；本树复验 make all / fmt-check / check-rust-only /
  host 电池与 m3r-s7d 树一致。
