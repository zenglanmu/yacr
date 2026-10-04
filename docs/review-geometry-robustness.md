# 几何数值正确性审查（2026-10-04）

本轮仅修改 `crates/cad-geometry`，依据需求 v2.0 §16.1 的 f64 数值与工作平面约束；没有修改源坐标、其他 crate、共享交接文档或引入依赖。

## 缺陷与修复

1. **向量长度的中间平方溢出/下溢**：原 `sqrt(dot(a, a))` 会把长度仍可表示的 `1e200` 量级向量变成无穷，把 `1e-200` 量级向量变成零。改用 `hypot`。归一化直接逐分量除以长度；有限分量但真实长度超过 f64 上限时，先按最大分量缩放再归一化，避免返回零向量。
2. **平移后多边形面积丢失**：原鞋带公式直接相减绝对坐标乘积；位于 `(1e12, 1e12)` 的 `3 × 2` 矩形会丢失真实面积。改为在公式内部使用首点相对差，输入数据不变，保持 XY 投影、有符号绕序及现有错误语义。
3. **工作平面正交检查被 NaN 绕过**：原 `abs(dot(u,v))/(length(u)*length(v))` 对大有限基向量得到 `inf/inf = NaN`，比较为假而接受平行基。改为单位向量点积，并明确拒绝非有限正交结果；大正交基仍可接受。

## 回归契约

新增 `crates/cad-geometry/tests/numeric_robustness.rs`：

- `vector_length_avoids_intermediate_overflow_and_underflow`：长度与距离在 `1e200` / `1e-200` 下保持相对精度。
- `normalization_preserves_direction_for_large_finite_vectors`：大向量方向正确；两个 `f64::MAX` 分量仍产生有限单位向量。
- `translated_polygon_preserves_area_and_winding`：大平移矩形面积为 6，反转绕序有符号面积为 -6，测量仍为 6，原输入不变。
- `large_work_plane_basis_does_not_bypass_orthogonality_validation`：`1e200` 与 `f64::MAX` 量级平行基拒绝、正交基接受。

## 验证状态与边界

**NOT RUN**：根据主代理分工要求，本子代理未运行编译、测试、检查或格式化器，未提交或推送；上述均为新增合成契约，尚无通过证据。由主代理统一格式化、运行 `cargo test -p cad-geometry --locked` 与完整门禁。

这不是通用稳健谓词实现：面积仍为 XY 投影，无法恢复输入 f64 已丢失的坐标差，极大局部跨度仍可能溢出；面积的既有共面与退化容差未调整。归一化保留原有 `< 1e-24` 退化处理。其他模块的原始点积、叉积及直接平方计算未在本轮全面替换，真实 DWG、真实 GPU 和真机能力均未验证。
