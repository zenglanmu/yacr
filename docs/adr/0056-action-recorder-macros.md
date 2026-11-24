# ADR 0056：动作录制器宏（ACTRECORD/ACTSTOP/ACTUSERINPUT/ACTMANAGER）

- 状态：**Accepted（2026-11-24）**。实现未开始；命令状态在实现与契约测试落地前仍是 `部分实现`
  （具名拒绝）。
- 关联：[ADR-0034](0034-typescript-plugin-api.md)（插件/命令边界）、
  [ADR-0053](0053-application-domains.md)（cad2d 面板摆放）、
  [ADR-0043](0043-parametric-dependency.md)（命令事务/失败边界）。
- 影响：`cad-app`（命令捕获/回放、宏文件 codec）、`cad-ui-core`/`apps/desktop-ui`
  （Manage→Action Recorder 面板）、`cad-host`（native 文件 I/O）、`cad-ui-core` 命令词表。

## 背景

AutoCAD 的动作录制器把用户命令序列保存为宏（`.actm`）并可回放。当前构建没有记录器状态、
宏文件格式、回放器或面板；`ACTRECORD`/`ACTSTOP`/`ACTUSERINPUT`/`ACTUSERMESSAGE`/
`ACTMANAGER` 及其 `-` 拼写全部是具名拒绝。命令已经通过单一 `cad-app` 派发漏斗和事务边界，
因此记录“成功的命令 + 载荷”是可行的，不需要第二套执行引擎。

## 决策

### D1 — 宏是外部版本化文件，不是 `.ymodel`

- 新增宿主侧宏文件（建议 `.actm.json` 或 zip+JSON），包含 `format`、`version`、创建时间、
  命令步骤列表（命令名、规范化载荷、可选标签）。
- 不写入 `.ymodel`，不改 `FORMAT_MINOR`；宏不是文档内容。浏览器/wasm 没有文件系统时按
  `unsupported-runtime: action-recorder` 显式拒绝（读/写/回放都拒绝）。
- 不在宏里保存文档/实体 id 以外的私有状态；回放重新走正常命令漏斗。

### D2 — 记录成功命令，不记录内部状态

- `ACTRECORD [name]` 开始记录；从该点起，凡是**已受理并成功执行**的 `cad_command`（含 payload）
  记录为一步；被拒绝/取消/失败的命令不写入宏，并保留一条诊断。
- 交互输入不伪造：`ACTUSERINPUT` 记录一个“等待用户输入”的步骤；`ACTUSERMESSAGE`（及 `-`
  拼写）记录一条面向用户的消息步骤。回放遇到输入步骤时暂停并要求用户继续，不自动猜值。
- 记录器是会话状态；停止/保存前不写文件。宏文件由宿主文件桥写出，失败按名报告。

### D3 — 回放走同一漏斗

- `ACTMANAGER` 列出/选择/回放/删除宏；回放逐步骤调用现有命令漏斗，每一步仍遵守权限、
  文档能力、事务和撤销边界。回放中途被拒绝时停止并报告步骤号；已执行的步骤不回滚成空成功，
  由用户 UNDO 或明确失败处理。
- 回放不直接改 `cad-db`、不绕过 `ChangeScope`/`ChangeSet`/GPU 派生路径。
- 宏不能记录/回放自身（避免递归）：`ACTRECORD` 在录制中再次调用按名拒绝。

### D4 — 命令与面板

- `ACTRECORD` 开始；`ACTSTOP`/`-ACTSTOP` 停止并保存（`-ACTSTOP` 为命令行拼写）；
  `ACTUSERINPUT` 记录输入暂停；`ACTUSERMESSAGE`/`-ACTUSERMESSAGE` 记录消息步骤；
  `ACTMANAGER` 打开管理面板。
- Manage→Action Recorder 面板只在命令真实可用后接线；之前保持 `pending`，不摆假按钮。
- `ACTMANAGER` 的列表/回放/删除是真实操作；没有宏时报告空列表，不伪造行。

### D5 — 非目标

- 不做 AutoLISP/VBA/任意脚本执行；宏只回放 CAD 命令步骤。
- 不做跨文档/跨会话宏库、云端同步、定时录制。
- 不承诺与 AutoCAD `.actm` 二进制格式互换；自有格式必须版本化并能在未来迁移。
- 不做录制期间的 UI 事件级回放（只回放命令/输入步骤）。

## 实施分期

1. **P0 记录/回放核心**：宏 model + 外部文件 codec；`cad-app` 成功命令捕获 hook；回放漏斗；
   拒绝/失败诊断。
2. **P1 命令**：`ACTRECORD`/`ACTSTOP`/`-ACTSTOP`/`ACTUSERINPUT`/`ACTUSERMESSAGE`/
   `-ACTUSERMESSAGE`/`ACTMANAGER`；移除对应具名拒绝。
3. **P2 UI**：Manage→Action Recorder 面板（列表/回放/删除/空状态），i18n/fixture/结构测试。
4. **P3 宿主**：native 文件对话框/路径桥；Web/wasm 显式拒绝。

## 证据边界

以契约测试为主（记录成功/拒绝、文件往返、回放事务/撤销、空宏、Web 拒绝）。真实宿主文件对话框、
Windows/macOS/Android/Web 运行时与 GPU/像素未验证时写 **NOT RUN/NOT VERIFIED**，不得推广。
