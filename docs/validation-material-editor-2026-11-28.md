# 材质编辑命令（MATERIAL）（2026-11-28）

新增 `MATERIAL <name> RENAME|COLOR|TRANSPARENCY|DELETE`：直接编辑文档材质行，一步撤销；
delete 同时清除实体/面赋值。面板文档材质行增加 Delete。设计见
[ADR-0099](adr/0099-material-editor-commands.md)。无格式档位变化。

## 1. 已接线

- **命令**：改名保 seq/赋值；颜色/透明度校验；删除清实体与面赋值；未知 `material.unknown`、
  无变化 `material.no_change`；Work-only；一次整表写入/一步撤销。
- **typed**：`MATERIAL` grammar 支持引号名称与四种操作；命令表 264。
- **UI**：文档材质色块重涂 + Delete（`MATERIAL "<name>" DELETE`，名称含 `"`/换行禁用）。
- **边界**：名称/颜色编辑表单、贴图/材质库、拖放未建模。

## 2. 验证

| 命令 | 结果 |
|---|---|
| `cargo test -p cad-app --lib --locked` | PASS（**3565 passed**，materials 新增 rename/color/transparency/delete/refusal） |
| `cargo test -p cad-ui-core --lib --locked` | PASS（**996 passed**，MATERIAL grammar 2） |
| `cargo test -p cad-host --lib --locked` | PASS（**508 passed**） |
| `(cd apps/desktop-ui && npm test)` | PASS（**2151 passed**，MATERIAL delete 命令 1） |
| `cargo fmt --all -- --check` | PASS |
| `python3 scripts/check-file-size.py` | PASS |
| `python3 scripts/check-i18n.py` | PASS（3770 keys） |
| `python3 scripts/check-architecture.py` / `check-command-docs.py` | PASS |

**边界**：没有真实 GUI 证据；命令/面板只有前端命令生成与组件接线，真实点击与其它载体运行时
**NOT VERIFIED**。

## 3. NOT VERIFIED / 不在范围

- 名称/颜色编辑 UI、贴图/材质库、拖放涂刷；
- 真实 GUI/GPU 交互与其它载体运行时验收。
