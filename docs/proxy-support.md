# 代理支持（解码证据表 + 失败闭合契约）

`ProxyRecordDecoder`/`ProxyPlayer` 只消费公开 `graphic_data`；`raw_dwg_data`
从不当作代理缓存回放（`ProxyPlayer::inspect_raw_dwg` 显式返回
`Unverified`，不尝试解释）。

> 状态：**契约框架 + 合成测试**，**没有真实厂商样本**，因此不声明与天正/探索者
> 任何版本兼容。合成数据只能证明“我们自己按文档布局的编解码自洽”和“失败闭合”，
> 不能证明厂商格式正确。

## 1. 记录布局（framing）——证据

布局与 acadrust 0.5.5 `entities/proxy_graphics.rs` 的 `ProxyGraphics::decode`
一致（同源 crate 公开实现）：

```text
u32 total_size        （含 8 字节头）
u32 record_count
重复 record_count 次：
  u32 record_size     （含 8 字节记录头）
  u32 record_type
  [u8; record_size-8] payload
```

来源：`acadrust-0.5.5/src/entities/proxy_graphics.rs`，
`ProxyGraphics::decode` / `encode`（常量 `HEADER_SIZE=8`、
`RECORD_HEADER_SIZE=8`、`UNICODE_TEXT_FIXED_SIZE=96`）。
本仓库复刻该布局并加严校验（见 §3）。

## 2. 逐 opcode 证据表

| record_type | 含义 | 证据 | 本仓库行为 | 状态 |
| --- | --- | --- | --- | --- |
| 21 | `FillOff` | acadrust 仅当 payload 为空时类型化（`decode_record`: `21 if payload.is_empty()`） | 空 payload → 置 `fill=false`；非空 payload → **Unsupported**（保留原字节，不猜测） | 证据支持 |
| 36 | `UnicodeText` | acadrust `decode_unicode_text`：96 字节定长前缀（position/normal/direction + height/width_factor/oblique）+ UTF-16LE，`0x0000` 终止 | 严格按该布局解码；缺终止符/奇数尾字节/非法 UTF-16/非有限字段 → **Corrupt**（保留原字节） | 证据支持（本仓库比 acadrust 更严） |
| 其他 | 未类型化 | acadrust 归为 `ProxyRecord::Unknown { record_type, data }` | **Unsupported**，`ProxyOutput.unsupported` 保留原始 payload | 未支持，明确建模 |

**当前已解码**：仅 type 21（`FillOff`，状态指令）与 type 36（`UnicodeText`，
单行文字）。二者都是 acadrust 0.5.5 自身类型化的记录，因此是**证据支持**的。

**仍是猜测/未支持，必须保持显式**：

- 线、面、多段线、圆弧、填充、块的代理记录 opcode：**未解码**，一律
  `Unsupported` 并保留原始字节。**禁止**按“看起来像”推测顶点。
- 样式/变换/绘图状态栈的保存/恢复指令：格式**无公开证据**，未实现；
  未知状态指令会使其后记录不可信，回放因此停止（见 §3）。
- `raw_dwg_data`：独立于 `graphic_data`，需另设样本验证的解析器；
  当前只报告 `Unverified`。
- 天正/探索者兼容性：**无真实样本，不声明**。`PROXYGRAPHICS=1` 只能由具备
  生成能力的源软件重新保存，本应用不能恢复不存在的数据。

## 3. 失败闭合（fail-closed）契约

任何无法**按证据**完整解码的记录，都不允许产出“部分正确”的几何。实现规则：

1. **记录边界**（`metafile::decode`）：`record_size < 8`、`total_size` 撒谎、
   记录越界、记录列表未填满文件、payload 超过 `max_bytes`，全部报错
   （`CorruptData`/`Unsupported`），不返回部分记录。
2. **未知 opcode**：`Unsupported`；其后状态不可信 → 回放停止，原字节进入
   `ProxyOutput.unsupported`，完整性降级为 `Partial`/`Missing`。
3. **畸形记录**：定长不足、缺 `0x0000` 终止符、奇数尾字节、非法 UTF-16、
   非有限 f64 → `CorruptData`；同样保留原字节、停止回放。
4. **顶点预算**：单条记录产出的顶点数在**写出之前**校验；超限则该记录几何
   整体**丢弃**（此前版本会把超限几何 append 进输出，属缺陷），结果为
   `Partial`/`Missing`。
5. **记录数预算**：`max_records` 在回放循环与 framing 两处都生效。
6. **递归/调度深度**：`max_stack_depth` 约束解码器链长度（`with_decoder`
   可无限追加，是本 crate 唯一真正无界的派发路径）；超限 → 拒绝并保留原文，
   不再继续咨询更深链路。
7. **字节预算**：`max_bytes` 在 `replay`（整体）与 `metafile::decode`
   （单记录 payload）两处校验。

**原始字节保留**：`ProxyOutput.unsupported: Vec<UnsupportedRecord>`
（`record_type`、`reason`、`data`）保存被拒记录的精确 payload，供后续证据收集，
不重新解析缓存。

## 4. 完整性与来源

- `ProxyOutput.completeness`：全部记录证据解码成功 → `Complete`；有拒绝/超限 →
  `Partial`（已产出几何）或 `Missing`（无几何）。
- `proxy.capture` 诊断记录类名、handle、缓存字节数、源版本。
- 普通语义实体已成功绘制时不得再叠加代理缓存（由 `cad-import-acadrust` 负责，
  本 crate 只产出几何与完整性）。
- 代理展开进入普通表示/索引/批处理，不为每条指令创建 draw call。

## 5. 测试与样本

- 单元测试（`crates/cad-proxy/src/lib.rs`）：截断记录、无终止符、超限顶点、
  递归炸弹（解码器链）、未知 opcode、已知+未知混合、空缓存、超限输入、
  raw_dwg 不当缓存。
- 合成字节语料（`crates/cad-proxy/tests/synthetic_corpus.rs` +
  `fixtures/proxy/*.hex`）：**合成，非厂商证据**，见
  `fixtures/proxy/README.md`。
- **仍缺**：天正/探索者真实样本（有缓存、无缓存、仅包围盒、不同版本、
  不同实体类别），以及损坏缓存的授权真实样本。没有这些，兼容性保持未验证。

## 6. 接手清单

必须继续验证：记录边界、长度溢出、记录/顶点/状态栈限制、格式版本。
后续若引入状态保存/恢复指令，必须维护样式/填充/变换栈，并有公开证据与样本；
未知状态影响后续输出时保守标记不完整。真实样本到位前，不得把任何厂商格式
标记为已支持。
