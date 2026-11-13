# ADR 0034：TypeScript 二次开发 API（AutoCAD .NET 命名空间镜像 + 宿主代理）

- 状态：**Accepted（2026-11-13，骨架 + 第一条只读操作）**。本 ADR 定的是**契约与目录边界**：API
  包、宿主 adapter、`cad_api_status` 能力描述符与显式拒绝语义，以及 `cad_api_call` 的最小只读
  通道。**已真实服务的宿主操作有三项**：`documentManager.mdiActiveDocument`、
  `documentManager.documents`、`database.revision`；其余 18 行仍 `pending`：事务、编辑器提示、
  符号表都没有实现。
  证据只到 Node 契约测试与 Linux 上的 `cargo check`/`cargo test`；真实 GUI、真机、WebView、
  GPU/像素全部 **NOT VERIFIED**。
- 日期：2026-11-13。
- 相关：[ADR-0010](0010-react-linux-host.md)（React/Tauri 宿主）、
  [ADR-0023](0023-dom-on-top-cross-host-composition.md)（`native-below` 合成）、
  [ADR-0024](0024-dom-canvas-wasm-cad-path.md)（`dom-canvas` 载体）、
  [ADR-0029](0029-plugin-manager.md)（厂商解码器插件管理器）、
  [ADR-0030](0030-vendor-entity-selection-and-plugin-properties.md)（插件属性/命令提供者）。
- 影响：新增非 Cargo 的 TS 包 `packages/cad-plugin-api`、`apps/desktop-ui/src/plugin-api`、
  `crates/cad-ui-core/src/session/api.rs` 与共享描述符 `crates/cad-ui-core/api/plugin-api.json`、
  一个共享 wire 命令 `cad_api_status`。**不**改变：`cad-db` 唯一权威、单一 React 树、
  `native-below`/`dom-canvas` 的文档所有权、厂商 ABI v1 的二进制帧、渲染器 GPU 边界。

---

## 1. 问题

CAD 二次开发需要一个**稳定的、面向第三方**的编程接口。AutoCAD 的既有事实标准是 .NET API
（`Autodesk.AutoCAD.ApplicationServices` / `DatabaseServices` / `EditorInput` / `Runtime` /
`Geometry` / `Colors`）。本项目的 UI 是 WebView 里的 React，插件只能是 ES module，因此需要一个
**TypeScript 版的 AutoCAD .NET 语义子集**，并且必须回答：

> 插件在 WebView 里写 `doc.Database` / `tr.GetObject` / `ed.GetPoint` 时，调用到底落到哪里？
> 哪些能力今天真的存在，哪些必须显式拒绝？API 代码放在仓库哪里才不会和宿主/厂商插件混在一起？

约束：

- `cad-db` 是唯一权威；插件访问不了它，只能通过宿主代理，`native-below` 与 `dom-canvas` 的
  文档所有者不同（native 会话 / wasm 会话）。
- 现有 wire 只有一个文本命令漏斗 `cad_command`，**没有**结构化请求通道；结构化编辑 API
  （`cad_app::ApiRequest`）是 Rust-only，未上线。
- 已有厂商插件 ABI v1（`decode` 二进制帧 + 只读属性/命令提供者）是**另一件事**：它只解码
  `Opaque` 图元，不碰数据库。二者不能混成一套 ABI。
- 不允许占位成功：尚未接线的操作必须给出稳定拒绝码，而不是空对象或假数据。

## 2. 决策

### D1 三层目录：契约包 / 宿主 adapter / 共享能力描述符

| 层 | 路径 | 职责 | 禁止 |
|---|---|---|---|
| 契约 + 参考客户端 | `packages/cad-plugin-api/`（`@yacr/plugin-api`，非 Cargo） | 命名空间类型、事务簿记、manifest 解析、能力门控；**零宿主依赖**（不 import Tauri/React/`apps/`） | 不含 CAD 实现、不发 IPC、不读文件系统 |
| 宿主 adapter | `apps/desktop-ui/src/plugin-api/` | 实现 `CadHostBridge`：`hostInfo()` 取 `cad_api_status` + 状态快照，`request()` 转发 wire 或按名拒绝 | 不复制命名空间语义；不自己实现事务规则 |
| 能力描述符 | `crates/cad-ui-core/api/plugin-api.json` + `src/session/api.rs` | 唯一权威的操作表；`cad_api_status` 原样返回 | 不得由 TS 端单方面声明支持 |

**理由**：插件作者只依赖一个包；语义只有一份实现（在契约包里，Node 可测）；宿主 adapter 薄到
只剩传输与拒绝；Rust 侧只提供"本构建到底支持什么"的事实。与 i18n 目录同构——共享权威数据放在
`crates/cad-ui-core` 下，TS 侧按同一份文件做契约测试。

**`packages/` 是新顶层目录**：这是仓库第一个非 Cargo、非应用的发布型 TS 包。它不属于
`apps/desktop-ui`（那会迫使插件 import 宿主源码，并把 API 打进 UI 的 `src` 污染职责表），也不属于
`plugins/`（那是厂商插件产物）。`apps/desktop-ui` 用**相对路径**（带 `.ts` 扩展）引用它，而不是
tsconfig/vite 别名或裸包名：仓库没有 npm workspace、也没有签入的 `node_modules`，别名只在 `tsc`
里成立、裸包名只在发布打包后成立，只有相对路径能让 `tsc` 与 `node --test` 同时解析到同一份源码；
`packages/cad-plugin-api/test/surface.test.ts` 据此断言 `src/` 的每个 import 都是相对路径且不越出
包目录（`@tauri-apps`/`react`/`apps/`/`crates/` 一律拒绝）。三段产物（桌面 `dist`、Web
`dist-ui`、Android `dist`）不新增第四份输出。

### D2 镜像命名空间，惯用 TS

- 命名空间与类型名对齐 AutoCAD：`ApplicationServices`、`DatabaseServices`、`EditorInput`、
  `Runtime`、`Geometry`、`Colors`；类型名保留 `Document`/`Database`/`DBObject`/`Entity`/`Line`/
  `Polyline`/`BlockTableRecord`/`LayerTableRecord`/`ObjectId`/`Handle`/`Transaction`/
  `TransactionManager`/`PromptStatus`/`SelectionSet`/`CommandFlags`。
- TS 惯用法：**async 而非阻塞**（wire 是异步的）；**union + `const` 对象而非 `enum`**（可剥离
  语法）；`Point3d` 等是 readonly 值类型 + 构造函数，不是可变类；无 `IDisposable`（用显式
  `commit`/`abort` 与作用域）；无全局静态 `Application`（`PluginContext` 注入，按文档身份寻址）。
- 明确**不移植**：`SendStringToExecute` 的异步排队语义、`DocumentLock` 的线程锁、COM Interop、
  `PaletteSet`/`StatusBar`/`TrayItem`、托管 `Drawable` 自定义渲染、`Session` 命令上下文双模式。
- 命令注册的形状保留（`CommandFlags` 子集、`[CommandMethod]` → `registerCommand`），但**本轮
  不接入命令漏斗**（见 D6）。

**理由**：迁移成本主要在概念（ObjectId、事务打开模式、符号表），不在拼写；照搬 `Get`/`Set` 前缀
与 `ObjectId` 内存地址语义会把 C# 的坑一起搬过来。

### D3 能力协商：三态、单一权威、客户端前置门控

`cad_api_status`（`shared` + `pure`）返回：

```json
{ "apiVersion": 1,
  "namespaces": ["ApplicationServices", "DatabaseServices", "EditorInput", "Runtime", "Geometry", "Colors"],
  "operations": [
    { "namespace": "DatabaseServices", "operation": "transaction.commit", "status": "pending",
      "reason": "api.pending: DatabaseServices.transaction.commit" },
    { "namespace": "Runtime", "operation": "runtime.registerCommand", "status": "pending",
      "reason": "api.pending: Runtime.runtime.registerCommand" }
  ] }
```

- `supported`：宿主本构建真的服务该操作；**不得**带 `reason`。
- `pending`：契约已定、实现未落地；`reason` 必须是 `api.pending: <namespace>.<operation>`。
- `unsupported`：该载体永不提供；`reason` 以 `unsupported-runtime: ` 开头。

客户端规则（`@yacr/plugin-api` 的参考客户端，不是宿主）：

1. `createCadApi(bridge)` 先取 `hostInfo()`；`apiVersion` 不匹配 → 拒绝
   `api.host-outdated: <version>`。
2. 每次受门控的调用先查操作表：`pending` → 抛 `CadApiError('api.pending: …')`且**不调用**
   `bridge.request`；显式 `unsupported` 行 → `unsupported-runtime: cad_api.…`；表里**没有**这一
   行 → `api.descriptor_missing: …`（契约失配，不是载体缺能力）。三者都不触达宿主。
3. `supported` 才发请求；宿主返回无法识别的形状 → `api.invalid_argument: bridge.answer`，绝不
   把空值当成功；`callVoid` 还要求 `{ accepted: true }` 的显式确认。
4. 纯值类型（Geometry/Colors）与 manifest 解析不经过 wire，因此**没有描述符行**，永远可用。
5. 自洽性：`documentOwner: 'none'` 却出现 `supported` 的 `ApplicationServices`/`DatabaseServices`
   行 → 拒绝 `api.descriptor_invalid: documentOwner`（没有文档所有者却声称服务文档操作）。

**本版本状态（诚实表）**：描述符 21 行——**18 行 `pending` + 3 行 `supported`**
（`documentManager.mdiActiveDocument`、`documentManager.documents`、
`database.revision`）。`ApplicationServices`（3，两项已服务）、`DatabaseServices`（8，一项已
服务）、`EditorInput`（9）、`Runtime.registerCommand`（1）其余全 `pending`。**插件只能读取
文档身份、文档列表与当前 revision；不能读实体/符号表、不能提示输入、不能注册可执行命令。** Geometry/Colors/`parsePluginManifest` 不是宿主能力，不进表，由包
无条件提供。这不是遗漏，是描述符明示的下一步。

**理由**：能力来自宿主，TS 端不得单方面宣布支持；把"未实现"建模成稳定拒绝码，插件作者在编译期
之外还能拿到可解释的运行时错误。

### D4 身份与事务：不透明 ObjectId、乐观 revision、LIFO

- `ObjectId`/`Handle` 是**不透明品牌字符串**。插件不得解析、拼接或跨文档复用；宿主负责
  `Handle → ObjectId` 的解析。`DBObject.handle` 只是导入出处（`source_handle`，ADR 0030 D1），
  **通过本 API 创建的实体是 `null`**，解码接受缺失/null 且永不合成。
- 图层与块按**名字**寻址，**没有 handle**：`cad-db` 的层是 `{ id, name, visible }`，linetype 是
  符号化的命名概念，给它们造 handle 等于第二套身份系统。`Entity.layer` 是层名，
  `Entity.linetype` 是 `byLayer`/`byBlock`/`named` 三态；`LayerTable`/`BlockTable` 用
  `record(name)`/`names()` 查询，`Database.clayer` 返回层名。
- `TransactionManager.startTransaction({ expectedRevision })` → `Transaction`：最外层
  **必须**给 revision（Rust 契约是 `expected_revision: Revision`，非可选）；省略就拒绝
  `api.invalid_argument: expectedRevision`，因为"没有前置条件"等于隐式 last-writer-wins。调用方
  先读当前 revision（读操作在 M2 落地），嵌套事务继承外层前置。
- `getObject(id, mode)` 在**任意深度**可用，按宿主答案的 `typeName` 标签解码
  （`Line`/`Circle`/`Arc`/`Polyline`/`DBText`/`MText`）成判别联合，未知/缺失标签是
  `api.invalid_argument: bridge.answer.typeName`；没有调用方指定的 `as T`。
- `addNewlyCreatedDBObject(obj, add)` 也在任意深度可用：实体类型是**闭集**
  `Line`/`Circle`/`Arc`（`SUPPORTED_NEW_ENTITY_TYPES`），带 layer **名字**与 host 定义的 space；
  同一事务对同一 `ObjectId` 的第二次 `translate`/`delete` 是
  `api.invalid_argument: edits.duplicate`（对齐宿主"一个批次不能重复编辑同一实体"）。
- 嵌套 **LIFO 只约束 `commit`/`abort`**（外层先关闭 = `api.transaction.nesting_order`），深度
  上限 8（`api.transaction.nesting_limit`），重复关闭 = `api.transaction.closed`。
- 只有最外层 `commit()` 走 wire，信封是
  `{ document, transactionId, expectedRevision, edits }`：`document` 来自 `DocumentDescriptor.id`
  （manager 按文档创建），`transactionId` 只是**一次提交的关联 id**，不是宿主事务句柄。
  `CommitResult` 是 `{ applied, revision, created }`：嵌套 commit 是**保存点释放**（并入父事务、
  不发请求）返回 `{ applied: false, revision: null, created: [] }`，**不编造 revision**；最外层返回
  宿主 `revision` 与 `ApiOutcome.created`。
- `abort()` 本轮**纯本地**（丢弃当前缓冲、弹栈、不触达宿主）：宿主从未签发事务 id，远端 abort
  只会被拒并泄漏层级。将来 `transaction.start` 由宿主签发 id 时，abort 才成为 wire 调用，描述符
  保留 `transaction.abort` 行为此预留。合并/迁移缓冲由 manager 拥有（模块级 `WeakMap`），公开的
  `Transaction` 没有第二个写入入口。
- revision 冲突由宿主判定，映射为 `api.revision-conflict`；Rust 侧是 `CadError::StaleResult`，
  **不带实际 revision**，因此 `details.actualRevision` 只在宿主给出时出现（M3 决定怎么取）。
  客户端**不**自动重试（重试会把插件意图应用在错误基础上）。
- 一个事务 = 一次提交 = 一个 revision；不实现 AutoCAD 的 `DocumentLock`/线程语义（单 WebView
  主线程 + 宿主 owner 线程）。

**理由**：AutoCAD 的 `ObjectId` 是内存地址、`Handle` 跨图不唯一；直接照搬会产生"跨图 id 冲突"
与"插件缓存 id 永不失效"两类错误。乐观 revision 是宿主已有的并发契约（`ApiRequest::Commit`
的 `expected_revision`），复用它而不是新造锁。

### D5 信任模型：同权 ES module，代理不是沙箱

- 插件仍是**不可信、同权**的 module script（与 ADR 0029 的厂商解码器同一信任级别）；它拿到的
  `CadApi` 是宿主注入的代理对象，**不是** `cad-db` 的句柄。插件无法写库、无法改选择、无法伪造
  `ObjectId`。
- 拦截/权限清单/Worker 隔离**未实现**；API 不是安全边界，文档必须写明。
- 沿用 ADR 0029 的生命周期决定：安装/更新在下次启动生效，**没有热重载**。

### D6 载体一致性与"本轮不接命令漏斗"

- `cad_api_status` 是 `shared` 行：`native-below`（Tauri 生成的 wrapper）与 `dom-canvas`
  （wasm `DirectSink`）服务同一份描述符。未来受门控的操作也必须**先在 `cad-ui-core` 定义一次**，
  再按 `surfaceMode`/`documentOwner` 接线；不按构建目标分叉。
- Web 在 canvas 会话启动前也直接应答描述符（`apps/app-web/src/direct.rs::without_session`）：
  它是编译期契约，不是文档状态，不能变成假的 `no live CAD session`。
- **Runtime.registerCommand 标记 `pending` 且调用即拒绝**：命令漏斗（`admitCommandText`）只承认
  宿主词汇表里的命令，把插件命令塞进去需要新的 chrome-local 准入路径，本轮不做。宁可显式拒绝，
  也不注册一个永远不会被派发的命令。

### D7 与厂商插件 ABI 的边界

| | 厂商解码插件（ADR 0029/0030） | 二次开发 API（本 ADR） |
|---|---|---|
| 目的 | 解码 `Opaque`/`TCH_*`，提供只读属性面板与命令拦截 | 通用文档/编辑器编程接口 |
| 契约 | `abiVersion=1`，`decode` 二进制帧 + `register(host)` | `apiVersion=1`，命名空间代理 + JSON 请求 |
| 数据 | 派生显示，不回写 | 目标是事务性读写（本轮未实现） |
| 加载 | `plugin.js` + `manifest.json`（ABI 1） | `plugin.json`（`manifestVersion/apiVersion` 分离） |
| 信任 | 同权 module，非沙箱 | 同权 module，非沙箱 |

两者**不合并**：厂商解码器是"渲染前的一次纯函数"，二次开发 API 是"有状态、跨事务"的接口；
合并会让 `decode` 的帧契约承担它不该承担的语义。

## 3. 备选方案与弃用理由

| 方案 | 弃用理由 |
|---|---|
| API 直接放进 `apps/desktop-ui/src` | 插件必须 import 宿主源码；API 与 UI 职责表纠缠；无法独立类型检查/发布 |
| API 放进 `plugins/` | 那是厂商插件产物目录（git submodule / bundle），不是库 |
| 扁平惯用 TS（`cad.db.entities.find()`） | 放弃 AutoCAD 迁移心智；用户明确要求镜像命名空间 |
| 严格 1:1 移植（`ObjectId` 内存地址、`IDisposable`、同步 `commit`） | 把 C# 生命周期缺陷搬进 WebView；与异步 wire 冲突 |
| 插件本地缓存整库快照 | 一致性/失效成本高，且与"`cad-db` 唯一权威"冲突 |
| Worker 隔离 + RPC | 方向正确，但需跨 worker 帧协议与新的输入通道；本轮不把它伪装成已完成 |
| 能力表写死在 TS | 宿主升级后 TS 会谎报支持；必须由宿主返回 |

## 4. 数据契约

```ts
// 宿主 → 客户端（cad_api_status 原样 + 载体快照）
interface CadApiHostInfo {
  apiVersion: number;
  carrier: 'native-below' | 'dom-canvas' | 'unknown';
  documentOwner: 'native' | 'wasm' | 'none';
  operations: readonly CadApiOperation[];   // { namespace, operation, status, reason? }
}

// 客户端 → 宿主（本轮所有受门控 operation 均为 pending，故不会真正发出）
interface CadApiRequest {
  namespace: CadApiNamespace;
  operation: string;
  parameters: Readonly<Record<string, unknown>>;
}

// transaction.commit 的 parameters（唯一携带 document 的信封）
interface CommitParameters {
  document: string;              // DocumentDescriptor.id，manager 按文档创建
  transactionId: string;         // 一次提交的关联 id，不是宿主事务句柄
  expectedRevision: number;      // 必填：Rust 契约是 expected_revision: Revision
  edits: readonly TransactionEdit[];
}

type TransactionEdit =
  | { kind: 'insert'; typeName: 'Line' | 'Circle' | 'Arc';
      layer: string;             // 层名，不是 handle
      space: string;             // host 定义的 space id/name，必须等于 active space
      parameters: Readonly<Record<string, unknown>> }
  | { kind: 'translate'; objectId: ObjectId; displacement: Vector3d }
  | { kind: 'delete'; objectId: ObjectId };

// commit() 的答案：applied 才是"真的写入了"，嵌套提交是保存点释放
interface CommitResult {
  applied: boolean;
  revision: number | null;       // 只有 applied 时来自宿主，否则 null
  created: readonly ObjectId[];  // 宿主 ApiOutcome.created
}

interface CadHostBridge {
  hostInfo(): Promise<CadApiHostInfo>;
  request<Result>(request: CadApiRequest): Promise<Result>;
}

// 插件清单（plugin.json），与厂商 manifest 刻意不同名
interface PluginManifest {
  manifestVersion: 1;
  id: string;            // /^[a-z0-9][a-z0-9_-]{0,63}$/
  name: string;
  version: string;
  apiVersion: number;    // 必须等于 1
  entry: string;         // *.js / *.mjs，无 ..、无绝对路径、无反斜杠
  commands?: readonly string[];  // /^[A-Z][A-Z0-9_]{0,31}$/
}
```

拒绝码（稳定、可搜索）：`api.pending: <ns>.<op>`、`unsupported-runtime: cad_api.<ns>.<op>`
（**只**来自显式 `unsupported` 行）、`api.descriptor_missing: <ns>.<op>`（宿主描述符没有这一行）、
`api.descriptor_invalid: <detail>`（如 `documentOwner`）、`api.host-outdated: <version>`、
`api.revision-conflict`、`api.transaction.nesting_order`、`api.transaction.nesting_limit`、
`api.transaction.closed`、`api.invalid_argument: <field>`（含 `edits.duplicate`、
`bridge.answer`）、`api.manifest.*`、`api.adapter-missing: <ns>.<op>`。

## 5. 载体矩阵

| 载体 | `surfaceMode` | `cad_api_status` | 受门控操作 | 说明 |
|---|---|---|---|---|
| Linux 原生 Wayland | `native-below` | 由 Tauri wrapper 服务 | 三项只读（owner-tick 镜像），其余 `pending` 拒绝 | 描述符一致，本轮无运行证据 |
| Linux X11/XWayland | `dom-canvas` | 由 wasm `DirectSink` 服务 | 同上（读实时 `CadSession`） | 同一份 JSON、同一条 wire 名 |
| Windows / macOS / Android | `native-below` | 生成 wrapper | 同上 | 仅编译/契约，未运行验证 |
| Web | `dom-canvas` | wasm 会话 | 同上 | Web 没有插件目录；本轮不装插件 |

"契约支持"只表示命令/拒绝语义在同一共享层定义并有测试，**不**表示该平台已运行验证。

## 6. 测试与证据

第一轮（骨架落地，2026-11-13）已运行：

- `packages/cad-plugin-api`：`tsc --noEmit` 通过；`node --test test/*.test.ts` **32/32**（命名空间
  形状、manifest 拒绝码、事务 LIFO/深度/重复关闭/乐观 revision、能力门控 pending 不触达 bridge、
  与 `crates/cad-ui-core/api/plugin-api.json` 的 27 行逐行契约）。**该行随后被第二轮取代**：
  能力表归一后是 20 行、全部 pending，测试套件是 45 项（见下）。
- `apps/desktop-ui`：`tsc --noEmit`（含跨包相对引用）通过；`node --test test/plugin-api/*.test.ts`
  **23/23**；`test/host-adapter.test.ts` **36/36**；全量 node 套件 **1518/1529**——11 个失败全部是
  `plugins/tch` submodule 未初始化（`ERR_MODULE_NOT_FOUND …/plugins/tch/src/index.ts`），与本轮无关。
- Rust：`cargo test -p cad-ui-core --test plugin_api_status` **6/6**、
  `cargo test -p cad-desktop-shell --test wire_goldens` **4/4**、`cargo test -p app-web` **122/122**
  （含新增的"无会话也应答描述符"用例）、`cargo check -p cad-ui-core/-p cad-desktop-shell`、
  `cargo check -p app-web --target wasm32-unknown-unknown`、`cargo fmt --check` 均通过。
- 门禁：`check-file-size.py`、`check-i18n.py`、`check-architecture.py` 通过。

第二轮（能力表归一 + 事务/身份修正，2026-11-13）已运行：

- `packages/cad-plugin-api`：`tsc --noEmit` 通过；`node --test test/*.test.ts` **45/45**——20 行
  逐行契约、`pending`/`unsupported`/`descriptor_missing` 三态、`{ accepted: true }` ack、
  保存点提交/本地 abort/必填 `expectedRevision`/闭集实体类型/重复编辑、
  `typeName` 解码，以及"`src/` 的 import 必须相对且不越出包目录"的边界测试。
- `apps/desktop-ui`：`tsc --noEmit` 通过；`node --test test/plugin-api/*.test.ts
  test/host-adapter.test.ts` **60/60**（含"每个 `supported` 行必须有 adapter 实现"）；
  `test/host-adapter.test.ts` 钉住的 `npm test` 字符串未变。初始化 `plugins/tch` submodule 后，
  **`npm test` 整条**（app + 契约包 + 插件三处 typecheck，再加全部 node 套件）**1630/1630** 通过。

- Rust：`cargo check -p cad-ui-core --locked` 通过；`cargo test -p cad-ui-core --test
  plugin_api_status` **6/6**、`--lib session::api` **5/5**、`cargo test -p cad-desktop-shell --test
  wire_goldens` **4/4**、`cargo test -p app-web` **122/122**。第二轮一度因
  `assert!(supported > 0)` 失败（旧测试断言"本构建支持纯值类型"，与 20 行全 pending 冲突）；该断言
  已改为"没有任何 `supported` 行"并重跑全绿。
- 门禁：`check-file-size.py`、`check-i18n.py`、`check-architecture.py`、`check-command-docs.py`
  （及其自测）、`cargo fmt --check` 通过。

第三轮（第一条只读操作 `cad_api_call`，2026-11-13）已运行：

- Rust：`cargo test -p cad-ui-core --lib` **607 passed**（含 `session::api_call` 9 项与
  `tests_wire` 的 snapshot 两种 `None` 语义）、`cargo test -p app-web` **123/123**（矩阵含
  `cad_api_call` 的成功/未知/无文档/`cad_waiting` 用例）、`wire_goldens` 4/4、
  `cargo check -p app-linux --all-targets`、`cargo fmt --check`、`check-architecture` /
  `check-file-size` 通过。
- TS：包 `tsc` + 48/48（1 行 supported 由名字钉住）、desktop-ui `tsc` + plugin-api/host-adapter
  66/66、`npm test` 整条 **1639/1639**。
- **NOT VERIFIED**：Windows/macOS/Android 的 tick 赋值只有同形代码，未在该平台编译/运行；
  真实 GUI 读取（镜像最多滞后一个 owner tick）未验证。
- 第四轮补充：`documents`/`database.revision` 走同一快照；`cargo test -p cad-ui-core --lib
  session::api` 21、`app-web` 123、`plugin_api_status` 6/6；包 node 53/53、desktop-ui
  plugin-api+host-adapter 70/70、`npm test` 整条 **1648/1648**。

**NOT RUN / NOT VERIFIED**：真实 GUI 中加载插件、任何 `supported` 的文档/编辑器操作、命令派发、
Windows/macOS/Android/Web 运行、真实 WebView CSP、GPU/像素、插件隔离与签名。

## 7. 后果与未决

- 里程碑（明确、不在本轮）：
  - **M2** 只读操作：`cad_api_call` 通道与 `documentManager.mdiActiveDocument` / `.documents`、
    `database.revision` **已落地**；剩余 `database.symbolTables`、`editor.currentSelection`
    （继续沿用同一条 `reads` 车道与镜像/实时双实现）。
  - **M3** 事务提交：`expectedRevision`、编辑缓冲、撤销分组、冲突映射。上线前必须解决：
    - `translate`/`delete` 要映射到 `SelectionRef { document, entity, instance, sub_element }`；
      instanced / 子元素引用由宿主侧拒绝（客户端只有 `ObjectId`，没有 instance/sub-element 位）。
    - 宿主 `ApiOutcome.created` 必须回填到 `CommitResult.created`（客户端已校验形状，M3 只需接线）。
    - Rust 的 `expected_revision` 不匹配是 `CadError::StaleResult`，**不带** actual revision；
      要么 M3 定义一次廉价 revision 读取，要么让 `details.actualRevision` 永远缺席——不能编造。
    - 每次提交只产生**一条** undo 记录，label 硬编码英文、`merge_key: None`；M3 要加分组
      （同一插件事务的连续提交合并）。
    - 宿主拒绝类需要稳定 TS code 才能上线写入：`layer.locked`、`CadError::Unsupported`、
      space 与 active space 不一致（当前只有自由文本/`InvalidInput`）。

  - **M4** 插件装载（`plugin.json` + 入口模块）+ `Runtime.registerCommand` 接入 chrome-local
    命令漏斗。
  - **M5** Worker/权限清单/签名隔离。
- 债务：**已修**——`apps/desktop-ui` 的 `npm test` 现在会 typecheck 契约包
  （`tsc -p ../../packages/cad-plugin-api/tsconfig.json --noEmit`）并跑包测试
  （`node --test ../../packages/cad-plugin-api/test/*.test.ts`），该字符串由
  `test/host-adapter.test.ts` 钉住，不再需要维护者手工确认 CI 覆盖。剩余：插件命令尚无准入
  路径；`dom-canvas` 的插件装载/持久化仍未定义（ADR 0029 已显式排除）。
- 不变量：插件 API 永远不是文档权威；所有写路径经宿主事务；不支持的能力显式拒绝；不按构建目标
  分叉语义。
