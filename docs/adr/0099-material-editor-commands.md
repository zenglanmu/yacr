# ADR-0099：材质编辑命令（MATERIAL RENAME/COLOR/TRANSPARENCY/DELETE）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，命令层 + 面板 Delete；无真实 GUI 证据）**
- 关联：ADR-0062（材质表）、ADR-0079（文档材质行）、ADR-0081（逐面材质）

## 背景

材质表有创建（首次 `PAINT` 内置色）、赋值与逐面涂刷，但没有改名/改色/改透明度/删除入口；覆盖表
把“材质编辑器”列为未建模。命令层缺少这些行编辑，UI 也无法提供。

## 决策

- 新增命令 `MATERIAL`（命令数 264），参数：
  - `<name> RENAME <new>`：改名，保留 sequence 与全部实体/面赋值；重名/非法名按域错误拒绝。
  - `<name> COLOR <r> <g> <b>`：设置 sRGB；r/g/b 是 0..=255 的十进制。
  - `<name> TRANSPARENCY <0..=1>`：设置透明度。
  - `<name> DELETE`：删除材质行并清除所有实体/面赋值（不删实体）。
  - 名称含空格时用双引号（与 group grammar 相同；无转义）；未知材质 `material.unknown`、无变化
    `material.no_change`。
- 每次编辑是一次 `tables/materials.json` 整表写入/一步撤销（复用既有 MaterialPatch 与历史路径）；
  Work-only（Viewer `PermissionDenied`）。
- UI：文档材质行保留色块重涂，并增加 **Delete**（`MATERIAL "<name>" DELETE`，名称含 `"`/换行时
  禁用）；改名/改色/透明度的表单 UI 未建模，命令层已可用。

## 不在本 ADR 范围

- 名称/颜色编辑表单、材质缩略图、拖放涂刷；
- 贴图/纹理材质、材质库、按材质选择；
- 真实 GUI/GPU 与其它载体运行时。

## 后果

- 材质“编辑器”的命令层缺口关闭（改名/改色/透明度/删除），面板可用于删除；Materials 仍为
  partial（编辑表单/贴图/库未建模）。
- 无格式档位变化：仍是 `tables/materials.json`（minor 19/25）。
