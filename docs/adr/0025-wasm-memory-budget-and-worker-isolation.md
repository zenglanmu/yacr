# ADR 0025：wasm 内存预算、分块上传与 Worker 隔离（DOM-canvas 阶段 ⑧ 首批）

- 状态：**Accepted（2026-10-24）**，首批已实现并实测（见 §7）。
- 相关 ADR：[ADR 0024](0024-dom-canvas-wasm-cad-path.md)（DOM-canvas 载体；其 §7 阶段 ⑧ 由本文承接）、
  [ADR 0023](0023-dom-on-top-cross-host-composition.md)（合成模型，不变）。
- 需求：`CAD_IMPLEMENTATION_SPEC.md` §21（本文不改合成/权威不变量）。
- 影响：wasm 会话的内存策略、上传策略、线程模型与**能力声明**；不改 `cad-db` 唯一权威、
  单 presenter、renderer-only wgpu、无整幅回读。

---

## 1. 问题

ADR 0024 阶段 ⑧ 的首个真实样本给出硬数字：`fixtures/complex-test.dwg`（**4.2 MB**）在 wasm 内
导入后 **wasm 线性堆 1.91 GB**（32-bit wasm 上限 4 GB）、单次开图 **14.1 s**。也就是说
"浏览器/dom-canvas 可打开合格样本范围内的 DWG"这句话在**没有内存工程**时不成立；而大图撞上限时
必须**显式拒绝**，不能空白画布或静默降级。

同时，两个引擎对 Worker 内 GPU 的能力**不一致**（实测）：
Chromium 141 可在 dedicated Worker 内通过 `OffscreenCanvas` 拿到 WebGL2；
WebKitGTK 2.52.6 的 `OffscreenCanvas.getContext("webgl2")` 返回 **null**。

## 2. 决策

### D1 显式预算（先拒绝，再解析）

- `MAX_INPUT_BYTES = 64 MiB`：**解析前**按文件字节拒绝。
- `MAX_WASM_HEAP_BYTES = 3 GiB`（4 GiB 的 75 %）：**建场景后、上传前**按 `wasm_bindgen::memory()`
  拒绝。
- 拒绝一律 `step: "budget"`，消息含**预算名、上限、实测值**，status 附
  `budget: {name, limit_bytes, measured_bytes}`；**上一份文档与像素保持不变**。
- 语义边界：`open_bytes` 的参数是 `&[u8]`，文件在检查前已复制进 wasm 内存；因此 pre-check 只保证
  "不进入解析"，不保证"不复制"。上限 64 MiB 让这次复制有界；需要更省时由**调用方**先按
  `max_input_bytes` 拒绝（宿主/外壳可读 `file.size`）。

### D2 数据库保留策略是**显式常量**，不是副作用

`budget::DB_RETAINED = false`，status 报 `db_retained: false`。canvas 契约当前没有保存/测量/查询，
所以开图后释放 `Database` 是正确取舍；**一旦**引入这些能力，必须把常量翻成 `true` 并改本文，
不允许"某处悄悄又留一份"。

### D3 分块上传、单点原子提交

- 每块上限 `UPLOAD_CHUNK_MAX_BATCHES = 2048` / `UPLOAD_CHUNK_MAX_VERTICES = 262144`（与引擎分页
  同量级），用 `prepare_upload_batches` 逐块 staging，块源几何随即释放，`append` 累积，最后
  **一次** `commit_upload(prepared, 0)` 发布。
- 完整性守卫：发布 batch 数必须等于总 batch 数，否则 `step: "upload"` 硬失败——**不存在**
  "第一帧只画了一部分却报成功"。
- 代价如实记录：块边界会增加常驻 stream / 每帧 draw 提交（实测 draws 7,707 → 7,709）；wgpu GL
  后端可能把 `write_buffer` 负载暂存到首次 submit（backend 内部，不可观测）。

### D4 Worker 隔离**按能力**，且必须可见

- 判定顺序：`transferControlToOffscreen` 可用 **且** worker 自报 `{ok:true, isolation:"worker"}`
  → 隔离模式（worker 独占会话 + `OffscreenCanvas` 渲染，页面**不**实例化 wasm 模块）。
- 否则退回主线程载体，并在 status 明写 `isolation=main-thread (<reason>)`；
  WebKitGTK 实测得到 `isolation=main-thread (no WebGL2 in worker)`。**禁止**静默退回。
- 单所有者规则：worker 模式下页面若再创建会话即违反 D5；`Offscreen` 载体在非 worker 上下文
  直接以 `step: "isolation"` 拒绝（结构性约束，不靠约定）。

### D5 单会话/单 presenter/无回读不变

worker 与主线程各持自己的 wasm 实例与堆，但**同一时刻只有一个实例**在跑（页面在隔离模式下从不
实例化主线程模块）。合成方向、`cad-db` 唯一权威、renderer-only wgpu、无整幅回读全部不变。

## 3. 否决的备选

- **只放宽上限/不设预算**：等于把 4 GiB 撞墙留给用户，且失败形态是 OOM abort（不可诊断）——拒绝。
- **worker 解析 + 主线程渲染（跨线程 batch 线）**：会引入显示表示的第二份编码与复制；
  Chromium 上 `OffscreenCanvas` 已能零拷贝，WebKitGTK 上则该线只为补一个引擎缺口——收益不抵成本。
- **WebKitGTK 上强上 worker 渲染**：实测 `getContext("webgl2") === null`，只能显式标注，不能假装。

## 4. 后果

- 真实样本从 **1.91 GB → 1.43 GB（−25 %）**、**14.1 s → 11.2 s（−21 %）**；峰值仍由
  `cad-import-acadrust` + `cad-app::render_scene` 的导入/建场景阶段决定（本 ADR 不修改这些 crate）。
- 下一轮若要继续压低峰值，只能在导入/表示层做**流式/分页**或跨 crate 释放，需另立 ADR。

## 5. 非目标

- 不改 Windows/macOS/Android；不改 native `native-below` 路径。
- 不承诺浏览器矩阵、性能指标、vendor 兼容；worker 隔离只在**实测支持**的引擎上声明。
- 不在本 ADR 引入跨线程 batch 协议、SharedArrayBuffer 或 WASM 线程。

## 6. 失效条款

| 被修订/补充的条款 | 变化 |
|---|---|
| ADR 0024 §7 阶段 ⑧ | 首批（预算/分块/worker 隔离）已实现；剩余"降低导入峰值"另立项 |
| SPEC §21 平台状态段 | 追加"⑧ 首批已实现 + 数字"，不改合成/权威条款 |
| AGENTS 铁律 8 | **不变**；新增的 `step:"isolation"` 与 `budget` 属其显式拒绝族 |

## 7. 证据（实测，限定范围）

| 观测 | 主线程载体（Chromium 141） | Worker 隔离（Chromium 141） | WebKitGTK 2.52.6 |
|---|---|---|---|
| synthetic fixture | 4 batches / 1 draw / 出图 | 同左（`target=offscreen-canvas`） | 同左（`target=html-canvas`） |
| `complex-test.dwg` 堆 | **1,425,866,752 B** | **1,423,900,672 B** | 未测（宿主仅开 fixture） |
| 开图耗时 | **11,204 ms** | **11,387 ms** | — |
| 分块 | 114 块，峰值 2,048/块 | 同左 | — |
| `isolation` | `main-thread (…)` | `worker` | `main-thread (no WebGL2 in worker)` |
| 预算拒绝 | 70 MiB 输入 → `step:"budget"`，旧文档/像素不变 | — | — |

证据目录与边界见 `docs/validation-dom-canvas-wasm-2026-10-24.md` §8/§10。**不**构成性能结论、
硬件归属结论或跨引擎/跨平台结论。
