# 独立分支并行审查（2026-10-04）

本轮由用户要求增至 4 个子代理，在主控已验证并推送的 `2d6b9ec` 基线上
分别使用独立分支/worktree。子代理只写代码与回归测试，不编译、格式化、测试或推送；
主控负责审查、整合分支、执行统一门禁及最终提交/推送。

| 分支 | 审查边界 |
|---|---|
| `review/20261004-history` | 合并 patch 的无变化消去与输入契约 |
| `review/20261004-dependencies` | revision 流连续性与有界依赖失效 |
| `review/20261004-mapping` | 批注侧车仿射映射的方向向量与验证 |
| `review/20261004-resource-limits` | 图片像素计数整数溢出 |

## 已审查的交付

资源分支改用 checked multiplication，像素计数溢出时返回
`resource.size_overflow`，即使预算是 `u64::MAX` 也不接受饱和后的假计数。
诊断的 `actual=u64::MAX` 是下界而不是精确值；合法零维度、精确边界和
可表示的普通超限行为保留。主控同步补齐诊断 schema 常量、双语目录和
“所有稳定代码均可本地化”的契约列表（203 个目录键）。

依赖分支改为精确检查 ChangeSet 前后 revision：合法空提交仅在当前 revision
停留；跳跃、回退、旧空提交、数据库切换和不能前进的最大 revision 均明确
请求 snapshot 重建。受影响数量包含去重后的根；只有发现超限的新消费者
才触发数量/深度预算，边界叶子/已访问环不误报，超限不发布部分集合。
`clear()` 保留原有仅清图边、不重置订阅 revision 的语义并补契约说明。

历史分支在合并后消去 before/after 相同的 patch，全部消去时只移除该条记录，
保留更早绘图/批注历史；新提交仍清空 redo，预算计数重算。进入历史前拒绝
重复 ID、快照身份不匹配及两侧均不存在的 patch，不修改原栈。没有追加数据库
身份绑定或相邻快照连续性验证。

映射分支用仿射矩阵线性部分变换方向，不再从两个带巨大平移的点相减，
避免小轴向量消失；拒绝非规范仿射齐次行以及非有限 determinant。
保留原有 determinant 阈值 `1e-12`。点坐标仍受 f64 平移精度限制，
映射后测量数值重算不在本次修复范围内，不能声称任意仿射测量记录保真。

## 当前证据

资源分支新增 4 项、依赖分支新增 15 项测试，子代理**未运行**；主控已审查 diff，
并在各自 worktree 实际执行单 crate 测试：resources **21 passed / 0 failed**、
dependencies **20 passed / 0 failed**，复用主控 target 目录，不推广为完整门禁通过。
历史/映射各新增 6 项，主控 worktree 测试 history **23 passed / 0 failed**、
annotations **24 单元 + 15 codec 契约 passed / 0 failed**。
主控 i18n 静态门禁、稳定诊断双语描述契约与 Node 六套模块契约（53/53）通过。
四个开发分支（b1061cb、19a58b3、baa6763、0a0d764）已由主控无冲突整合，
统一代码树主控完整门禁实际通过：

- fmt、严格主机 workspace clippy（排除 Android/Web）、架构、fixture manifest、
  workflow 和 i18n（203 keys）检查。
- 主机 workspace 串行测试（包含 Linux App/Slint、排除 Android/Web）：
  **1229 passed / 0 failed / 1 ignored**；输出
  `/home/zenglanmu/.local/share/opencode/shell/da739b3910d63162f4687d9e8c13d613a0959f68/sh_104a41cfd001LZx2Hz8Om3OJT2.out`。
- wasm32 全 workspace lib 编译检查（包含 Android/Web 宿主）。
- Linux release 编译及实际 lavapipe 离屏 smoke：
  `/tmp/opencode/yacr-linux-20261004-020914-462863`，2 CAD frames、导航相机/像素均变化，
  `renderError=null`。输入为合成图纸，视觉正确性仍需人工参考图验收。

开发分支交付不等于各分支独立完整门禁通过；上述结果针对合并后的代码树。
真实 DWG 条件测试未提供输入，跳过不是已执行证据；真实 GPU、桌面窗口及真机未运行。
