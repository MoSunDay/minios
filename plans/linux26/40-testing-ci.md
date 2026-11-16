# 40 — QEMU、测试与证据验收规范

> 本文件是 minios x86_64 / Linux 2.6 的 **QEMU 配置、测试判定、CI 与证据验收唯一权威规范**。
> 其他计划中的简写或示例不得降低本规范；冲突时先修订并冻结本规范，再执行验收。
> Core（M0R–M8）与 External Parity（M9–M13）分别判定，禁止合并通过率或互相借用证据。

## 1. Core QEMU 配置

### 1.1 基线与工具锁

- 配置 ID：`CORE-UP`；程序为 `qemu-system-x86_64`，版本系列为 **QEMU 6.2.x**。
  M0R 必须在工具锁清单固定具体 patch 版本，以及 CI 软件包或镜像的不可变 digest；
  每次运行记录 `qemu-system-x86_64 --version`、配置 ID、完整 argv/命令行。
- 基础参数不得依赖 QEMU 默认值：

```sh
qemu-system-x86_64 -machine pc-i440fx-6.2,accel=tcg -cpu qemu64 -m 256M -smp 1 -display none -monitor none -serial stdio -nic none -no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 -kernel <kernel-elf>
```

- M0R 主路径是 `-kernel <kernel-elf>` 的 **Multiboot1 直载**，静态 gate 必须保存检查
  命令和输出并同时满足：完整 header 在文件前 8192 字节内、起始偏移 4 字节对齐且位于
  可加载段；magic=`0x1BADB002`，magic+flags+checksum 的 32 位和为 0。ELF64 `e_entry`
  必须是可用低地址空间内的 **32 位保护模式过渡入口**，不是高半区 `rust_main`；所有由
  Multiboot 装载的 `PT_LOAD`，其 LMA/`p_paddr`（含过渡代码与页表）须位于可用低地址
  空间，高 VMA/`p_vaddr` 可保留。还须记录入口到长模式的跳转及高半区链接结果；静态
  通过不替代下述真实 QEMU 启动。
- 真实 QEMU 必须经该过渡入口完成启动，并满足串口完整标记、退出码 33 与 host verdict；
  若 QEMU 6.2 无法加载最终 ELF64 高半区布局，必须先用 ADR 冻结 GRUB Multiboot ISO
  回退配置（独立配置 ID、GRUB/工具版本、完整命令和镜像 hash），再判 M0R；不得中途换
  配置却沿用旧日志或旧通过结论。
- **M0R 回退已冻结（ADR-0014）**：QEMU 6.2.0（Debian 1:6.2+dfsg-2ubuntu6.31）对 ELF64
  `-kernel` 报错退出，M0R 真实启动使用独立配置 `CORE-UP-GRUB`。工具与版本：
  `grub-mkimage`（GRUB 2.06-2ubuntu7.2，format `i386-pc-eltorito`，prefix `/boot/grub`，
  内嵌 early config，模块 `biosdisk iso9660 multiboot normal configfile terminal serial
  echo boot search memdisk`）与 `genisoimage 1.1.11`；QEMU 参数除 `-kernel` 换为
  `-cdrom <iso>` 外逐项继承 `CORE-UP`：

```sh
qemu-system-x86_64 -machine pc-i440fx-6.2,accel=tcg -cpu qemu64 -m 256M -smp 1 -display none -monitor none -serial file:<serial.log> -nic none -no-reboot -device isa-debug-exit,iobase=0xf4,iosize=0x04 -cdrom <iso>
```

  ISO 生成与 early config 内容由 `make m0r-iso`（见仓库 Makefile）固定复现；early config
  为 `serial --unit=0 --speed=115200`、`terminal_output serial console`、
  `search --file --set=root /boot/minios-x86_64.elf`、
  `multiboot /boot/minios-x86_64.elf`、`boot`。每份证据记录内核 ELF 与本轮 ISO 的
  sha256；内核 ELF 不变则 hash 必须不变，mkisofs 产物含时间戳、不做跨轮 byte-stable
  假设。M0R 首次全绿参考内核 ELF（Rust 1.97.1，含 out32 debug-exit 与未补零计数）：
  `8aacf344361b0eab84a26787882950f50c1fb8bf1f4b3113ca96b4a1a26ac5cd`。GRUB 自身串口
  输出行（如 `loading minios ...`）不参与协议行判定。

#### M0R 工具锁清单（已审计，2026-11-16）

本地基线二进制（dpkg 数据库为来源，`dpkg-query -W` 版本 + `sha256sum`）：

| 工具 | 包 | 版本 | 二进制 sha256 |
|---|---|---|---|
| `qemu-system-x86_64` | qemu-system-x86 | `1:6.2+dfsg-2ubuntu6.31` | `55862c768c21fa7d3e72ca1b24797d8ab5b406eb5ef4c9ab9c2280f4e0b8f330` |
| `grub-mkimage` | grub-common | `2.06-2ubuntu7.2` | `4c193bc0461280b04edd52f2d03a3b805bb8fc3d10c0bad64f21b9c81059939d` |
| `grub-file` | grub-common | `2.06-2ubuntu7.2` | `a3600d1dac86ff0677a31b793fb7b650f4b83cebb62c3d0d3a732cf74c29357e` |
| `mkisofs`/`genisoimage` | genisoimage | `9:1.1.11-3.2ubuntu1` | `51dae06b7c1b4292340d0cbf8df98414f89f23e3cece5226c7ac3b68cb19769b` |
| （GRUB i386-pc 模块目录） | grub-pc-bin | `2.06-2ubuntu7.2` | 见包 deb digest |

CI 包级不可变 digest：`.github/workflows/ci.yml` 将 apt 源冻结到
`https://snapshot.ubuntu.com/ubuntu/20261116T000000Z/` 并安装下列**精确版本**（Packages
索引内的 deb SHA256 即不可变 digest，来源为该 snapshot 的 `dists/*/binary-amd64/Packages`）：

| 包 | 版本 | deb sha256 |
|---|---|---|
| qemu-system-x86 | `1:6.2+dfsg-2ubuntu6.31` | `154327d064e982cd13fc26f1957f4a76054f89e5805427667d6d524af4c9fd5d` |
| grub-common | `2.06-2ubuntu7.2` | `5fd12ab56ec05a396095089778c5cef1885f59d87df4bc4a64aea4c0f13a3ea0` |
| grub-pc-bin | `2.06-2ubuntu7.2` | `51e3fc7d1758f9d72f09e12bfe51295d3f8a8da49b018a81e45c6d2f127d60c2` |
| genisoimage | `9:1.1.11-3.2ubuntu1` | `c962d270839618f4484a986c7b8dfaa12fab6b72f8968d12fa88127b299d15fa` |

snapshot 版本与本地 dpkg 审计值逐项一致。证据落盘：`make m0r-iso` 打印内核 ELF 与 ISO
的 sha256；`make test-qemu-smoke` 将串口日志与 QEMU stderr 固化到
`<target>/m0r-run/`（`--work-dir`）并打印 `serial.log` 的 sha256。

### 1.2 磁盘及派生配置

M1R 起每轮使用独立 raw IDE root 盘；M5 起增加独立 ext4 第二盘。参数依次追加：

```sh
-drive file=<root.raw>,format=raw,if=ide,index=0,cache=writeback,snapshot=off
-drive file=<ext4.raw>,format=raw,if=ide,index=1,cache=writeback,snapshot=off
```

不得省略 `format/if/index/cache/snapshot`，不得共享可写模板。M5–M8 始终保持 xv6 root，
ext4 仅为 `index=1` 的独立第二盘；M6 只在第二盘执行 `^has_journal` 无日志 RW。
派生配置由**基础配置 × 磁盘模式**生成，只改变下列显式参数，其余继承 `CORE-UP`：

| `disk_mode` | 用途 | 每轮镜像 | root/ext4 `snapshot` | `cache` |
|---|---|---|---|---|
| `EXT4-RO` | M5 只读语义及 M8 SMP RO | root/ext4 均用独立副本 | 均 `on` | `writeback` |
| `EXT4-RW` | M6 无日志 RW、干净卸载及 M8 SMP RW | root/ext4 均用独立副本 | 均 `off` | `writeback` |
| `EXT4-CRASH` | M7/M8 jbd2、kill 后原地恢复 | 同一独立副本贯穿注入和 replay | 均 `off` | `writeback` |

- M5–M7 的 UP 配置分别为 `CORE-UP-EXT4-RO`、`CORE-UP-EXT4-RW`、
  `CORE-UP-EXT4-CRASH`，对应命令为：

```sh
make test-ext4-ro CONFIG_ID=CORE-UP-EXT4-RO
make test-ext4-rw CONFIG_ID=CORE-UP-EXT4-RW
make test-ext4-crash CONFIG_ID=CORE-UP-EXT4-CRASH
```

- M8 基础配置 `CORE-SMP1`、`CORE-SMP2`、`CORE-SMP4` 分别从 Core 基线复制，仅将
  `-smp` 改为 1、2、4。配置生成器再与三个 `disk_mode` 正交组合，唯一
  `config_id=<base_id>-<disk_mode>`，并输出完整 argv、两盘全部参数和镜像 hash；
  裸 `CORE-SMP1/2/4` 不得作为任何 ext4 模式的结果 ID。九项必须全部展开为 required：

| vCPU | `EXT4-RO` config_id | `EXT4-RW` config_id | `EXT4-CRASH` config_id |
|---:|---|---|---|
| 1 | `CORE-SMP1-EXT4-RO` | `CORE-SMP1-EXT4-RW` | `CORE-SMP1-EXT4-CRASH` |
| 2 | `CORE-SMP2-EXT4-RO` | `CORE-SMP2-EXT4-RW` | `CORE-SMP2-EXT4-CRASH` |
| 4 | `CORE-SMP4-EXT4-RO` | `CORE-SMP4-EXT4-RW` | `CORE-SMP4-EXT4-CRASH` |

- `make test-qemu-smp` 必须逐项执行上述九种配置；单项复现命令为
  `make test-qemu-smp CONFIG_ID=<九项之一>`，但单项运行不能使 `verify-core` 通过。
  每个组合的适用 required 集及 expected 计数均在 M8 manifest 冻结且 `total>0`；
  非磁盘 Core 项也逐 case 明确分配到组合。runner 按 config_id 分别记录 base_id、
  smp、disk_mode、完整命令、suite/run、expected/actual、镜像 hash 和 evidence
  ID/path/result，汇总必须恰有 9 个唯一组合；缺项、重复、空集或借用其他组合结果均红。

- M7 的 **200/200** 完整 crash 活动只在 `CORE-UP-EXT4-CRASH` 计数，并满足 30 §5
  的逐类配额。M8 的每个 `*-EXT4-CRASH` 另跑 manifest 冻结的非空 SMP crash 专项，
  逐 config/类别核对 expected=actual>0；三项不得合并计数，也不得折抵 M7 的 200 轮。
  `verify-core` 同时索取 M7 UP 200 轮与 M8 三个 SMP 专项的完整证据。
- Parity 必须另行冻结配置 ID、QEMU CPU 型号、内核 preempt 配置和网络参数。网络派生
  配置删除 `-nic none`，显式使用 `-device e1000,netdev=n0 -netdev user,id=n0`；
  Core 默认无网，不得因 Parity 联网配置而改写 Core 证据。
- `make run` 与 `make test-qemu*` 必须复用同一个 Rust runner、配置生成器和基础参数；
  前者只提供交互/演示入口。演示进程到时退出、看到 banner 或正常关机均不等于测试成功。

## 2. 判绿协议

### 2.1 Guest 与 host 双重判定

1. Guest 仅在全部 required 用例完成后向 `0xf4` 写 `0x10`；`isa-debug-exit` 使 QEMU
   预期退出码为 **33**。任何其他 QEMU 退出码均不能作为通过证据。
2. Guest 每个用例输出唯一 `case_id` 和 `PASS/FAIL/SKIP/TCONF`；通过标记至多一次，
   且是最后一条协议行，格式为：

```text
MINIOS_X86_64_TEST_PASS_V1 suite=... run=... total=N passed=N failed=0 skipped=0
```

3. host **Rust verdict** 必须验证 suite/run ID 为本轮预期值，实际 case ID 与预期清单
   完全相等，数量一致，`total > 0` 且计数自洽。缺例、重复、未知 ID、required 的
   `SKIP`、任何 `FAIL/TCONF` 均为红；最终验收中任何 in-scope `SKIP` 也为红。
4. host 仅在“成功标记唯一且完整 + 全部预期用例完整 + 普通 suite 全日志无
   fatal/fail/panic + QEMU 实际退出码 33”同时成立时返回 0。标记之后发生 panic、
   异常复位/退出或 timeout 仍为红，不得截断日志规避检查。
5. suite deadline 由 Rust runner 执行并产生 `TIMEOUT` 失败证据；CI job timeout 是更外层
   的设施保护。两者分别配置且 `suite deadline < CI timeout`，为日志落盘和诊断预留时间；
   任一 timeout 都不是 skip-success。
6. M1R 受控 fault 只进入独立 `test-expected-fault` 日志与专用 Rust verdict，不送入普通
   suite 的 panic=FAIL 扫描器。专用 verdict 必须精确匹配预声明 fault_id、寄存器/RIP、
   符号化回溯和完成标记，任何额外/意外 fault 仍红；其结果不计入普通 suite 用例数，
   不得替代普通 required 项或作为普通套件通过证据。

### 2.2 隔离与判定器自测

- 每次运行创建独立临时目录、日志和镜像；启动 QEMU 前以“不存在则新建”方式创建本轮
  日志，禁止追加旧日志。记录并清理本轮回合，不覆盖失败证据。
- 为 verdict 编写 Rust 故障自测，至少覆盖：空日志、伪造/不完整成功标记、重复 case 或
  标记、少跑、未知 case、`total=0`、required skip、fail/TCONF、标记后 panic、错误退出码、
  timeout；全部必须被判定为非零。再验证一组合法完整日志确实返回 0，防止恒定失败。

### 2.3 crash 闭环

`kill` 只证明注入成功，不是测试通过。每个计数样本须记录 replay 前真实
`needs_recovery`，以及非零的已提交事务/replay 块计数；空 replay 不得计入任何 crash
专项。每轮必须依次完成：记录 config_id、主类别和注入点并终止 QEMU → 用**同一镜像**
重启并 replay journal → 执行业务语义校验和干净卸载 → 确认 QEMU 已停止后做严格离线
`e2fsck -fn`（退出 0）与语义复核 → 第二轮也满足完整标记、退出码 33 和 host verdict。
host 按 config/类别核对 expected/actual，任何类别为空、缺计数、缺步骤、镜像被重建、
fsck 非零或语义不一致均为红。M7/M8 的配额与组合按 §1.2 和 30 §5 分别验收。

## 3. 覆盖矩阵

### 3.1 机读 schema 与激活规则

Core 与 Parity 使用分开的机读 manifest。输入 manifest 只保存静态覆盖字段；runner 生成
派生运行/结果文件并补入 `activation`/`evidence`，不得回写输入。派生记录每条至少包含：

| 字段 | 约束 |
|---|---|
| `case_id` | 全局稳定且唯一，不因改名复用旧证据 |
| `source` | 具体到来源和用例，如 `xv6@2305a8c:usertests:forktest`；新例写 `new:<设计项>` |
| `milestone` | 首次必须执行的里程碑 |
| `activation` | 展开后的运行 manifest 标为 `active` 或 `not-active`，由当前阶段与 `milestone` 计算 |
| `config` | 输入可列多个冻结配置 ID；runner 每个派生结果只对应一个唯一 `config_id`，M8 必须为复合 ID |
| `mode` | 仅 `host`、`qemu`、`external` 之一 |
| `runner_test_name` | 可定位到 Rust runner 中的实际测试 |
| `requirement` | `required`、`optional`、`out-of-scope` 之一 |
| `adr` | `out-of-scope` 必须填写事先接受的 ADR，其余可为空 |
| `evidence` | 完整结构 `{id,path,result}`：本轮证据 ID、派生结果文件路径与用例结果；未激活用例不得伪造结果 |

- M1R 只执行 `milestone <= M1R` 的到期项；未来项标为 `not-active`，不是 skip 或通过。
  M8 时所有 Core 项必须激活且为 required；Parity 的 optional/out-of-scope 不得冲抵 Core。
- [Core 范围](00-scope-linux26.md)每一项必须映射到至少一个测试 ID，每个 Core 测试也能
  反查范围项。M9 从 Linux 2.6.39.4 权威源冻结 syscall/flag/结构体 ABI 清单，逐项映射
  测试 ID；编号相同或文档声称支持均不等于已验证。
- 最低演进覆盖：M0R 长模式/页表/早期 panic；M1R 异常/trap/oops、Rust 用户态、syscall
  与 xv6 对应用例；M2–M4 内存/调度/信号/VFS；M5–M7 ext4 RO/RW/jbd2/crash；M8 SMP/TLB/RCU、
  clone/futex/epoll/IPC。M9–M13 的 ABI、发行版和网络等只进入独立 Parity 矩阵。

## 4. CI 交付目标

> 以下是**待交付目标契约，不表示仓库当前已经存在这些 workflow/job**。验收时由
> `verify-core`/`verify-parity` 枚举预期 job 和矩阵；**已到期** job 缺失、空矩阵、工具或
> 依赖缺失、未生成证据均直接失败，不允许 skip-success。未到期 job/依赖记 `not-active`，
> 不提前判缺失，也不计通过。

### 4.1 分阶段 job

| 到期阶段 | 必须具备的 CI 目标 |
|---|---|
| M0R | 构建 ELF64 kernel；Multiboot1/ELF64 静态布局检查；`make test` 的 host 适用项；`make test-qemu-smoke` |
| M1R | 增加 `make test-qemu`、内部 ELF/force-frame-pointers 检查、`make check-rust-only`；独立 `make test-expected-fault` |
| M5 | 增加 `make test-ext4-ro CONFIG_ID=CORE-UP-EXT4-RO` |
| M6 | 增加 `make test-ext4-rw CONFIG_ID=CORE-UP-EXT4-RW` |
| M7 | 增加 `make test-ext4-crash CONFIG_ID=CORE-UP-EXT4-CRASH`，UP 200/200 且逐类配足额满 |
| M8 | 增加 `make test-qemu-smp`（九项 SMP×EXT4 复合配置均非空）及 `make verify-core` |
| M9 | 独立 `make test-abi-ltp`：仅静态外部 ABI/LTP 选择集、执行结果与可再生成差距清单；不要求 rootfs、`test-distro` 或 M10+ 依赖 |
| M10 | 增加 MM/VFS/Block/FS 对应的 parity 子系统 job 与到期 LTP 选择集，不提前要求发行版或网络/安全 job |
| M11 | 增加进程/平台/完整 tty parity job，及独立 `make test-distro`；此时才按冻结配置准备 rootfs/loader |
| M12 | 增加派生网络配置互操作、安全/容器 parity job；Core 默认配置保持不变 |
| M13 | 增加汇总 `make test-parity` 与最终 `make verify-parity`，重验 M9–M12 和完整 Core |

M0R 不提前要求 M1R 用户程序/usertests，也不要求全树 Rust-only；M1R 才启用对应门禁。
expected-fault 使用独立 job、日志和证据命名空间，普通 CI 不对其套用普通 panic 判绿，
其 PASS 也不进入普通套件汇总。各阶段只检查到期依赖；M9 的静态 fixtures/选择集缺失
是 M9 失败，但 rootfs/loader 在 M11 前属于未到期，不得记 M9 依赖失败或测试通过。
`verify-core` 校验 M0R–M8 所有 required 证据，包括 M7 UP 200/200 和 M8 三个 SMP
crash 专项；`verify-parity` 先重验 Core，再校验 M9–M13。M13 的 `test-parity` 是到期
总验，不得在 M9–M12 用空的总矩阵或未来项 SKIP 冒充进展。

- `make test` 必须把到期 host 包和目标显式展开为
  `cargo test --locked -p <package> --target <host-triple>`（或等价的显式 manifest/test
  target 参数）；禁止不加选择的 `cargo test --workspace`，host 测试不得构建 QEMU/external 包。
- 编译与测试的 `RUSTFLAGS`/构建参数必须含 `-D warnings`，并执行 fmt/静态检查；不得用
  全局 allow 掩盖新增警告。
- Rust-only 检查覆盖 `abi/`、`kernel/`、`user/`、`tools/`、`host-tests/`、`tests/`。
  测试用例、runner、manifest 解析和 verdict 必须由 Rust 实现；shell/Python 仅可启动进程、
  传参、设置 timeout 和收集日志，不得解释结果或自行判绿。
- 外部 C 编译器、LTP/glibc/rootfs 构建只存在于独立 Parity job，不进入基础 build、
  `make test`、Core 镜像或 Core 依赖链。

### 4.2 可重复工具链与安全检查

- M0R 将 `rust-toolchain.toml` 从 `stable` 改为具体、已审计的 Rust 版本；CI 不得再使用
  `stable/latest`。所有 Cargo 解析使用 lockfile 和 `--locked`。
- GitHub Actions 的每个 `uses` 固定到完整 commit SHA；runner/container、QEMU、e2fsprogs
  固定版本及包/镜像 digest。本文不虚构 SHA/digest，交付时填入经审计的真实值并留存来源。
  **M0R 已填入真值（2026-11-16，`.github/workflows/ci.yml`）**：`uses` SHA 取自
  `api.github.com`（checkout `11d5960a…`、upload-artifact `ea165f8d…`、
  rust-toolchain `df2a88e…`）；不使用 container，改用 `ubuntu-22.04` runner +
  `snapshot.ubuntu.com/ubuntu/20261116T000000Z/` apt 冻结（包级不可变 digest 见 §1.1
  工具锁清单），QEMU/GRUB/genisoimage 逐包精确版本安装。后续阶段（e2fsprogs 等）沿用
  同一规则：先审计、再填值、留存来源。
- CI 接入固定版本的 secret scan；日志不得打印秘密，失败日志也应先脱敏再上传。
  无二进制检查必须同时检测 NUL 和实际文件类型，不能只看扩展名，以防改名绕过。
- 所有外部源码、工具链和 rootfs 下载后，先校验预期密码学 hash，再解压、编译或执行。

## 5. 外部验证边界

- 外部源码、编译器、构建树和 rootfs 必须位于 checkout 外，或位于已忽略的专用生成区；
  不得进入 Cargo workspace、实现树或 git 历史。固定并记录来源版本、hash/签名、包清单、
  loader 和测试选择清单；不得为通过测试而修改外部测试二进制。
- 内部 Rust 用户 ELF 的 `ET_EXEC`、静态链接和入口检查只适用于内部布局；不得把该规则套到
  外部动态 ELF。外部 `ET_DYN`/interpreter/loader 按冻结的 ABI 与发行版配置单独检查。
- LTP 结果明确映射：`TPASS=PASS`，`TFAIL=FAIL`，`TBROK=BROKEN/FAIL`，`TCONF=CONF`；
  缺少原始结果也算失败。in-scope `TCONF` 不通过，out-of-scope 必须有事先 ADR。
  Rust Core 报告与 External Parity 报告分开生成、分开汇总。

## 6. 证据有效性与完成门禁

每份证据至少记录：

1. 干净 commit（`git status` 为空）、配置 ID、完整 QEMU argv、实际 QEMU/verdict 退出码；
2. Rust/工具链版本与锁文件、QEMU/e2fsprogs 包或镜像 digest、内核 preempt/CPU 等配置；
3. 输入/派生 manifest 与所有输入镜像 hash；到期且适用的外部源码/rootfs/loader 版本、
   hash 和选择清单，未到期或不适用字段显式记 N/A，不得伪造或提前形成依赖；
4. suite/run ID、预期及实际用例数，PASS/FAIL/SKIP/TCONF/XFAIL 数和日志位置。

实现或测试源码、ABI 清单、runner、manifest、镜像生成输入、工具链或工具配置发生变化，
相关证据立即标为 stale 并重跑；跨子系统变更必须重跑完整 Core。dirty tree 只可用于调试，
不得产出验收证据。旧配置、旧 commit 或旧镜像的结果不得复制到新证据。

- XFAIL 只用于进度报告，不得计入通过。最终任何 in-scope XFAIL/FAIL/SKIP/TCONF 均阻断；
  out-of-scope 必须在失败前由 [ADR](50-risks-adr.md) 冻结范围、理由和替代测试，禁止失败后豁免。
- 镜像按 [04 的构建流水线](04-rust-userspace.md)可复现生成：固定输入、参数和工具并记录 hash；
  crash 按 [30 的闭环](30-ext4.md)记录随机 seed、轮次和注入点，失败必须能用同 seed 重放。
- **Core done** = M0R–M8 证据全部有效且 Core 矩阵全绿；不要求外部 glibc/LTP/rootfs。
  **Parity done** = Core 仍全绿，M9–M13 冻结配置项全部关闭且 External Parity 矩阵全绿。
  文档记录差距、预期结果或单次演示均不等于完成。
