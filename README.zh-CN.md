# fsync-bench

用 Rust 测试 Linux `fsync`、`fdatasync`、`fallocate`，以及单 writer WAL 的 buffered / direct I/O。只使用 Rust 标准库与 Linux libc 接口，无第三方 Rust 依赖。支持 Linux x86_64、Rust 1.77+，MIT 许可证。

[English](README.md) · [本次完整报告](results/2026-10-07/REPORT.md) · [历史数据](results/historical/README.md) · [WAL 详细说明](WAL-BENCH.md)

## 编译与复现

清除代理变量，离线编译两个程序：

```sh
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
    -u http_proxy -u https_proxy -u all_proxy \
    -u CARGO_HTTP_PROXY cargo build --release --offline

# --dir 选择真正要测的磁盘；--output 必须是新目录。
python3 scripts/run-benchmarks.py \
    --dir /path/on/test/disk --output results/my-run
```

脚本需要 Python 3.9+ 和 `findmnt`。默认顺序运行两个 benchmark：16 KiB block、每 case 每轮 2000 次测量与 100 次预热、8 轮、64 MiB 固定长度文件、每次写入后同步。输出逐轮 CSV、WAL 合并分位数、运行环境、复现命令和报告。使用 `--help` 查看可修改的参数。

## 四种情况

| 情况 | 准备与可能发生的工作 |
|---|---|
| ① append + allocation | 空文件开始顺序追加，增长 `i_size`；可能分配数据块、修改 extent 映射，更新 inode 和 timestamps。并非每次写入都一定重新分配块或修改 extent tree。 |
| ② fallocate + first write | 计时前 `fallocate(mode=0)` 分配空间并设定固定长度；XFS 首次写入仍需 unwritten→written 转换及必要的映射 metadata 持久化。 |
| ③ initialized extent + overwrite + fsync | 计时前真实写满整个文件并同步，覆盖时避免 EOF 增长与首次写入转换；`fsync` 还请求同步文件 metadata。 |
| ④ initialized extent + overwrite + fdatasync | 同样覆盖已初始化空间；`fdatasync` 可跳过 timestamps 等非必要 metadata，但仍须持久化正确读取数据所必需的 metadata。不能一概称为 data only。 |

本机 XFS 的 fallocate 已分配实际物理空间，unwritten 表示该映射尚未初始化、读取返回零。[本次 extent 检查](results/2026-10-07/extents.txt)确认：首次写入前后对应物理块不变，只是标记转换；完整写满并同步后，unwritten 标记消失。WAL 初始化采用 1 MiB buffer 顺序写零，再统一 `fsync`；这部分不计入覆盖性能。只同步文件不能完成初始化，`ZERO_RANGE` 也通常会产生 unwritten extent。

参考：[fsync 手册](https://man7.org/linux/man-pages/man2/fsync.2.html)、[fallocate 手册](https://man7.org/linux/man-pages/man2/fallocate.2.html)、[Linux iomap 映射说明](https://docs.kernel.org/filesystems/iomap/design.html)。

## 历史数据与本次重跑

本机 Intel SSDPE2KX040T8 NVMe、XFS、Linux 5.14、release 构建、16 KiB buffered 写入，每次写后同步。下表是**同步调用的平均耗时**，不包含前面的写入：

| 文件状态 | 同步方法 | 前次 2026-10-04，µs | 本次 2026-10-07，µs |
|---|---|---:|---:|
| append | fsync / fdatasync | 38.95 / 38.85 | 38.27 / 38.26 |
| fallocate 后首次写入 | fsync / fdatasync | 38.47 / 38.72 | 38.68 / 38.52 |
| 已初始化覆盖 | fsync | 25.94 | 24.15 |
| 已初始化覆盖 | fdatasync | 17.57 | 17.33 |

前面的约 39 / 39 / 26 / 18 µs 是历史实测，不是理论估值。本次重跑保留了相同趋势；前次为 6 轮，本次为 8 轮。各项 metadata 的分解用于解释可能的语义工作，不能用这些均值差直接计算某个 inode 字段、extent 操作或 journal transaction 的独立成本。实验没有隔离其他进程 I/O，也没有通过块层跟踪逐项归因。

WAL 应比较**完整提交耗时（write + sync）**：direct 写入本身可能已等待数据 I/O，不能只看 sync 更短。本次 initialized 状态下，buffered 的 fsync / fdatasync 平均提交为 36.13 / 31.61 µs，direct 为 23.33 / 16.50 µs；普通 append 下两种 I/O 平均提交约 55–57 µs。[报告](results/2026-10-07/REPORT.md)列出全部 12 种组合、吞吐与合并样本的 p99。

本机设备队列报告 `write through`。initialized direct + fdatasync 平均 sync 0.41 µs，write 16.10 µs；strace 核对了每次写后确实调用同步。这与已有映射、direct 写入已完成数据 I/O及设备缓存配置相符，但并未单独量化各部分成本。[内核写缓存说明](https://docs.kernel.org/block/writeback_cache_control.html)

## 两个程序

```sh
./target/release/fsync-bench --dir /path/on/test/disk \
    --workloads append,fallocate,overwrite --block-sizes 16K \
    --iterations 2000 --warmup 100 --rounds 8 --file-size 64M \
    --sync-every 1 --methods fsync,fdatasync --csv buffered.csv

./target/release/wal-bench --dir /path/on/test/disk \
    --layouts append,fallocate,initialized --block-sizes 16K \
    --iterations 2000 --warmup 100 --rounds 8 --file-size 64M \
    --io-modes buffered,direct --methods fsync,fdatasync --csv wal.csv

./target/release/fsync-bench --help
./target/release/wal-bench --help
```

`fsync-bench` 使用 buffered `pwrite`；可用 `--sync-every N` 测 group commit，或 `--methods none` 测不逐批同步的写入基线。其 `overwrite` 真实写满文件，必要时绕回覆盖。`none` 不代表逐批持久化吞吐。

`wal-bench` 不提供跳过同步或 batching 选项；每次写入都立刻同步，顺序偏移不绕回。buffered/direct 使用相同的对齐 buffer 与写入长度，无隐式补齐。`initialized` 先 fallocate、写满、同步，再通过文件级 `DONTNEED` 建议释放干净页；准备描述符关闭后才打开测量描述符。WAL 结束后读回首尾测量 block，验证序号、LSN、payload 和最终长度。

固定长度 layout 必须容纳预热与测量写入，越界参数会被拒绝。每 case 创建独立临时文件；初始化、预热、目录同步、末尾排空、读回与删除不计时。正常退出或普通错误返回清理自己创建的子目录，不覆盖已有文件；强制结束可能留下临时目录。CSV 路径必须不存在。

## 验证

```sh
# 同样清除代理变量后运行 Rust 检查。
cargo test --all-targets --offline
cargo clippy --all-targets --offline -- -D warnings

# 可选 syscall 核对需 strace；extent 核对需 XFS、fallocate、filefrag。
python3 scripts/verify.py --dir /path/on/test/disk --strace --extents
```

本次通过 8 个 Rust 测试、96 个 WAL 与 72 个 buffered 集成 case、四种组合的 syscall 核对及 XFS extent 检查。检查次数、顺序轮换、数值、读回、文件长度、输出不覆盖和清理行为；不对延迟设通过阈值。GitHub CI 检查 stable 与最低 Rust 版本，并运行 buffered 集成验证。

吞吐按总测量字节 / 总时间计算，包含采样开销。write/sync/commit 均值分别对应各自计时区间；WAL 末尾分位数合并所有轮原始样本再计算，不平均逐轮 p99。CSV 保存逐轮统计，未保存每条原始延迟。测试验证同步返回与读回内容，没有进行断电持久性验证。
