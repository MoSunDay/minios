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

### m4r-s1（VFS 纯核：四 ops vtable + dcache/icache + namei 纯逻辑，2026-11-21）

- 交付：`fs/vfs/{mod,ops,dentry,dcache,devict,icache,namei}.rs`（7 文件全部
  ≤400 行，纯核零内核依赖，host include-by-path 直测）：
  - **ops.rs**：四 ops vtable（SuperOps/InodeOps/FileOps/AddressSpaceOps，
    struct+fn 指针，无 class）+ SuperBlock/InodeCommon/FileCommon 公共头
    （fs 私有尾经 offset 回收，RcuHead container_of 同纪律）；M4 Core
    FileOps 最小集冻结（23 §1：AIO/sendfile/inotify 为 M10 禁入）。
  - **dentry/dcache/devict**：dentry 树（sibling hlist + d_first_child 分离，
    Linux d_subdirs 同构）、负 dentry（缓存/原位转正/淘汰计数）、rename 失效
    （占位者摘链 + 空闲子树回收 + 引用者存活语义）、ref/LRU 不变量
    （d_ref==0 ⇔ 在 LRU，d_on_lru 显式）、叶序 prune、池耗尽自动回收。
  - **icache**：(dev,ino) 哈希 + idle LRU + prune-dispose 回调（内核侧接
    fs destroy_inode）。
  - **namei**：PathCursor 组件切分（多斜杠折叠/尾部斜杠/NAME_MAX）、
    dac_allows（owner/group/other + root 特例：exec 需至少一个 x 位）、
    may_search（目录 + x 位）、SymlinkBudget（ELOOP@40，Linux 值）。
- host vfstests+dcachetests 17 例（路径切分/DAC 矩阵/ELOOP/负 dentry 生命
  周期/rename 三态/replace 占位者/LRU 叶序/池回收/icache iget-iput-prune/
  vtable 分发+泛型 DAC 回退）；host 电池 156→173（96+72+5）。
- 修真实缺陷 1 项：d_next 同字段复用「兄弟链/首孩子头」致树环（host
  钉死后拆 d_first_child，Linux d_subdirs 同构）。
- gate：make all 0 警告 / fmt-check / check-rust-only（files=172）绿；
  QEMU 电池 6 目标 ×2 全 PASS 且 serial/root 钉值与 m3 收口完全一致
  （m0r `028048dd…`、m1r-suite `6137aefd…`/`341ae68a…`、m1r-user
  `da87aa63…`/`45cf8a2b…`、m1r-fault `855f856a…`/`f702cdf2…`、m2
  `d376e5b4…`、m3 `e7b5a2fc…`）——纯核经 LTO 消除，无镜像移位，零重钉。
- dcache 锁层次（UP：DENTRY_LOCK 与 PTABLE/RCU_LOCK 关系）为 ADR-0023
  素材，s2 落账；负 dentry 淘汰经 call_rcu 由 s2 胶水包（首个内核消费者）。

### m4r-s2a（syscall.rs 四分拆：dispatch/mm/proc/time，2026-11-21）

- 交付：`kernel/src/kernel/syscall.rs`（569 行）→ `kernel/src/kernel/syscall/
  {mod,mm,proc,time}.rs`（95/100/244/141 行，全 ≤400），**纯代码搬运**（排序
  行多重集 diff 证明仅路径前缀/可见性/import 拆分/`super::signal`→
  `crate::kernel::signal` 四类接线差异，逻辑零改动）：
  - **mod.rs**：`dispatch()` 分发表 + errno `pub use` 面 + SYS_* 引入 +
    `pub use proc::on_tick`（time/mod.rs 既有 `super::syscall::on_tick`
    调用点保持不动）；callee 改 `mm::`/`proc::`/`time::` 前缀。
  - **mm.rs**：sys_brk/sys_mmap/sys_munmap/sys_mprotect（pub(super)）。
  - **proc.rs**：sys_getpid/sys_wait4/sys_execve/sys_sched_* 族 +
    sys_sched_prio_bound（pub(super)）；on_tick 保 pub；sched_target/
    sched_valid 私有。
  - **time.rs**：sys_gettimeofday/sys_alarm/sys_nanosleep（pub(super)）；
    nanosleep_wake 私有。
- 动机（brief 风险②预案）：s2b 新 syscall（dup2/fcntl/F_SETFD 等）落地前
  先拆表控复杂度，file.rs/ipc.rs 随 s2b 增量加入。
- gate：make all 0 警告 / fmt-check / check-rust-only（files=172→175）/
  test-host 173（96+72+5）绿；QEMU 电池 6 目标全 PASS，serial/root 9 项
  证据钉值与 m4r-s1 参考逐字节一致（m0r `028048dd…20f1`、m1r-suite
  `6137aefd…e585`/`341ae68a…15ce`、m1r-user `da87aa63…f6f6`/
  `45cf8a2b…f70`、m1r-fault `855f856a…808`/`f702cdf2…8ca5`、m2
  `d376e5b4…d019`、m3 `e7b5a2fc…1b1e`）。内核 ELF 哈希随符号路径变化
  （`767e4ae7…`→`eb323447…`，预期非钉值）。

### m4r-s2b（VFS 内核胶水：vfs_glue 五文件 + 全路径接线 + m1r-user 泄漏攻坚，2026-11-21）

- 交付：`kernel/src/fs/vfs_glue/`（mod 397 / mutate 377 / xv6_adapt 374 /
  xv6_obj 141 / wrapper 122，全 ≤400）——dcache(256) wrapper 钉 legacy inode
  表(NINODE=50) 引用的完整胶水：walk/step（锁内 dcache、锁外 fs op、重查竞态
  收敛冗余 wrapper）、kern_create/kern_unlink/kern_link 三相（resolve+pin /
  fs-op unlocked / cohere）、zombie（nlink=0 最终 iput 延迟到进程上下文
  begin_op 事务）+ call_rcu 延迟槽回收（ADR-0022 首个真实消费者）。接线：
  main 挂载根 dentry、syspath 全量改走 kern_*、exec/sys_execve/m1r-suite
  case_fs_file 改 kern_namei_c、proc fork/exit/chdir 持 cwd dentry 引用、
  dcache 加 dispose 钩子+alloc 空闲链 prune+invalidate/recycle、inode.rs
  新增 `free_slots()`、abi 补 ELOOP。legacy namei 保留为参考移植
  （#[allow(dead_code)]）。
- 关键纪律：`mod.rs` 自由函数 `dput()` 更名 `dput_unlocked` 并文档化——持
  DENTRY_LOCK 临界区内唯一合法拼法是 `dc().dput(d)`（非重入自旋锁，误用即
  自锁死；本轮实际死锁根因）。
- m1r-user 连修六类 bug 后 45/45（createtest/rmdot/fourteen/bigwrite/bigfile/
  subdir/linkunlink/linktest/unlinkread/createdelete…全 PASS）：
  1. iget 表穷尽 → 新增 `relieve_inode_pressure(need)`（DENTRY_LOCK 下
     `while free_slots<need { prune_lru(1) }`，LRU 干则 break），接线
     step_locked lookup 前 + create/unlink/link phase B 前；
  2. fourteen → legacy skipelem/namecmp 只比较前 DIRSIZ 字节：parent_locked
     终组件、step_locked 每 comp 入口截断；
  3. ilock ref<1 panic → `kern_namei_d` 契约 = 返回 (idup 过的 ip, pinned d)
     （chdir/exit 配平；不 idup 则 proc.cwd 吃 wrapper 引用→双重 iput）；
  4. subdir "dd/ff/ff" file-作-父探针 → create_common/xv6_unlink/xv6_link
     三写者补 `dp->type != T_DIR` 检查（折 EEXIST/ENOENT/回退 nlink）；
  5. walk 全部 Err 路径 `dc().dput(cur)`、step_locked insert-Err
     release_wrapper（否则泄漏钉死 root/iget 表）；
  6. **最终根因（本轮主攻坚）**：`kern_namei` 在 `kern_namei_d` 已 idup 的
     结果上再次 idup——每次成功 lookup 净 +1 引用泄漏，电池累计 59 个恰好
     耗尽 50 槽表（DS=59 与泄漏量相等纯属巧合）。诊断方法：iget/idup/iput/
     wrap_new/release/zombie/dispose 逐 inum 打点，事件台账对账（孤立
     createdelete 即复现：每个幸存文件净 +1，验证期 open 呈 d/d/u 双取单还）。
     修法：kern_namei 直通返回 kern_namei_d 的 idup 引用。同类清理：
     create_common EEXIST 分支与 xv6_unlink NotEmpty 分支的 dirlookup 引用
     补 iput（先误用 iunlockput 触发 iunlock panic——ilock/iunlock 对已闭合，
     必须纯 iput）。
- gate：make all 0 警告 / fmt-check / check-rust-only（files=175→180）/
  test-host 176（99+72+5）；QEMU 电池 7 目标全 PASS（smoke×2/m1r-suite/
  **m1r-user 45+coverage**/m1r-fault/m2 16/m3 22）。
- 钉值重钉（4 项，均归因）：m1r-suite root `341ae68a…`→`312ff398…`、
  m1r-user root `45cf8a2b…`→`5a040446…`（wrapper 钉引用改变 iget/ialloc 时
  序→盘上 inode 分配序变）；m1r-fault serial `855f856a…`→`cc198eaa…`
  （内核 ELF 变化→RIP 偏移）；m2 serial `d376e5b4…`→`a97db394…`（胶水池
  改变启动空闲页水位行）。**不变（强证据）**：m0r/m1r-suite/m1r-user/m3
  serial 与 s2a 逐字节一致——尤其 m1r-user serial `da87aa63…f6f6` 意味着
  胶水路径在 45 用例全电池上与 legacy namei 行为字节等价。

### m4r-s2b-2（fd 表补全：dup2/fcntl/F_SETFD/close-on-exec，2026-11-21）

- 交付（计划 23-vfs-block「每任务 fd 表支持 dup/dup2/fcntl/close-on-exec」）：
  - abi：`SYS_DUP2=33`/`SYS_FCNTL=72`（asm-generic x86-64）、`F_GETFD=1`/
    `F_SETFD=2`/`FD_CLOEXEC=1`；
  - proc：`ofile_flags: [u8; NOFILE]` 每任务 fd 标志位（allocproc/freeproc
    清零、fork 继承、close 清除——POSIX close(2) 语义）；
  - sysfile：`sys_dup2`（Linux 语义：oldfd==newfd 仅校验原样返回、目标被占
    先 close、新 fd CLOEXEC 清零）、`sys_fcntl`（F_GETFD 返回 /
    F_SETFD 掩码置位、其余 cmd → EINVAL——M4 Core 面，F_GETFL/F_SETFL 随
    文件标志位工作增量）；
  - sys_execve 成功提交点（exec_elf Ok 后）关闭全部 FD_CLOEXEC fd（POSIX
    exec 语义；fileclose 在进程上下文、锁已释放）；
  - dispatch 两臂 + user/src/sys.rs dup2/fcntl 包装。
- gate：make all 0 警告 / fmt / check-rust-only files=180 / test-host 176；
  QEMU 电池 7 目标全 PASS。**钉值零漂移**：9 项 serial/root 与 m4r-s2b
  逐字节一致（m1r-fault serial 与 m2 serial 未随内核 ELF/内存布局移动：
  故障 RIP 为符号相对偏移且新增代码未改变相关布局，水位行不受 bss 增量
  影响）——纯增量提交的理想形态。
