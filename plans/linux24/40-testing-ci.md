# 40 — 测试与 CI

## 1. 测试金字塔

| 层 | 载体 | 内容 | 阶段 |
|----|------|------|------|
| L1 单元 | host `cargo test` | 纯逻辑：位图/伙伴/slab/extent 树/htree 哈希/电梯排序/环形缓冲/CRC | M1 起持续 |
| L2 组件 | host 程序（Rust bin/test） | 以内存字节数组仿真磁盘，跑 ext4 RO/读写全路径（复用内核模块）| M5 起 |
| L3 集成 | `scripts/run-qemu.sh` | make run 冒烟、sh 交互脚本化、usertests 全套、ext4 黄金清单 | M1 起 |
| L4 一致性 | `scripts/e2fsck-loop.sh` | 负载→e2fsck -fn 判定；崩溃注入循环统计 | M6/M7 |
| L5 性能 | `scripts/bench.sh` | boot 时间、syscall 微基准、文件创建吞吐、crash 循环 RPS | M4 起每里程碑记录基线 |

## 2. L3 集成框架

- `run-qemu.sh <kernel> [append]`：qemu-system-i386 -kernel … -serial stdio -display none
  -no-reboot，带输入脚本（串口灌入命令）与超时/expect 匹配（python pexpect 或纯 shell grep 轮询）。
- 用例清单目录 `tests/integration/*.expect`：banner、shell echo、forktest、pipe 链、
  usertests 分段、ext4 挂载清单 diff、信号、IPC、/proc 快照。
- 内核侧协作：`ktest:` 打印前缀便于 grep；sys_exit 码即用例结果。

## 3. usertests 移植与扩展

- M1：原样移植 xv6 usertests（C）+ zombie/stressfs/forktest。
- M4+ 新增 minios 用例（tests/user/）：lseek/getdents、多 fd、rename、fifo、
  procfs 快照、mmap 匿名、shm/sem/msg、ext4 压力（大量小文件/深目录/截断重命名循环）。

## 4. CI（GitHub Actions，.github/workflows/ci.yml）

```yaml
jobs:
  build-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: i686-unknown-linux-gnu, components: rustfmt }
      - run: sudo apt-get install -y qemu-system-x86 e2fsprogs gcc-multilib
      - run: make check && cargo fmt --check
      - run: cargo test --workspace
      - run: make run            # qemu 冒烟 + banner 校验
      - run: bash scripts/check-no-binaries.sh
```
- Push/PR 全触发；M5 起加 L2 组件测试 job；M6 起加 e2fsck 矩阵 job（矩阵 3×2，超时 15min）。
- artifact：仅日志文本（遵循无二进制政策，镜像不入库不入 artifact）。

## 5. 质量门禁（每里程碑报告必填）

1. make build 零 warning；cargo fmt/clippy(host 面) 通过。
2. 全部测试层通过率与新增用例数。
3. `find -name '*.rs' | xargs wc -l` 无 >400（新增）/ >800（存量）违例清单。
4. 代码评审清单：unsafe 点数变化、每处 SAFETY 注释完备性。

## 6. 已知环境事项

- 多项目共享 cargo 包缓存时可能出现文件锁等待：等待，或为仓库设置独立 CARGO_TARGET_DIR；
- 长任务（构建/测试循环）以后台 + 轮询方式执行；
- 受限网络下 HTTPS 推送可能失败：推送/克隆统一 SSH（git@github.com:）。
