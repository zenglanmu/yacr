# ACIS 实体离散契约（F15：契约已定义，内核未接入，功能未验收）

本文件描述 `cad-kernel-adapter` 的对外契约，以及为什么 ACIS（SAT/SAB）实体
在当前构建中仍**未实现**。契约是真实的、带类型与稳定诊断码的；但“契约存在”
不等于“功能完成”，本文件不声明任何 ACIS 兼容性。

## 1. 边界

`cad-kernel-adapter` 是工作区中**唯一**允许接触内核/ACIS 交换数据的 crate。
任何 kernel handle、SAT/SAB 解析器或曲面对象都不得越过该边界；跨边界只传递
纯数据的 `TessellationRequest` 与结构化的 `TessellationResult`。
`cad-domain`、`cad-representation`、`cad-app` 等只看到这些类型。

## 2. 契约

### 请求

```rust
TessellationRequest {
    geometry: GeometryHandle,     // Resolved(ObjectId) | Missing { key }
    exchange: SolidExchange,      // Sat(Vec<u8>) | Sab(Vec<u8>) | Unsupported { type_key, data }
    tolerance: TessellationTolerance, // linear_deflection + angular_deflection（世界单位，必须有限且 > 0）
    budget: TessellationBudget,   // max_faces / max_vertices / max_edges（均须 > 0）
    stamp: TaskStamp,             // 任务身份，用于过期检查
}
```

* `SolidExchange` 的载荷保持**不透明字节**：在接入内核前不得解释。
* `GeometryHandle::Missing` 表示句柄无法解析；必须显式报告，不得静默丢弃。
* 容差是**世界空间伺服**量，不从显示像素直接驱动测量语义；`from_policy`
  只把显示预算按缩放换算成线性伺服，角度伺服取文档化的默认值。

### 结果

```rust
TessellationResult {
    stamp: TaskStamp,
    outcome: TessellationOutcome,
}

TessellationOutcome {
    Success { geometry: TessellationMesh, diagnostics },
    Partial { geometry: TessellationMesh, degradation: TessellationDegradation, diagnostics },
    Unsupported { reason: UnsupportedReason },
    Failed { diagnostics },
}
```

* **没有“成功但空网格”的变体**：产不出可用面片时只能是 `Unsupported` 或
  `Failed`，绝不以 `Ok(空网格)` 冒充完成。
* `TessellationDegradation` 显式记录 `missing_faces` / `open_edges` /
  `dropped_shells`，每项带稳定原因，供诊断聚合与 UI 展示。
* `UnsupportedReason { code, exchange, detail }` 用稳定码分类，UI 本地化
  `detail`，不解析人类文本。

### 入口

* trait `SolidTessellator { registration(); tessellate(request, cancelled) }`。
* 函数 `tessellate_solid(tessellator, request, cancelled)` 是唯一推荐调用点，
  保证结果带上请求的 `stamp`。
* 默认实现 `NoKernelTessellator`（别名 `PendingTessellator`）：对任何**非空**
  SAT/SAB 返回 `Unsupported`，码为 `kernel.no_acis_kernel`；对未知交换类型返回
  `kernel.unsupported_exchange`；空载荷返回 `Failed(kernel.empty_geometry)`；
  句柄未解析返回 `Failed(kernel.missing_handle)`；取消返回 `CadError::Cancelled`；
  非法容差/预算返回 `CadError::InvalidInput`。

### 稳定诊断码

| 码 | 含义 |
|---|---|
| `kernel.no_acis_kernel` | 本构建未链接 ACIS 内核，SAT/SAB 无法求值 |
| `kernel.unsupported_exchange` | 无法分类的交换载荷 |
| `kernel.empty_geometry` | 载荷为空字节 |
| `kernel.missing_handle` | 几何句柄无法解析 |
| `kernel.invalid_tolerance` | 伺服容差非有限/非正 |
| `kernel.invalid_budget` | 每实体预算非法（如为 0） |
| `kernel.budget_exceeded` | 产出的网格超出请求预算 |
| `kernel.missing_face` | 源实体某个面未能离散 |
| `kernel.open_edge` | 边界边未闭合 |
| `kernel.dropped_shell` | 因缺面而丢弃的壳/体 |
| `kernel.cancelled` | 请求在产出前被取消 |

## 3. 为什么 ACIS 仍未实现

审计（`docs/code-audit-and-agent-handoff.md` F15）确认：

1. **没有内核**：仓库未链接任何 ACIS 实体建模内核，也无许可证决策。
2. **没有解析器**：没有 SAT/SAB 解析器，`SolidExchange` 载荷保持不透明；
   importer 的 ACIS opaque payload 目前为空。
3. **没有样本**：没有可交付的 3DSOLID/BODY/REGION/SURFACE 真实或脱敏夹具，
   无法做分项验收，也就不能声称任何兼容性。

因此默认路径只做**诚实报告**：返回 `Unsupported`，绝不伪造网格。
“返回 `Unsupported`”是契约正确行为，不是完成。

## 4. 未来接入必须提供的内容

接入方在把 F15 从“契约”推进到“已实现”前，必须补齐：

* **输入**：可用的 SAT/SAB（或自有交换）载荷路径，并能从 importer 的
  3DSOLID/BODY/REGION/SURFACE 得到非空字节。
* **内核选择与许可**：明确的内核（自研或第三方）及其许可证；许可与
  第三方声明写入 `THIRD_PARTY_NOTICES.md`；不得修改 acadrust，不得使用
  Cargo `[patch]`。
* **夹具样本**：按 3DSOLID/BODY/REGION/SURFACE 分类的真实/脱敏样本，
  以及“缺面、开边、丢弃壳”的退化样本，纳入 `fixtures/manifest`。

## 5. 验收标准

一项实现只有同时满足以下条件才算完成：

1. 对分项样本产出**真实**面片；`Success` 的网格非空且闭合（或带
   `Partial` 的显式退化报告）。
2. 缺面 / 开边 / 丢弃壳均以 `kernel.missing_face` / `kernel.open_edge` /
   `kernel.dropped_shell` 报告，且与样本预期一致。
3. 容差单调性：更小的伺服容差产生不低于更粗容差的保真度（误差受控）。
4. 预算被真实执行：超预算产出报告 `kernel.budget_exceeded`，不静默截断。
5. 空载荷、未解析句柄、未知交换、取消、非法输入全部走到对应的
   `Failed` / `Unsupported` / `CadError` 分支，且有契约测试覆盖。
6. `cargo test -p cad-kernel-adapter --locked`、workspace 测试与 wasm 检查
   全部通过；`docs/validation.md` 记录本轮可复现实据。

在满足以上标准前，本模块必须保持 `Unsupported` 默认路径，并继续标注
F15 为**未实现/未验收**。
