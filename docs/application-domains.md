# 应用域与界面 profile

> 决策见 [ADR-0053](adr/0053-application-domains.md)。本文件记录**当前实现状态**与证据边界；
> 平台/载体结论以 `docs/compatibility.md` 与 AGENTS 平台表为准。

## 当前状态（2026-11-24，阶段 1）

| 能力 | 状态 | 证据 |
|---|---|---|
| 文档域字段 `application_domain`（minor 13，缺省不写） | 实现 | `cad-domain` header 契约、`cad-db` snapshot 往返、`cad-store-zip` minor 选择 |
| 导入推导（dwg/dxf/dgn→cad2d，ifc/rvt→bim，skp→concept，step→mechanical，其余不声明） | 实现 | `cad-app` import publish；无真实文件运行证据 |
| 用户写入 `cad_set_application_domain { domain }` | 实现（shared sink） | 共享 gate 契约 + `cad-app` header/脏标记契约；载体运行 **NOT VERIFIED** |
| `cad_status.applicationDomain` | 实现（无文档 `null`） | `cad-session` status 投影 |
| 前端 profile（4 域，token 解析，预览 store） | 实现 | `apps/desktop-ui/test/domains.test.ts` |
| 右导航栏切换器 + `<html data-domain>` 主题切换 | 实现 | 组件 + CSS token 契约；浏览器/真机视觉效果 **NOT RUN** |
| `cad2d` 参考结构 | 实现：文件/常用/插入/注释/参数化/视图/管理/输出/附加模块；Home/Insert/Annotate/View/Manage/Output 按参考面板顺序，未接线面板显式 pending | `test/autocad-structure.test.ts` + `test/fixtures/autocad-2024-drafting.json`；视觉 **NOT RUN** |
| BIM/SketchUp/SolidWorks | 显式占位：隐藏功能区与命令行，显示说明卡 | `domains.ts` profile 标记 + 组件契约 |
| 每域主题持久化 `ui.theme.<domain>` | **未实现**（阶段 2） | — |
| `DOMAIN` 命令 / 文档属性入口 | **未实现**（阶段 2；解析函数已就绪并有测试） | `domainFromArgument` |
| 官方手册结构 fixture | 实现（结构转录；文件内注明 File 为扩展 tab） | 同上 |
| golden 截图 lane | **未实现/未运行**（阶段 2） | — |

## 不变式

1. **域是文档属性**：切换写 `document.json`（一个 revision、`ChangeScope.header`），不转换数据。
2. **profile 只决定表现**：命令可用性仍由文档能力 ∩ 构建能力决定；缺能力 disabled + 稳定
   reason，不隐藏、不空成功。
3. **占位不撒谎**：占位域不显示 AutoCAD 功能区/命令行，不摆可点击的假按钮；文档数据与文件
   命令仍是真实路径。
4. **中性 token**：文件名与命令只使用 `cad2d|bim|concept|mechanical`；参考软件名只出现在
   tooltip/内部文档。
5. **无文档只预览**：欢迎页切换不写盘、不改 `NEW` 默认。

## 阶段 2 计划（未开始）

- 自有 golden 截图 lane（headless Chromium + SwiftShader；结构 fixture 已就位）。
- 每域默认停靠布局（BIM：属性+项目浏览器；concept：左工具栏+右托盘；mechanical：
  CommandManager+FeatureManager），隐藏是默认值、会话内可重开。
- `DOMAIN` 命令（裸命令开切换器、`DOMAIN <id>` 直接设置）与文档属性行。
- `ui.theme.<domain>` 每域主题设置；渲染器画布背景按域预设（当前 `theme` 仍只管 dark/light）。
- golden lane：headless Chromium + SwiftShader，固定视口/DPR/字体，标记 layout-only。
  官方手册截图不入库，只留来源清单与采样色值。

## 验证命令

```bash
cargo test -p cad-domain -p cad-db -p cad-app -p cad-ui-core --lib --locked
cargo check -p app-web --target wasm32-unknown-unknown --locked
(cd apps/desktop-ui && npm test)
python3 scripts/check-i18n.py && python3 scripts/check-file-size.py
```

全量门禁与真机/真实 GPU/截图对比：**NOT RUN**。
