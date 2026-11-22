# ADR-0046：ymodel 的 zstd entry codec（zip 容器 + Stored zstd 帧）

- 状态：已接受（2026-11-22）
- 相关：[ADR-0037](0037-ymodel-schema-and-persistence.md)（D11 容器 / D13 版本门禁 / D17 增量保存）、
  [ADR-0038](0038-mesh-base-color-texture.md)、[ADR-0040](0040-ymodel-embedded-resources.md)、
  [ADR-0042](0042-bim-semantics-in-ymodel.md)
- 影响：`cad-store-zip` 新增 `codec` 模块；manifest 增加 `codec`/`len_c` 并重新定义 `crc`；
  `FORMAT_READ`/`FORMAT_WRITE` 升到 **2**；依赖新增 `structured-zstd =0.4.3`；
  workspace `rust-version` 升到 **1.92**

## 背景

`.ymodel` 的 JSON 分片此前用 zip 的 deflate 压缩。真实模型测量显示，deflate 的 32 KB 窗口
无法利用大分片内部的远距离重复：

| 语料 | JSON 总量 | zip deflate | zstd 6 |
|---|---:|---:|---:|
| `test-fonts.dwg`（14 MB） | 45.5 MB | 4,226,201 B | 3,506,870 B（−17%） |
| `rmeadvancedsampleproject.rvt`（38 MB） | 189.1 MB | 23,965,324 B | 7,819,008 B（−67%） |

同时 zstd 的编码/解码速度更高（大分片 L6 约 600 MB/s，L1/L3 约 2 GB/s；deflate 约 43 MB/s）。
分片不是"很多小 JSON"：最大单分片在 DWG 为 6 MB、RVT 为 111 MB，因此 zstd 的大窗口直接生效。

## 决策

**保留 zip 容器，改变 JSON 条目的含义**：

- 本构建写出的每个 zip entry 都是 **`Stored`**；manifest 的 `codec` 说明 payload 如何解释：
  - `zstd`：一条标准 RFC 8878 zstd 帧，解压后是 JSON（所有受建模的 JSON entry）；
  - `raw`：字节原样（`textures/<key>`、`resources/<kind>/<key>`）；
  - `opaque`：foreign/未建模条目，永不解析，只按原字节复制。
- `manifest.json` 自身是 `Stored` 明文 JSON（`unzip -p doc.ymodel manifest.json` 仍可读）。
- 压缩由本仓库在写 zip 之前完成，使用纯 Rust 的 `structured-zstd =0.4.3`
  （`default-features = false, features = ["std"]`，level **6**，pledged content size）。
  **不启用** zip 的 `zstd` feature（那会引入 C `zstd-sys`）。
- `zip` 的 `deflate-flate2` 只保留给 `previous::measure` 读取 foreign deflate 条目。
- 版本：`FORMAT_READ = FORMAT_WRITE = 2`；`FORMAT_MINOR` 不变（11）。读取是**等值门禁**：
  `format.read != 2` 直接 `ymodel.unsupported_version`。项目未发布，旧归档不承诺兼容；
  恢复快照/旧文件需要从源格式重新导入。

### Manifest

`EntryRecord` 增加 `codec: {zstd,raw,opaque}` 与 `len_c`，`crc` **重新定义**为
"zip 中央目录记录的、Stored payload 的 CRC-32"（即 zstd 帧/raw 字节的 CRC），不再是解压后
字节的 CRC。`sha256` 仍是逻辑字节哈希；`sha256_c` 是 Stored payload 哈希，本构建对
`codec != opaque` 的条目**必然写入**。

解析不变式（违反即 `ymodel.invalid_document`）：

1. `codec` 只允许三个值；
2. `opaque` ⇔ `sha256_c == null`；
3. `raw` ⇒ `sha256_c == sha256` 且 `len == len_c`。

### 完整性

- 读侧先用 zip 中央目录做 O(1) 检查：`crc32 == crc`、`compressed_size == len_c`；
  受建模条目必须是 `Stored`。
- 对 `zstd`：先校验帧字节的 `sha256_c`，再按 `len` 预分配并解码（`decode_all` 不扩容），
  最后校验逻辑 `sha256`。帧头声明的长度必须等于 manifest `len`，否则在解码前拒绝。
- 新增 `MAX_ENTRY_BYTES = 1 GiB`（单 entry 的 `len`/`len_c` 上限），防止 manifest 声明
  天文数字驱动分配；真实最大分片为 111 MB。
- `sha256_c` 现在**可计算也可校验**：ADR-0037 D11 rev.8 记录的历史缺口关闭。

### 增量保存

未触及分片、未变资源与 foreign entry 仍通过 `ZipWriter::raw_copy_file` 原样复制；
复制条目的 `EntryRecord` 逐字段继承自上一归档（包括真实的 `sha256_c`）。下一次打开会在
解压前校验帧哈希。

## 备选（拒绝）

- **zip method 93（C zstd）**：改动最小，但 `zip` 的 `zstd` feature 引入 C `zstd-sys`，
  wasm 目标需要 clang/wasm-shim，破坏本仓库"纯 Rust、可编 wasm"的依赖约束；Info-ZIP
  也不能读 method 93。放弃。
- **自写 TOC 容器**：可少一层 zip 头、便于未来字典，但需要重写 read/write/previous 与
  8 个契约测试，且失去"任意 zstd 工具可取出 entry"的逃生通道。收益不足以抵消工程量与风险。
- **保持 deflate**：真实数据上压缩率与速度都明显更差。

## 后果

- **可检查性降级**：`unzip -p doc.ymodel elements/x.json` 现在输出二进制；需要
  `| zstd -d`。manifest 仍可直接读。标准 RFC 8878 帧意味着任何 zstd 实现都能恢复数据，
  不锁定在本 crate。
- **依赖风险**：`structured-zstd` 0.4.3 较新（2026-11 发布）；decoder 成熟、encoder 仍在
  改进。以精确 pin + 固定 feature/level + 每次读取的 `sha256_c`/`sha256` 校验控制风险；
  若将来更换实现，帧格式仍是标准 zstd。
- **内存**：每个 JSON entry 峰值从"一个 JSON buffer"变为"JSON + 帧"（最坏 111 MB → +约
  4.7 MB）。"一次一个分片"的不变式保持不变。
- **预算**：`SinkWriter` 的逐块 abort 不变；zstd 条目在压缩后才可能触发超限。
- **确定性**：entry 排序、固定 `DateTime`、全部 `Stored`、固定 codec 版本/level/feature，
  两次保存字节相同；跨版本/跨目标不承诺字节稳定。
- **MSRV**：workspace `rust-version` 从 1.88 升到 1.92（`structured-zstd` 的声明）。

## 证据

- 契约/单元测试见本轮 `docs/validation-ymodel-zstd-2026-11-22.md`。
- 实测语料为仓库 `fixtures/` 与操作者提供的真实 DWG/RVT；结论仅限 Linux/native + 本机，
  不是跨平台、GPU、像素或产品验收。
