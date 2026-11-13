# TypeScript 二次开发 API（`@yacr/plugin-api`）

> 状态：**可用（2026-11-13）**。18 行宿主操作中 **8 行 `supported`**：文档身份/列表、revision、
> 符号表、当前选择、handle→ObjectId、按 id 读实体、事务提交。插件可以运行、注册命令，并在事务
> 里创建/移动/删除实体。仍 `pending` 的只有交互式提示（`editor.get*` 等）与 `document.lock`。
> 包内特性（Geometry/Colors 值构造、manifest 解析、命令注册）不走 wire，不在描述符里。设计依据见
> [ADR 0034](adr/0034-typescript-plugin-api.md)。

AutoCAD 的二次开发事实标准是 .NET API（`Autodesk.AutoCAD.ApplicationServices` /
`DatabaseServices` / `EditorInput` / `Runtime` / `Geometry` / `Colors`）。本项目对外提供的是
**同名命名空间、惯用 TypeScript** 的版本：插件是 WebView 里的 ES module，拿到的是宿主注入的
代理对象，不是 `cad-db` 的句柄。

## 目录（哪里放什么）

| 路径 | 职责 | 不做什么 |
|---|---|---|
| `packages/cad-plugin-api/` | 发布契约 + 参考客户端：命名空间类型、事务簿记、manifest 解析、能力门控 | 不 import Tauri/React/`apps/`，不读文件系统，不含 CAD 逻辑 |
| `apps/desktop-ui/src/plugin-api/` | 宿主 adapter：校验 `cad_api_status`、组装客户端、按名拒绝未实现操作 | 不复制命名空间语义，不自己实现事务规则 |
| `crates/cad-ui-core/api/plugin-api.json` | 操作表的**唯一权威** | 不由 TS 单方面声明支持 |
| `crates/cad-ui-core/src/session/api.rs` | 描述符类型 + `cad_api_status` 的服务端实现 | 不读文档、不碰 `cad-db` |
| `apps/desktop-ui/test/plugin-api/` | adapter 契约测试（payload、载体映射、拒绝路径、supported 行必须有实现） | — |
| `packages/cad-plugin-api/test/` | 客户端行为测试 + 与 JSON 的逐行契约测试 | — |

`packages/` 是本仓库第一个非 Cargo 的发布型 TS 包。宿主以**相对路径**引用它
（`apps/desktop-ui/src/plugin-api/*` → `../../../../packages/cad-plugin-api/src/index.ts`）：
仓库没有 npm workspace，也没有签入的 `node_modules`，裸包名（或 `@plugin-api/*` 别名）在
`tsc` 与 `node --test` 下都解析不到，只有相对路径两边都成立。插件作者侧按包名
`@yacr/plugin-api` 引用（发布时由打包器解析）。

## 载体与 wire

`cad_api_status` 是一个 **shared + pure** 命令：`native-below`（Tauri wrapper）与 `dom-canvas`
（wasm `DirectSink`）返回同一份描述符；Web 在会话尚未启动时也应答它（它是编译期契约，不是文档
状态）。它**不是** CAD 命令，不在 `cad_commands` 词表里。payload 见
[`docs/commands.md`](commands.md#cad_api_status)。

adapter 把描述符与当前 `cad_status` 快照合成 `CadApiHostInfo`：

```ts
{ apiVersion: 1,
  carrier: 'native-below' | 'dom-canvas' | 'unknown',
  documentOwner: 'native' | 'wasm' | 'none',
  operations: [{ namespace, operation, status, reason? }, ...] }
```

`carrier` / `documentOwner` 只用于诊断；**路由与门控只看 `operations`**，不按构建目标分叉。
唯一的例外是自洽性检查：`documentOwner: 'none'` 却出现 `supported` 的
`ApplicationServices`/`DatabaseServices` 行时，`createCadApi` 拒绝
`api.descriptor_invalid: documentOwner`——没有文档所有者却声称服务文档操作是宿主 bug。

## 插件作者视角

```json
// plugin.json —— 与厂商解码器的 manifest.json 刻意分开
{ "manifestVersion": 1, "id": "my-tools", "name": "My Tools", "version": "0.1.0",
  "apiVersion": 1, "entry": "index.js", "commands": ["MYCMD"] }
```

```ts
import { definePlugin, CommandFlags, CadApiError } from '@yacr/plugin-api';
import type { PluginContext } from '@yacr/plugin-api';

export default definePlugin({
  id: 'my-tools',
  apiVersion: 1,
  async activate(ctx: PluginContext) {
    const doc = await ctx.api.application.documentManager.mdiActiveDocument();
    if (!doc) return;                       // 没有活动文档是事实，不是错误

    const layers = await doc.database.symbolTables;
    const revision = await doc.database.revision();

    // 一个事务 = 一次提交；expectedRevision 必填（乐观并发）
    const tr = doc.transactionManager.startTransaction({ expectedRevision: revision });
    tr.addNewlyCreatedDBObject({
      typeName: 'Line', layer: '0', space: 'model',
      parameters: { start: [0, 0, 0], end: [100, 0, 0] },
    }, true);
    const { revision: next, created } = await tr.commit();

    ctx.api.runtime.registerCommand('MYCMD', CommandFlags.modal, async (c) => {
      const selection = await c.api.editor.currentSelection();
      // created[0] / selection 里的 ObjectId 都可直接用于 translate/delete
      void next; void selection;
    });
  },
});

// 分支只认 code，不认 message
try { /* ... */ } catch (error) {
  if (error instanceof CadApiError && error.code === 'api.revision-conflict') { /* 重读 revision */ }
}
```

生命周期：宿主加载 `entry` → 调一次 `activate(ctx)` → 卸载/退出时调一次 `deactivate(ctx)`。
**没有热重载**（沿用 ADR 0029 的"持久化后重启"）。

## 能力三态与拒绝码

| 状态 | 含义 | 客户端行为 |
|---|---|---|
| `supported` | 本构建真的服务 | 才发请求；答案形状不对即 `api.invalid_argument: bridge.answer` |
| `pending` | 契约已定、实现未落地 | `api.pending: <Namespace>.<operation>`，**不触达宿主** |
| `unsupported` | 本载体永不提供 | `unsupported-runtime: cad_api.<Namespace>.<operation>`，**不触达宿主** |
| 表里没有 | 宿主描述符不认识这个操作 | `api.descriptor_missing: <Namespace>.<operation>`，**不触达宿主** |

三态拒绝是**三种不同的失败**，不要合并：`pending` 是"保留但还没做"，`unsupported` 是"这个
载体永远没有"，`descriptor_missing` 是"两边对契约的理解不一致"。描述符只列宿主操作，所以
Geometry/Colors 与 manifest 解析**没有行**；它们由包无条件提供，查询它们属于契约失配。

其它稳定码：`api.host-outdated: <version>`、`api.descriptor_invalid: <detail>`、
`api.revision-conflict`、`api.transaction.nesting_order|nesting_limit|closed`、
`api.invalid_argument: <field>`（含 `edits.duplicate`、`bridge.answer`）、`api.manifest.*`、
`api.adapter-missing: <ns>.<op>`（宿主说支持但 adapter 没有实现——必须响亮失败）。

`callVoid` 语义：无返回值的操作（`editor.writeMessage`、`document.lock`）必须回
`{ accepted: true }`；`{ accepted: false }`、`{}`、`undefined` 都是
`api.invalid_argument: bridge.answer`——没有"静默成功"。

## 事务语义（已接线）

- `startTransaction({ expectedRevision })` **同步**，最外层**必须**给 revision：省略就是隐式
  last-writer-wins，客户端直接 `api.invalid_argument: expectedRevision`。嵌套事务继承外层前置。
- 只有最外层 `commit()` 走 wire，携带 `{ document, transactionId, expectedRevision, edits }`；
  `document` 是 `TransactionManager` 所属文档 id，`transactionId` 只是**一次提交的关联 id**，
  不是宿主事务句柄。
- `CommitResult` 是 `{ applied, revision, created }`：嵌套 `commit` 是**保存点释放**，合并进父
  事务、不发送请求，返回 `{ applied: false, revision: null, created: [] }`——**不编造 revision**；
  最外层返回宿主的 `revision` 与 `ApiOutcome.created`。
- 嵌套**严格 LIFO**（仅对 `commit`/`abort`），深度上限 8；`getObject` 与
  `addNewlyCreatedDBObject` 在任意深度都可用。重复关闭 → `api.transaction.closed`。
- `abort()` 本轮**纯本地**：丢弃缓冲、弹栈、不触达宿主（宿主从未签发事务 id，远端 abort 只会
  被拒并泄漏层级）。将来 `transaction.start` 由宿主签发 id 时，abort 才变成 wire 调用；
  描述符保留 `transaction.abort` 行为此预留。
- 新建实体是**闭集** `Line`/`Circle`/`Arc`，带 layer **名字**与 active space；同一事务对同一
  `ObjectId` 的第二次 `translate`/`delete` 是 `api.invalid_argument: edits.duplicate`（与宿主
  "一个批次不能重复编辑同一实体"一致，本地先拒）。
- `commit()` 是 **Promise**（`cad_api_submit` + `cad_api_poll`）：宿主返回 `{ revision, created }`，
  冲突映射为 `api.revision-conflict`；`details.actualRevision` 目前缺席——Rust 的
  `CadError::StaleResult` 不带实际 revision，客户端不编造。
- 客户端**不**自动重试。

## 身份（不伪造 handle）

- `ObjectId` / `Handle` 是不透明品牌字符串：不解析、不伪造。`DBObject.handle` 只是导入出处
  （`source_handle`），**通过本 API 创建的实体是 `null`**（ADR 0030 D1）；解码接受缺失/null，
  永不合成。
- 图层与块按**名字**寻址：`cad-db` 的层是 `{ id, name, visible }`，没有 handle；linetype 是
  符号化的命名概念。`Entity.layer` 是层名，`Entity.linetype` 是
  `{ method: 'byLayer' | 'byBlock' | 'named', name? }`；`LayerTable`/`BlockTable` 用
  `record(name)` / `names()` 查询，记录本身没有 handle。
- `getObject(id, mode)` 按宿主答案的 `typeName` 标签解码（`Line`/`Circle`/`Arc`/`Polyline`/
  `DBText`/`MText`），返回判别联合；未知/缺失标签是
  `api.invalid_argument: bridge.answer.typeName`，没有调用方指定的 `as T`。

## 明确不提供（本轮）

交互式提示（`editor.getPoint/getEntity/getSelection/getString/getDouble/getInteger/getKeyword/
writeMessage/selectAll`）、`document.lock`、插件安装 UI / 持久化商店、Worker/权限/签名隔离。
`dom-canvas` 没有插件目录；插件由宿主调用 `loadPluginModule` 装载。插件是**同权 ES module，
不是沙箱**。

## 里程碑

| | 内容 |
|---|---|
| M2 | ✅ 只读：文档、revision、符号表、当前选择、handle→ObjectId、按 id 读实体 |
| M3 | ✅ 事务提交：`expectedRevision`、编辑缓冲、`created`、冲突映射（撤销分组仍是一条记录） |
| M4 | ✅ 装载（模块/URL）+ `Runtime.registerCommand` → chrome-local 命令漏斗（安装 UI 未做） |
| M5 | Worker/权限清单/签名隔离 |

## 证据边界

契约测试（Node / `cargo test`）只证明**接线与拒绝语义**；真实 GUI 加载插件、真机/WebView、
GPU/像素、任何 `supported` 的文档/编辑器操作都 **NOT VERIFIED**。
