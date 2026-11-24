# STATUS — x86_64 / Linux 2.6.39.4 实施状态

> 本文件只报告实施与证据状态。**计划文档终审完成不等于实现完成，也不使任何 gate 变绿。**
> x86_64/Rust-only 迁移已启动：M0R 已在净树本地取证与 GitHub CI 首跑双重全绿（见 [STATUS-history/m0r-m1r.md](STATUS-history/m0r-m1r.md)）；
> 验收规则以 [40-testing-ci.md](40-testing-ci.md) 为唯一权威。

## Rollup

| 范围 | 当前状态 | 判定 |
|------|----------|------|
| Core-2.6（M0R–M8） | 🟢 M0R–M3 done（M4–M8 pending） | M0R/M1R 见 [STATUS-history/m0r-m1r.md](STATUS-history/m0r-m1r.md)；M2 判绿 HEAD `67bc6ca` + CI run 44556592736 success（[STATUS-history/m2.md](STATUS-history/m2.md)）；M3 判绿（m3 22/22、host 156、电池 6×2 字节稳定，HEAD `d4bc6a7` 树）+ `b5dfbaf` 推送触发 CI（[STATUS-history/m3.md](STATUS-history/m3.md)） |
| Parity-2.6（M9+） | ⬜ **pending** | 未启动；权威差距表、Parity 配置和外部矩阵均未验收 |

最近同步：2026-11-24。唯一实现基线为 Linux 2.6.39.4 / x86_64；不沿用旧完成度或工期。

## 里程碑状态

| 里程碑 | 交付摘要 | 状态 | 当前证据/前置 |
|--------|----------|------|---------------|
| M0R | 工具锁、Multiboot1/ELF64、长模式、高半区、真实 QEMU smoke、CI workflow | 🟢 **done**（2026-11-16 判绿） | 净树 HEAD `4c22b7b` 全套 exit 0 + 三组 sha256 + CI 首跑 success：https://github.com/MoSunDay/minios/actions/runs/43886738739 |
| M1R | syscall/FPU/UserPtr、静态 Rust ET_EXEC 用户态与到期 required 用例 | 🟢 **done**（2026-11-19 判绿） | 净树 HEAD `ef2a335` 电池 9 步 EXIT0（smoke/m1r-suite/m1r-user 35+coverage/m1r-fault/demo-full，serial 钉值 `028048dd…`/`6137aefd…`/`0c39a3e5…`/`1ba8b13a…` 全复现）；CI run 44381221846 success；详见 [STATUS-history/m0r-m1r.md](STATUS-history/m0r-m1r.md) |
| M2 | 匿名 VMA/brk/mmap/COW 与匿名 OOM | 🟢 **done**（m2r-s1–s7，2026-11-20 判绿） | 物理页基础 + VMA/brk/mmap/COW fork + 匿名 OOM 全链交付：本地电池 + CI run 44556592736（`67bc6ca`）双绿；m2 套件 16/16、m1r-user 39/39；证据见 [STATUS-history/m2.md](STATUS-history/m2.md) |
| M3 | UP PIT/PIC、CFS、信号、EPIPE/SIGPIPE、RCU | 🟢 **done**（m3r-s1–s7，2026-11-21 判绿） | CFS+RT+waitqueue+时间层+kthread/workqueue/softirq+信号+RCU 全链交付：m3-suite 22/22、m1r-user 45/45+coverage、host 156、QEMU 电池 6 目标 ×2 字节稳定（HEAD `d4bc6a7` 树）；证据见 [STATUS-history/m3.md](STATUS-history/m3.md) |
| M4 | VFS/page cache/file-backed mmap、回收、tmpfs、tty、BIO/flush | 🟡 **进行中**（m4r-s0–s4a-2，2026-11-24） | VFS 纯核+内核胶合+fd 表补全+m4suite 脚手架+page cache 核与读路径+uptodate 忙等门+sys_read 改道（影子字节等价）+双页窗 readahead 已交付：host 197、QEMU 电池 8 目标 PASS、m4 17/17（钉 `86af3590…`）+fd 入口永久用例 m4.pc.fd_read（review 三项 P0 钉死）；ADR-0024/0025；详见 m4r 滚动窗口 |
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

### m4r-s2c（M4 内核态套件脚手架：m4suite + runner 变体 + test-qemu-m4 + walk gate 修正，2026-11-21）

- 交付（计划 20-roadmap M4 s2c；ADR-0023 补档）：
  - kernel：`kernel/src/kernel/m4suite/{mod,vfs,fsops}.rs`（108/97/161 行）——
    boot 上下文套件，run 入口 `io::sti()` 后跑 9 例合并判决，exit
    0x10/0x11；共享 `drained_slots()`（zombie 排干后读 free_slots——
    unlink 族延迟末次 iput，台账只在排干后结算）与 `with_inode()`
    （探测后必还引用，测试自身不钉槽位）；
  - wave-1 用例（绝对路径 / m4 前缀 ≤DIRSIZ / 每例自清理 + 台账钉）：
    namei 4——`root_paths`（"/", "/.", "/..", "/../.." 全 ROOTINO）、
    `dirsiz_trunc`（15 字符拼写↔14 前缀互认=skipelem 截断、13 前缀
    NoEnt）、`negative_flip`（miss→create→hit→unlink→miss 台账平）、
    `file_as_parent`（file 中段 NotDir，namei 与 create 双 Err）；fsops
    5——`create_eexist`（dir 重建 + file-over-dir 双 EEXIST + 失败路径
    free_slots 不变=s2b EEXIST 泄漏钉）、`dir_dots`（. 自指 / .. 归根）、
    `link_two_names`（同 inum、删一存一、末删 zombie 排干台账回基线）、
    `unlink_notempty`（NotEmpty + dirlookup 引用已回还钉）、
    `table_pressure`（80 文件>NINODE=50 全 Ok 全删台账回基线=
    createdelete 回归类）；
  - main.rs：BootMode::M4Suite（cmdline `m4-suite`，置于 Full fallback
    前）——set_active_suite("m4") → init_m1r_fs()（真盘）→ m4suite::run()；
  - runner：Suite::M4Suite（needs_root=true 走 M1R 盘套件路径、
    CONFIG_ID_M1R）、`run-m4-suite` verb + usage、`verdict/m4.rs` spec
    （9 例冻结表 + 5 个红绿自测）；
  - Makefile：M4_SUITE_{CFG,DIR,ISO,RUN}、`m4-suite-iso`（make-m1r-iso
    宏）、`test-qemu-m4`（ROOT_TEMPLATE 拷贝 + --root + sha256sum）、
    .PHONY；`scripts/m4-suite-grub.cfg`。
- **真实缺陷（首跑钉出，已修）**：walk_locked 搜索 gate 把 may_search 的
  NotDir 改写为 Access（s2b 遗留）——syscall 边缘对 file 中段路径错报
  EACCES 而非 legacy ENOENT 折叠，且违背 vfs_glue 模块文档「ENOTDIR
  surfaces with the m4suite walk cases」的既定意图。修为原样传播
  （errno_of 的 NotDir→ENOENT 折叠不变，电池字节稳定得以保持）。
  诊断：m4.namei.file_as_parent 首跑 8/9，临时打点实证 WALK=Access /
  MK=NotDir / 台账平，即刻定位。
- gate：make all 0 警告 / fmt-check / check-rust-only files=180→183 /
  test-host 176→181（99+77+5，m4 spec 自测 +5）；
- QEMU 电池 **8 目标全 PASS**（新增 test-qemu-m4：**9/9**，serial 钉
  `20996882f598734911efda98719af6106a88d07c2432b5366b4b089d5fd58460`）。
- 钉值重钉（2 项，worktree bde4a5e 重建逐行 diff 归因）：m2 serial
  `a97db394…`→`49180fd2…`（仅 2 行：zones DMA=0x872→0x870 与 directmap
  free-frame 诊断对 0xf843/0xf84b→0xf841/0xf849——内核镜像 +2 页致启动
  空闲页水位平移，+8 关系保持）；m1r-fault serial `cc198eaa…`→
  `fa350131…`（全部差异行仅 `rust_main+0x1242→+0x1287` 回溯偏移与 rodata
  页 `0x…2c000→0x…2e000`——main.rs 新增 boot 臂的代码布局效应）。
  **不变（强证据）**：m0r/m1r-suite/m1r-user/m1r-fault-root/m1r-user-root/
  m1r-suite-root/m3 共 7 项 serial/root 钉逐字节一致——含 gate 修正
  （NotDir 原样传播）下 m1r-user serial 仍 `da87aa63…`，证明冻结电池
  无任何用例走 file 中段路径，legacy 字节等价持续成立。

### m4r-s3（page cache 纯核 + 读路径接线：pagecache 四文件 + pcache 胶合 + m4suite wave-2，2026-11-22）

- 交付（计划 20-roadmap M4 s3a+s3b；ADR-0024）：
  - 纯核 `kernel/src/fs/vfs/pagecache/`（4 文件 565 行：mod 58 / page 130
    / tree 206 / lru 171）：`AddressSpace{a_ops,host,root,nrpages}`、
    `PageDesc` + const `PageCacheStats::ZERO`、PageCache 引擎（池内
    PageDesc 数组 + free list；insert/lookup/remove、note_readpage/
    note_readahead 计数钩子）、LRU（ref/put 引脚语义、FIFO 回收净页
    跳过脏页——23 §2 不丢已写数据、invalidate_mapping 逐映射作废）；
  - 索引结构 = per-mapping index 有序侵入链表——对 Linux xarray 的
    主动裁剪（xv6fs MAXFILE=140 块 ≈18 页/文件，dcache 线性扫先例），
    已记 ADR-0024 divergence；lookup 返回 pin（find_get_page 契约）；
    insert 池满自 LRU 回收最旧净页（NPAGE_DESC=256 ≈1 MiB 缓存）；帧
    kalloc + `free_data` 回调（内核侧 = kfree）；锁序 DENTRY_LOCK →
    PC_LOCK → KMEM_LOCK，PC_LOCK 绝不跨睡眠 fs 调用（readpage 持
    ilock 可睡 → insert-then-unlock-fill）；
  - `ops.rs`：InodeCommon 内嵌 `i_data: AddressSpace` + `i_mapping`
    指针（Linux 形态，mapping 由 wrapper/dentry 1:1 持有）；
    AddressSpaceOps 重钉 page 类型（`*mut AddressSpace`/`*mut PageDesc`）；
  - host 测试 `host-tests/src/pagecachetests.rs` 14 例（369 行；含
    double-put panic、FIFO 回收序、脏页存活、池压自回收、计数器对账）；
  - 内核胶合 `vfs_glue/pcache.rs`（281 行）：PC 静态池 + PC_LOCK、
    init（free_data=kfree）、`kern_read_cached`（读走缓存）、
    `kern_page_probe`/PageProbe、`kern_pc_reclaim`、`pc_stats`、
    `kern_readahead`（#[allow(dead_code)]，s4 预留）；`wrapper.rs`
    wrap_new 绑 mapping（仅 T_FILE 挂 a_ops=&XV6_ASPACE_OPS）+
    wrapper_free 作废 mapping 并断言 nrpages==0；`xv6_obj.rs`
    XV6_ASPACE_OPS + xv6_readpage（readi + EOF 尾零填 + uptodate，
    写侧 NotSup 桩）；`buffer.rs` 加 `bread_count()` 门钩；
  - m4suite wave-2 `m4suite/pc.rs` 4 例（262 行）：`hit_miss`（首读
    miss→bread+1，二读命中 bread 不变）、`share_handles`（两 fd 同页
    共享）、`fill_matches_readi`（缓存页逐字节 == readi 直读）、
    `evict_clean`（回收后重读仍正确）；make_file 改每 512B chunk 一
    事务（xv6 LOGSIZE=30 约束，原单事务 16 KiB 写 panic "too big a
    transaction"）；mod.rs 例表 9→13；runner verdict/m4.rs spec 同步；
  - 评审遗留修复：fsops dirsiz_trunc 拆卸改无条件 `kern_unlink`（不以
    unlink 结果分支泄漏探测文件）；vfs press_name `assert!(i<100)`
    防御环。
- scope 判定（brief 默认）：净页 LRU 回收纳入 s3 ✓；readahead 仅计数
  钩子（窗口算法归 Parity）；sys_read 冻结 legacy fileread（字节等价
  证明留 s4/s5 决断）。
- gate：make all 0 警告 / fmt-check / check-rust-only files=183→190 /
  test-host 181→195（113+77+5，+14 pagecache）；runner 77 自测 PASS。
- QEMU 电池 **8 目标全 PASS**；m4 套件 **13/13**，serial 新钉
  `13740dda8cf154c0610f160a0cb6a8b2caae1ba4d03d05274f072811942de90f`
  （复跑稳定）；demo-full PASS（serial `18dbc99a…`，非钉值）。
- 钉值：**7 项逐字节不变**（m0r serial `028048dd…20f1`；m1r-suite
  serial `6137aefd…e585` + root `312ff398…f12e`；m1r-user serial
  `da87aa63…22f6` + root `5a040446…f180d`；m1r-fault root
  `f702cdf2…8ca5`；m3 `e7b5a2fc…1b1e`）——sys_read 冻结在 legacy
  fileread、缓存路径仅 m4suite 并行取证，legacy 字节等价持续成立。
  重钉 2 项（worktree `94836ec` 重建先复现旧钉、逐行 diff 归因后
  移除）：m2 serial `49180fd2…`→`4ed54600…`（仅 2 行：zones DMA=0x870→0x867
  与 directmap 对 0xf841/0xf849→0xf838/0xf840——内核镜像 +9 页致启动
  空闲页水位平移，+8 关系保持）；m1r-fault serial `fa350131…`→
  `ad8e1f58…`（4 行，全部仅 cr2 rodata 页
  `0xffffffff8002e000→0x…80031000`，代码偏移逐行相同）。

### m4r-s3-2（m4r-s3 静态 review 收口：两项 P1 修复 + uptodate 哨兵 + ADR-0024 补记，2026-11-22）

- 修复（review 判定 2 项 P1 潜伏缺陷，均 s4 前必须清障；代码仅
  `vfs_glue/pcache.rs`）：
  - **P1-1 锁序**：`kern_page_probe` 收尾 `finish_refs(ip,d)` 原持
    PC_LOCK 调用——dput 取 DENTRY_LOCK 构成 PC→DENTRY 嵌套倒置，且
    dput 触发 prune → `wrapper_free` 再取 PC_LOCK 对非重入自旋锁递归
    自死锁；现照 `kern_read_cached` 收尾形态将 guard 作用域收在
    `out.nrpages` 后、finish_refs 移出 PC_LOCK（模块头锁序注释同步）；
  - **P1-2 哨兵+补记**：`read_page_bytes` 命中快路径加
    `debug_assert!(uptodate)`（触发即填页竞态真实可达）；ADR-0024 追加
    「已知缺口」——insert-then-unlock-fill 填页窗口并发 lookup-hit 可见
    未填帧，M4 不可达（boot 单上下文顺序调用），**s4 brief 必须钉死**
    等待机制（uptodate 门/waitqueue/页锁三选一）与 sys_read 改道的
    m1r-user 字节等价证明时点。
- 明确不做（防 scope 蔓延）：m4suite「dcache 压力下 probe」与填页窗口
  用例——随 s4 等待机制一并落（例表届时统一重钉）。
- gate：fmt-check / make all 0 警告 / check-rust-only files=190 不变 /
  test-host 195（113+77+5）/ runner 77 自测 PASS。
- QEMU 电池 **8 目标全 PASS**（m0r 6/6、m1r 20/20、m1r-user+coverage、
  m1r-fault 5/5、m2 16/16、m3 22/22、**m4 13/13**、demo-full）。
- 钉值：**9 项 serial/root 全部逐字节零漂移**（含 m4
  `13740dda…de90f`、m2 `4ed54600…`、m1r-fault serial `ad8e1f58…`
  ——修复不触输出路径，预测成立；无镜像页边界平移，无重钉）。
- 结论：review NOT READY 的两项 P1 已清障，**go-live ready**；下一步
  s4（等待机制 + sys_read 改道 + readahead 激活）。

### m4r-s4a（uptodate 忙等门 + sys_read 改道 page cache + readahead 激活 + m4suite wave-3，2026-11-22）

- 交付（ADR-0025；上游 brief：等待机制三选一、改道字节等价证明、
  readahead 双页窗、例表 13→16）：
  - **核心**：`pagecache/tree.rs` 新 `Peek{Absent, Filling(*mut),
    Ready(*mut)}` + `peek()`（无计数无 pin 的门分类；Filling 携指针是
    关窗翻转的前提——初版不带指针是本轮真实 bug 之二）；mod.rs 再导出。
  - **胶合 `vfs_glue/pcache.rs` 重构**：`read_page_bytes`→`VfsResult<bool>`
    过门；`wait_gate_copy`（GATE_SPIN_MAX=1_000_000 忙等 + yield_cpu +
    活锁 panic 哨兵）；`fill_page`；`read_cached_range`（readi 精确
    clamp）；`readahead_fill`；`kern_read_cached` 重构上 wrapper_get；
    `kern_read_cached_ip`（fd 入口，扫描绑定/null 回退 read_uncached）；
    套件钩子 `kern_gate_peek`/`kern_window_set`。
  - **wrapper 生命周期 `wrapper.rs`**：`i_count`（读者引用，icache 语义）
    + `dying` 延迟释放（unlink negate-in-place / prune 不可能在途读者
    脚下释放映射；最后一 put 完成 release：nlink==0→zombie，否则
    iput+free）+ `epoch` 槽代（readahead hint 槽复用假阳性 guard）+
    `wrapper_get_for_ip`/`wrapper_get`/`wrapper_put`。
  - **`readahead.rs` 新模块**：RA_WINDOW=2、RaHint/note_page/window/reset。
  - **sys_read 改道 `sysfile.rs`**：T_FILE fd → sys_read_cached（同
    BSIZE 分块/EOF 短读/部分 EFAULT/consume-then-copy 形状）；管道/
    设备/目录冻结 sys_read_legacy。
  - **m4suite wave-3（`pc.rs`+mod.rs+verdict/m4.rs，表 16）**：
    gate_window、readahead_seq、probe_pressure（120 文件 churn 经
    50 槽 inode 表 relief 阀驱逐最老 idle dentry→fresh wrapper 零页→
    重读正确）；write_name 辅助；hit_miss 计数注释更新。
  - host 测试 +2（peek 门态+逐索引+计数零污染）→ 197。
- 攻坚记录：m4 首跑 15/16——gate_window FAIL 后 probe_pressure churn
  期 `iget: no inodes` panic。bisect（跳过 gate_window 复跑）分离出
  两枚独立真实 bug：①probe_pressure 泄漏（kern_create Ok(ip) 未 iput，
  每文件泄一 iget 引用；且 300 并发>200 磁盘 inode 预算→ialloc panic，
  修正=平衡 iput+churn 120）；②Peek::Filling 无指针→kern_window_set
  关窗静默 no-op（修正=对称携带指针，双向翻转）。两修后 16/16。
- **影子字节等价证明（ADR-0025 时序：落库前先行）**：sys_read_cached
  内临时打点——每 BSIZE 窗口回卷 f.off 经 legacy fileread 复放，计数+
  逐字节+EOF/错误分类全对账，mismatch 即 panic；8 目标电池全 PASS 零
  panic 后打点移除（净树无 TEMP 残留）。
- gate：cargo fmt --all -- --check / make all 0 警告 / check-rust-only
  files=190→191（readahead.rs）/ test-host 197（115+77+5）/ runner 77。
- QEMU 电池 **8 目标全 PASS**；m4 套件 **16/16**，serial 新钉
  `49804f50027d8a9c252cb6b3bee573b90b6ccf8104c52bb77d8bdd0862fad793`。
- 钉值：**7 项逐字节不变**（m0r serial `028048dd…20f1`；m1r-suite
  serial `6137aefd…e585` + root `312ff398…f12e`；**m1r-user serial
  `da87aa63…22f6f` + root `5a040446…f180d`——sys_read 改道零漂移**；
  m1r-fault root `f702cdf2…8ca5`；m3 `e7b5a2fc…1b1e`）。重钉 2 项
  （worktree 1891877 复现旧钉后逐行 diff 归因）：m1r-fault serial
  `ad8e1f58…`→`96d35874…`（4 行全部仅 cr2 rodata 页
  0x…80031000→0x…80033000，镜像 +2 页；RIP/backtrace 逐字节同）；
  m2 serial `4ed54600…`→`13aaf67c…`（2 行：zones DMA=0x867→0x863 与
  directmap 0xf838/0xf840→0xf834/0xf83c，镜像 −4 页水位平移，+8 关系
  保持）。
- 结论：ADR-0024 两项待决已关闭（ADR-0025），s4a **go-live ready**；
  下一步 s4 续（mmap 文件页 / 回收深化 / tmpfs·tty·BIO flush 按 brief）。

### m4r-s4a-2（s4a review 收口：fd 入口永久用例 m4.pc.fd_read，表 16→17，2026-11-22）

- 背景：s4a 静态 review 判定三项 **P0 测试缺口**（生产路径零永久
  覆盖）：① fd 入口 `kern_read_cached_ip`（`pcache.rs`）无任何内核
  套件用例——wave-2/3 全走 path 入口 `kern_read_cached`；② 扫描 miss
  → `read_uncached` 回退分支在影子打点删除后无永久用例（该分支是
  「无 dentry 遮蔽 inode」时生产读的唯一路径）；③ readi 精确 clamp
  分类（off>size→`Err(Io)` / off==size→`Ok(0)` / 尾部 clamp）无断言。
- 交付（`m4suite/pc.rs` + `m4suite/mod.rs` + `verdict/m4.rs`，表
  16→17）：新例 `m4.pc.fd_read`（`/m4pcfdr`，PGSIZE+512=1.5 页）一例
  钉死三项 P0：
  - 预热形状：readahead reset 后 read_verify——misses+2 /
    readpages+2 / inserts+2、双页窗 clamp 至空（尾页=idx1）；
  - **fd 扫描命中**：`kern_namei` 自持 idup 引用（跨 unlink 存活）+
    `kern_read_cached_ip` 全页读：字节==pat 且 hits 恰 +1、misses
    不动；
  - **clamp 分类**：off=size+1→`Err(VfsErr::Io)`、off==size→
    `Ok(0)`（两条预页路径计数冻结）、off=size−256 len 512→`Ok(256)`
    字节==pat（恰一次常驻页 hit）；
  - **回退确定性构造**：unlink 叶子→dentry Detached（RCU retire）+
    zombie 入队；`drained_slots()` 借 `reclaim_zombies` 排空→
    `wrapper_free` 清 `i_mapping`→扫描必 miss→readi 直读全页+尾部
    字节==pat；期间 pc_stats（hits/misses/inserts/readpages）全程
    冻结 + `bread_count` 严格递增（真实盘流，缓存零可见）；
  - 收尾：截断 iput 包事务（`reclaim_zombies` 先例）+ 台账归位；
    teardown（unlink/iput）在所有路径无条件执行——本例早败不泄
    slot/引用（套件其余用例保持既有形状，记录不动）。
- P1/P2 记录不动（review 判定不阻塞）：用例早败 teardown 泄漏（套件
  既有形状）；`kern_read_cached_ip` 每 chunk O(128) 全池扫描（性能项，
  s8 waitqueue 重评一并处理）；`kern_window_set` 开窗被读路径穿越→
  活锁 panic（boot 单上下文用例序不可达，by-design 响亮失败）；
  `readahead_fill` absent 检查与 `fill_page` 间 TOCTOU（单上下文不可
  达，s8 并发下 insert-Busy 自愈）。
- gate：cargo fmt --all -- --check / make all 0 警告 / check-rust-only
  files=191（无新文件）/ test-host 197（115+77+5）/ runner 77。
- QEMU 电池 **8 目标全 PASS**；m4 套件 **17/17 首跑即绿**，serial 新钉
  `86af35904794752c875bed6835956e42b8de17cd746632358b742ac52fede603`。
- 钉值：**7 项逐字节不变**（m0r serial `028048dd…20f1`；m1r-suite
  serial `6137aefd…e585` + root `312ff398…f12e`；m1r-user serial
  `da87aa63…22f6f` + root `5a040446…f180`；m1r-fault root
  `f702cdf2…8ca5`；m3 `e7b5a2fc…1b1e`）。归因重钉 2 项（worktree
  `9c20601` 隔离 target 复现旧钉后逐行 diff）：m2 serial
  `13aaf67c…`→`07e82d5e…`（2/20 行：zones DMA 0x863→0x862 与
  directmap 0xf834/0xf83c→0xf833/0xf83b——内核镜像 +1 页水位平移，
  +8 关系保持）；m1r-fault serial `96d35874…`→`f38db2be…`（4/14 行
  全部仅 cr2 rodata 页 0x…80033000→0x…80034000，RIP/backtrace 逐字节
  同）。
- 结论：三项 P0 已永久钉死，s4a-2 **go-live ready**；下一步 s4 续
  （mmap 文件页 / 回收深化 / tmpfs·tty·BIO flush 按 brief）。

### m4r-s4a-2b（plans 泄露 todo 清扫：里程碑表滞后 + README 陈旧声明闭环，2026-11-24）

- 背景：对 plans/ 全目录做待办泄露清扫（TODO/FIXME/HACK/XXX 标记、
  `- [ ]` 复选框及「尚未迁移/未完成」类陈旧声明逐一核查），判定 2 处
  真实泄露、10 处关键词命中非泄露。
- 泄露 1（里程碑表 M4 行，滞后一轮）：仍写 m4r-s0–s4a / m4 16/16（钉
  `49804f50…`）——s4a-2 已交付（17/17 首跑即绿、新钉 `86af3590…e603`、
  fd 入口永久用例钉死三项 P0）。本轮同步为 m4r-s0–s4a-2 / 17/17（钉
  `86af3590…`）+ 摘要补记 m4.pc.fd_read，并同步「最近同步」日期
  （照 m4r-s0 对 M2 行同类表滞后的闭环先例）。
- 泄露 2（README.md 目标边界，M0R 前陈旧声明）：「当前代码尚未迁移到
  本目标，仍是原 i686/C 用户态实现」与 Rollup「M0R–M3 done」直接矛盾
  （内核与用户态均为 Rust，check-rust-only 门禁在位）；两次文档治理轮
  （`b5dfbaf`、`cc99df2`）只动索引行，此 bullet 漏扫。本轮改写为迁移
  已推进至 M4 的现状表述，保留「旧实现与证据只归档、未交付项仍为待交付
  契约」语义骨架。
- 非泄露判定（10 处命中不动）：50-risks-adr.md:155/411/565 为 ADR 接受
  时点快照（更新走 addendum，ADR-0025 已有 s4a-2 补记）；
  01-current-state-audit.md:40 为旧树归档审计；STATUS-history/ 5 处为
  已归档轮次叙事；50-risks-adr.md:328「未决未阻塞信号」为信号语义术语
  （pending signal）；23-vfs-block.md 与 20-roadmap.md 的 M4 未完成项
  （mmap/tmpfs/tty/BIO）为真实待办契约（roadmap 本义）；04-rust-
  userspace.md:82 为已兑现交付契约句式。TODO/FIXME/HACK/XXX 标记与
  `- [ ]` 复选框均为 0 处。
- 纯文档变更，零代码风险；本树复验 make all / fmt-check /
  check-rust-only files=191（与 s4a-2 树一致，无新文件）。
