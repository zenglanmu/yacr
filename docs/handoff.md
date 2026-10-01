# 后续 agent 接手入口

1. 阅读规范、AGENTS.md、requirements.md、architecture.md 与最近 Git diff。
   此次恢复中发现并发实现与提交，结束时可能仍有其他实现工作；本文描述的是框架交接，
   请以当前源码和最新构建证据确认状态，不覆盖陌生变更。
2. 运行 README 的 check/test/architecture 命令；不要删除陌生的已有实现。
3. 用 `rg 'pending\(' crates apps` 查找显式占位；文件路径与字符串标识对应实施单元。
4. Android 的 Slint + wgpu 组合已验证到打包层：见 ADR 0002
   （docs/adr/0002-gpu-ui-composition.md）与 docs/validation.md；
   `scripts/build-android.sh --release` 产出并签名 aarch64 APK。
   真机/模拟器运行仍未执行，不能以打包成功代替。Web 组合仍未完成
   （`host.web.file_api_canvas_composition`），不能以本机 Rust 编译代替。
5. 逐项实现 Builder/事务/历史 → 导入/基础语义 → 显示/索引/场景 → 应用/工具 → 宿主闭环；
   不把这个依赖顺序当作排期，也不绕过优先 GPU/UI 验证。
6. 每次移除占位同步补齐规范测试、能力表、构建说明、Git 提交。

## 本轮状态（compact，2026-10-01）

已完成并推送（main）：真实 DWG 端到端证据（`docs/validation.md`）、审计 B15（INSERT 展开、
块定义入 `SpaceId::Block`）、B20（能力/完整性按可绘性判定）、字体目录与 CDN 来源
（`docs/fonts.md`）、文字整形（sfnt/WOFF1 + SHX shapes/unifont/bigfont，缺失字体走回退
链）、TEXT/MTEXT 对齐、TTF kerning、主机接缝（`plan_fonts`、`FontLoader`、
`CadView::set_fonts`）、DIMENSION（展开匿名块）与 HATCH（边界/实心/图案）。核心测试
146 passed / 0 failed；Wasm/Android target、fmt、clippy、架构脚本通过；字体相关测试可用
`YACR_TEST_FONT`/`YACR_TEST_SHX` 指向真实字体。

仍开放：MultiLeader/ACIS/3D 渲染；Web/Android 实际取字体与真机/浏览器运行；复杂文字整形
与 MTEXT 高级排版；路线图 C 段交互（U01–U12、F01–F12）、D 段（F13–F15）、N01 双语、
N02 CI 完整层次。`fixtures/manifest` 仍为空（未授权样本不入库）。

## 已定义，但尚需设计审查

- RenderTarget/HostTexture 当前仅为自有数字 token；共享 Device/Queue 的拥有者、
  token 生命周期与安全访问方式须在平台合成 ADR 中固定，不可凭 token 猜测 GPU 对象。
- AnnotationId(u128) 只表达 UUID 位宽；生成/碰撞防护由宿主服务和数据库校验完成。
  JSON UUID 编码、时间格式、extension 原始 JSON 校验尚未实现。
- FingerprintPolicy 的显式策略不意味着导入已经能自动建立锚点；fingerprint 计算需分段/异步。
- CommandPayload 的类型校验、防止 ID 与 payload 不匹配、事务权限与撤销能力仍需接线。
  当前 Application 提前检查模式与文档身份，但 Work 模式执行仍返回 NotImplemented。
- RepresentationProvider 的字体/样式上下文目前只提供最小配置，需要扩充自有只读 views。
- DB 中已有基础实现应保留并复查几何边界/块实例/验证完整性，不代表工业语义已验收。
- 数据库/几何已有基础真实实现与合成测试，代理也已有有界 framing/记录回放代码。
  这些是在恢复过程中保留的新增实现；需审查实际支持范围，不能据此宣称真实 DWG/工业兼容。
- 扩展点包含 Importer/Tool/CommandHandler 等；注册/优先级/冲突策略需统一实现。
- PropertyProvider、字体 shaping、缓存容量、Worker 二进制传输、Journal codec、schema migrations
  都需要完整接线和验收，不允许以现有结构体宣称闭环。

## 接线验收

业务不调用 draw；批注确认一次事务，取消零事务；UI 回填不触发重复命令；
revision 不连续重建；GPU 重建不改数据库；设备失败和退出保护未保存批注；
局部失败保留对象级报告；未知源单位显示图纸单位。Android/Web 真机与浏览器分开记录。
