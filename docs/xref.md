# 外部参照（XREF）v1：只读导入期解析与显示

本文描述 **2026-11-02 落地的 XREF v1**：打开图纸时按宿主策略解析外部参照，把可读到的
参照内容**在导入期合并进宿主数据库**，并在 References 面板显示每个参照的真实状态。
规范条款：spec v2.0 §7.3、§16.2；实现入口
`crates/cad-import-acadrust/src/xref.rs`、`crates/cad-resources/src/xref_path.rs`、
`crates/cad-platform/src/xref_loader.rs`、`crates/cad-db/src/xref.rs`。

> **证据边界**：本文所列能力由**合成 DWG/DXF 夹具的契约测试**覆盖，**不是**真实 DWG
> 样本、AutoCAD 对照、真实 GUI/GPU 或像素验收。尚未跑过的项目在 §9 逐条写 **NOT RUN**。

## 1. v1 做什么、不做什么

**做：**

- 识别源图声明的外部参照块（acadrust `BlockRecord`），解析其保存的路径。
- 按宿主路径策略读取被参照文件，用同一个 acadrust importer 解析。
- 把被参照图纸的**模型空间**内容合并进宿主 xref 块；宿主 `INSERT` 的放置变换保持不变。
- 在 `cad-db` 中记录导入期元数据（原始路径、已解析路径、状态、原因、overlay、深度），
  经 `cad_status.references` 投影到 References 面板。
- 对失败给出**具名状态**与稳定 `import.xref_*` 诊断，绝不把失败改成空成功。

**不做（v1 明确无此能力）：**

- **没有 `XREF` 命令**：无 `ATTACH`/`DETACH`/`RELOAD`/`BIND`，无 xref 编辑、无 xref 创建。
- **不是活动链接**：导入完成后宿主数据库是唯一权威；不会在后续保存/重开时重新读取参照文件。
- 不实现 `XCLIP`（裁剪边界）语义；不实现完整的 `VISRETAIN` 状态持久化。
- 不做 xref 单位换算：合并时坐标逐位复制，不按 `$INSUNITS` 缩放参照内容，也不发明“图纸单位
  → 模型单位”的比例。
- 不导出 xref：DXF 另存不会写 xref 记录/路径，也拒绝把参照内容摊平成普通几何（见 §8）。
- 合并后的成员是宿主数据库里的普通实体；不能按“xref 成员”整体选择、隐藏或重载。

## 2. acadrust 暴露面

acadrust 0.6.8 **没有独立的 `Xref` 对象**。导入器只能从块表读取：

| 来源 | 用途 |
|---|---|
| `BlockRecord.flags.is_xref` | 普通附着（attach） |
| `BlockRecord.flags.is_xref_overlay` | 覆盖（overlay）附着 |
| `BlockRecord.flags.is_xref_unloaded` | 用户已卸载的附着；文件**从不读取** |
| `BlockRecord.xref_path` | 源文件保存的路径**原文** |
| `INSERT` 实体 | 指向 xref 块的引用；放置变换由 `INSERT` 自带 |

`read_blocks` 把任一 xref 标志为真的块保留在块表中（`INSERT` 仍能按名解析），但该块**没有
源实体**；成员要么来自成功加载的参照图纸，要么显式缺席。路径原文保存在
`cad_db::ExternalReference::raw_path`，导入器不会先清洗再保存，因此拒绝诊断可以对照原始
拼写。`resolved_path` 只由宿主在真正读取成功后记录。

## 3. 路径策略（`cad-resources::check_xref_path`）

策略是**纯词法**的：不访问文件系统、不解码 URL、不做 canonicalize；宿主是唯一允许 I/O 的层。
检查顺序与稳定原因码：

| 顺序 | 规则 | 拒绝码 |
|---|---|---|
| 1 | 去掉首尾空白与**成对**的单/双引号；空串、`unnamed`（大小写不敏感）、`.`、`./` 视为空 | `xref.path.empty` |
| 2 | 含 NUL 或其它控制字符 | `xref.path.control` |
| 3 | 以 URI scheme 开头（`^[A-Za-z][A-Za-z0-9+.-]*:`，含 `file:`） | `xref.path.url` |
| 4 | UNC 形式 `\\host\share` / `//host/share` | `xref.path.unc` |
| 5 | `\` 折叠为 `/`，重复分隔符合并，`.` 段丢弃 | — |
| 6 | 任何 `..` 段（即使词法上仍在允许根内） | `xref.path.traversal` |
| 7 | 非 Windows 宿主上的 Windows 盘符路径（`C:\...`、`C:/...`、`C:foo`） | `xref.path.windows_drive` |
| 8 | 绝对 Unix 路径不在 `drawing_dir` 或任一允许根之下 | `xref.path.absolute_outside` |
| 9 | 相对路径在 `drawing_dir` 下拼接；没有 `drawing_dir` | `xref.path.no_drawing_dir` |
| 10 | 拼接结果不在 `drawing_dir` 或任一允许根之下 | `xref.path.absolute_outside` |

补充规则：

- **相对路径以宿主图纸目录为基准**；嵌套导入时基准换成**被参照文件的父目录**，允许根列表与
  字节预算继续沿用。
- **当前原生宿主只授予宿主图纸目录**：`cad-host` 的打开管线与 CLI 都把图纸父目录同时作为
  `drawing_dir` 和**唯一**允许根（不把家目录/进程当前目录当根）；策略本身支持多个 `roots`，
  但 v1 没有用户可配置的额外根界面。
- **绝对路径只允许落在 `drawing_dir` 或显式允许根之下**；前缀必须是完整路径组件
  （`/work/drawings-other` 不算 `/work/drawings` 的子路径）。
- **不做 URL 解码**：`%2e%2e` 只是文件名，不当作 `..`；包含检查仍然适用。
- **成对引号才去掉**；不匹配的引号属于文件名本身。
- **Windows 路径在 Unix 上显式拒绝**（`xref.path.windows_drive`），不做“按 Unix 路径猜”的
  兼容；Windows 上的盘符路径只有在绝对且落在允许根下才通过，`C:foo` 这种依赖进程当前目录的
  盘符相对路径无法词法锚定，直接拒绝。
- **canonicalize + 包含检查在 `cad-platform::FsXrefSource`**：根与候选路径都 canonicalize，
  候选必须是 canonical 根下的**已存在普通文件**。符号链接会跟随，因此允许目录内指向外部的
  链接被拒绝（词法包含检查单独不足以防住这种逃逸）。
- 缺失文件是 `Err`，不是空字节；已存在的**空文件**返回空字节（是否可解析仍由 importer 决定，
  解析失败按 `unsupported` 记录，绝不改写成空成功）。

## 4. 状态表

`cad_db::ExternalReferenceStatus` 是契约值；`CadReferenceStatus` 原样投影，不根据几何或名字
重新推断。

| 状态 | 含义 / 触发 | 稳定诊断 | References 面板表现 |
|---|---|---|---|
| `loaded` | 文件读取、解析并合并成功。参照图纸没有模型空间几何时仍是 `Loaded`，另发信息诊断 | `import.xref_empty`（仅空模型） | 绿色 `Loaded`；显示宿主记录的 resolved path |
| `missing` | 路径为空、文件不存在、或读取错误（非 `Unsupported`/`CorruptData`）。`INSERT` 指向 missing/unloaded xref 时该引用也按 `Missing` 传播 | `import.xref_missing`（空路径）/ `import.xref_unreadable` | 红色 `Not found`；显示保存路径；提示该路径在本机不可读 |
| `browser-unavailable` | 当前运行时没有 xref 文件源：打开请求未提供 loader（浏览器 `xref: None`），或 carrier 返回 `Ok(None)` | `import.xref_unavailable` | 红色 `Unavailable in the browser`；提示浏览器不能读本机文件，需在桌面端打开；**不提供 fetch/重试/定位** |
| `refused-path` | 词法路径策略拒绝；`reason` 是 `xref.path.*` | `import.xref_refused` | 红色 `Path refused`；显示稳定原因码 |
| `unloaded` | 源块标志 `is_xref_unloaded`；文件**从不读取** | `import.xref_unloaded` | 灰色 `Unloaded`；无额外提示 |
| `cycle` | 规范化路径已存在于当前活动路径栈（自引用或环） | `import.xref_cycle` | 红色 `Reference cycle` |
| `depth-exceeded` | 当前嵌套深度超过 `max_depth`（默认 8；宿主图深度为 0） | `import.xref_depth` | 红色 `Depth limit reached` |
| `skipped-overlay` | **嵌套** overlay 被跳过；顶层 overlay 仍按普通附着加载 | `import.xref_overlay_skipped` | 灰色 `Overlay skipped` |
| `unsupported` | carrier 报 `Unsupported`/`CorruptData`、参照图纸解码失败，或总字节预算不足 | `import.xref_unreadable` / `import.xref_budget` | 灰色 `Unsupported`；显示原因码 |

面板只对 `browser-unavailable` 与 `missing` 追加解释句；其余状态只显示状态徽标、原始路径、
resolved path（仅真正解析成功时）、稳定原因码、attach/overlay 与嵌套深度。**没有**
“Reload”“Locate”“Bind”按钮——宿主没有这些命令，面板不伪造入口。

`INSERT` 到未加载的 xref 不会画假几何：块状态按上述损失传播，`Missing`/`Unsupported`
不会被提升为 `Verified`。

## 5. 合并语义

成功加载后，`splice_xref` 把被参照数据库按新 id 复制进宿主 builder：

- **模型空间**实体成为宿主 xref 块的成员（`SpaceId::Block(host_block)`）。
- **宿主 `INSERT` 的放置变换保留**：成员坐标逐位复制，不把插入变换或单位比例烘焙进成员，
  避免与宿主 `INSERT` 的 `T(insertion)·T(-base)` 双重施加。
- **纸空间不合并**：xref 按模型内容处理，被参照图纸的 paper-space 实体被跳过。
- **图层命名空间**：`<xref 块名>|<图层名>`；嵌套逐层组合为 `outer|inner|Layer`。
- **宿主覆盖优先**（VISRETAIN-like 的有限实现）：若宿主已有同名目标图层行，复用该行并保留
  宿主属性，只把参照实体重映射过去；否则新建图层并标记 `xref_dependent = true`。
- **块命名空间**：每个被参照块定义获得新宿主 id 与 `<xref 块名>$0$<块名>` 名称；匿名块也
  参与命名（`*D0` → `XREF$0$*D0`），不会与宿主同名匿名块冲突；嵌套为
  `outer$0$inner$0$Name`。
- **动态块可见性**：成员实体 id 重映射后保留；未合并成员（如纸空间内容）从可见性状态中
  丢弃，不会指向无关宿主实体。
- 嵌套 xref 的状态会在成员分类前写入状态表，因此嵌套 `INSERT` 触及 missing/cycle 等损失时
  继承该损失。
- 嵌套导入的 `import.xref_*` 诊断会复制到宿主报告；对象 id 属于另一数据库，**故意丢弃**，
  不映射成误导性的宿主 id。
- **不发明单位比例**：不读取单位设置做缩放，也不把“源单位未知”改写成某个默认比例。

## 6. 限制与取消

- **嵌套深度**：默认 `ResourceLimits::max_xref_depth = 8`；宿主图为深度 0，直接附着为深度 1。
  `XrefOptions::max_depth = 0` 只允许宿主的直接附着。检查在读取文件**之前**执行。
- **总字节预算**：一个 xref 树默认 1 GiB（`DEFAULT_XREF_BYTE_BUDGET`），在解析前按文件长度
  扣减；嵌套导入继承剩余预算。单个文件超过剩余预算 ⇒ `Unsupported` +
  `import.xref_budget`，**不会**只合并半张图。
- **取消**：每个顶层 xref 解析前与路径检查后轮询取消；嵌套导入接收同一个取消谓词。
  用户取消会以 `CadError::Cancelled` 终止整次打开，不写成某个 xref 状态。
- **实体预算**：参照内容与宿主实体共用 `max_entities` 预算；超出的成员计入 dropped，
  不生成假实体。
- 嵌套导入的进度计数**不**转发给宿主进度条（属于另一文档，转发会让进度回跳），取消仍然转发。
- overlay 只在**顶层**加载；嵌套 overlay 不读取、记为 `skipped-overlay`。
- `unloaded` 附着从不读取文件。

## 7. 浏览器 / wasm

浏览器/`dom-canvas` 打开路径当前把 `ImportRequest.xref` 设为 `None`（`app-web` 的打开代码）。
导入器因此把每个 xref 块记为 `browser-unavailable`，并在解析阶段发出
`import.xref_unavailable`：

- **不会**用 `fetch`/XHR 猜测或下载路径，也不会把参照当成空块。
- References 面板显示红色状态与固定提示：浏览器不能读取本机文件，请在桌面端打开。
- 若要支持浏览器 xref，需要宿主提供**显式 host bridge**（用户授权文件/目录后交给 importer）；
  本 v1 **未实现**该桥，也没有伪造“已加载”。

Android/原生宿主是否提供真实文件源、以及真实设备行为，属于 **NOT RUN**（见 §9）。

## 8. 导出

DXF 另存（v1）**不能**写外部参照记录或路径，因此拒绝把 xref 内容摊平：

- `INSERT` 指向 xref 拥有的块 ⇒ 整条引用不展开、不写成员，报告
  `export.xref_content_dropped`；块被标记为已处理，不会再被当成普通“未引用块”。
- 没有 `INSERT` 到达的 xref 块定义 ⇒ 报告 `export.xref_definition_not_written`，理由明确为
  “v1 导出器不写 xref 附着记录或路径”。
- 两种都是 **Dropped**，导出报告不可能保持 `Complete`；不会输出一个看似普通几何、却无法
  还原附着关系的 DXF。

**没有** xref 记录/路径往返，也没有 DWG 回写。需要保留参照关系时，应保留源图，而不是依赖
本项目的另存结果。

## 9. 测试

**合成契约测试覆盖的区域（本轮 lane 日志实际运行）：**

- `cad-resources`：`xref_path` 的 13 项词法策略（混合分隔符、引号/空白、空占位、控制字符、
  URL/`file:`、UNC、Windows 盘符、`..`、绝对根包含、相对基准、无图纸目录、UTF-8、重复分隔符）。
- `cad-platform`：真实临时目录下的相对/绝对读取、缺失文件为错误、根外路径拒绝、符号链接逃逸
  拒绝、空文件返回空字节、空根忽略、`NoXrefSource` 显式“无文件系统”。
- `cad-import-acadrust`：浏览器源不可用、缺失文件、非法绝对路径不读取、unloaded 不读取、
  深度 9 在读取前拒绝、字节预算在解析前拒绝、成功合并且图层命名空间、空模型 `Loaded` +
  信息诊断、宿主图层覆盖优先、`INSERT` 到 unloaded/missing/无 loader、循环收敛、嵌套 overlay
  跳过而顶层 overlay 加载、嵌套图层链、匿名块不碰撞。
- `cad-db`：状态字符串稳定、引用往返、重复名称拒绝、悬空 provenance 块拒绝、无 xref 时为空表。
- 导出：已解析 xref 的 `INSERT` 拒绝摊平、无成员时仍报告附着、无 `INSERT` 的 xref 定义报告
  `export.xref_definition_not_written`。
- `cad-ui-core` / `apps/desktop-ui`：`cad_status.references` 原样投影、References 面板状态/提示/
  拒绝畸形行、`layout` 页签与 `activeSpace` 投影、`SPACE` 参数解析（见 `docs/layouts.md`）。

具体通过数量见 `docs/handoff.md` 的 2026-11-02 条目；数量只说明这些测试当时通过，不改变下面的
证据边界。

**NOT RUN（不得从上述测试推导）：**

- **真实 DWG xref 样本**：没有用 vendor/AutoCAD 产出的带 xref 的 DWG 跑过端到端导入。
- **AutoCAD 保真**：xref 命名（`$0$`、匿名块、嵌套组合）、`VISRETAIN`、`XCLIP`、单位/比例、
  图层属性继承均**未与 AutoCAD 对照**。本项目的命名是**查看器命名方案**，不声称能还原原始
  xref 图。
- **Windows / 大小写不敏感文件系统**：路径策略的 Windows 分支只有编译级/单元级覆盖，未在
  Windows 真机验证；大小写敏感差异未验证。
- **真实 GUI / GPU / 浏览器视觉**：没有桌面端真实运行、没有 Web 端浏览器 smoke 的 xref/布局
  视觉证据；SwiftShader/无头浏览器结论都不能当作产品验收。
- **macOS / Android 真机**：NOT RUN。
