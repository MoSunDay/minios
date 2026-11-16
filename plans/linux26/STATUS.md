# STATUS — x86_64 / Linux 2.6.39.4 实施状态

> 本文件只报告实施与证据状态。**计划文档终审完成不等于实现完成，也不使任何 gate 变绿。**
> x86_64/Rust-only 迁移已启动：M0R 代码与 runner 已落地并在真实 QEMU 冒烟全绿（见下），
> 但**取证工作树为 dirty，尚不构成正式 gate 证据**；验收规则以
> [40-testing-ci.md](40-testing-ci.md) 为唯一权威。

## Rollup

| 范围 | 当前状态 | 判定 |
|------|----------|------|
| Core-2.6（M0R–M8） | ⬜ **pending**（M0R 实现+冒烟+工具锁+CI 接线完成） | M0R 冒烟绿仅在 dirty tree 取得；还需干净 commit 取证 + CI 首次绿后才能判 M0R 绿 |
| Parity-2.6（M9+） | ⬜ **pending** | 未启动；权威差距表、Parity 配置和外部矩阵均未验收 |

最近同步：2026-11-16。唯一实现基线为 Linux 2.6.39.4 / x86_64；不沿用旧完成度或工期。

## 里程碑状态

| 里程碑 | 交付摘要 | 状态 | 当前证据/前置 |
|--------|----------|------|---------------|
| M0R | 工具锁、Multiboot1/ELF64、长模式、高半区、真实 QEMU smoke、CI workflow | 🟡 **实现完成，待取证**（剩净树取证 + CI 首跑） | 见下文 M0R 现状 |
| M1R | syscall/FPU/UserPtr、静态 Rust ET_EXEC 用户态与到期 required 用例 | ⬜ **pending** | 等 M0R 全绿；旧 C usertests 不计 |
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

## M0R 现状（2026-11-16，dirty tree，尚非正式证据）

已完成（`make all`、`make test-qemu-smoke`、`make test` 全绿）：

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
2. CI job（`.github/workflows/ci.yml`，M0R 到期集合：build + 静态布局 gate + host 测试 +
   `test-qemu-smoke` + secret/no-binary 扫描 + fmt-check）已接线并在本地等价验证，但
   尚未在 GitHub 上跑过首次绿；push 属需授权操作。

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
