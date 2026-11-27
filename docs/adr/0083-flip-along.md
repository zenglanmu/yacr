# ADR-0083：Flip Along（FLIP X|Y|Z）

- 状态：**已接受（决策，2026-11-28）**；**已实现（2026-11-28，原地轴反射；无真实 GUI/GPU 证据）**
- 关联：`MIRROR`（CAD 保留原件）、选择包围盒、`mirror_selection_with_options`

## 背景

覆盖表把 Mirror/Flip 记为 “CAD 镜像，不是 Flip”。SketchUp 的 Flip Along 是**原地**反射：
选中对象绕自身包围盒中心、垂直于红/绿/蓝轴的平面镜像，源对象被替换而不是保留副本。

本仓库已有完整的镜像实现（任意 `Transform3`、源删除选项、文字字形真反射、一次事务/一步撤销），
缺的只是“以选择包围盒中心为反射平面”的命令入口与轴选择语法。

## 决策

### D1 命令与语法

- 新选择子 `FLIP`，参数只接受一个轴：`X|RED`、`Y|GREEN`、`Z|BLUE`（大小写不敏感）。
  裸 `FLIP` 或其它文本按 `flip.axis_required` 拒绝，不猜默认轴。
- `FLIP` 是 Work-only：Viewer 返回 `PermissionDenied`，且在写任何东西之前拒绝。
- 空选择 `flip.selection_required`；子图元/块成员沿用既有 `FLIP` 目标解析规则（按名拒绝或按
  整实体处理，取决于镜像实现已有契约）。

### D2 语义

- 反射平面：`axis = bounds centre[axis]`，即对角矩阵在该轴取 `-1`、平移 `2 * centre[axis]`；
  对每个选中实体调用既有 `mirror_selection_with_options(transform, mirrortext=true,
  erase_source=true)`，因此一次事务、一步撤销，文字/MTEXT 字形真反射。
- 选择包围盒来自既有 `selected_object_bounds`（模型/图纸空间、块成员展开、子图元拒绝规则与
  `ZOOM OBJECT` 相同）。
- 成功后在**新实体行**上重建选择，连续 `FLIP` 仍作用于同一对象；undo 后选择清空（旧 id 已删除，
  不伪造新 id）。

### D3 边界

- 副本是**新 entity id**：group/component 的“同一对象身份”不保留（Outliner/属性历史会看到新行）。
- 块定义成员/子图元按既有镜像规则拒绝；不新增块级原地身份改写。
- 无格式档位变化、无新表、无前端按钮/i18n 文案；`MIRROR` 的 CAD 语义不变。

## 后果

- Mirror/Flip 行从“没有 Flip”变为 “MIRROR + FLIP”，仍标记 **partial**（对象身份不保留），
  但 `FLIP X|Y|Z` 已真实可用。
- 命令总数 260 → 261；其它命令表/语法/契约测试同步。
