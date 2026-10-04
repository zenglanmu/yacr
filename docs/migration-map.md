# OpenCADStudio 历史调查记录（非源码迁移要求）

## 当前契约边界（2026-10-04）

用户已明确 OpenCADStudio 仅为功能规格参考，入口为 `ui-requirements/00-INDEX.md`。
以下记录保留此前源码调查事实，不表示当前允许或要求源码参考、抽取、移植或依赖采用。
下文“待办”与“采用”均为历史计划，已由 `CAD_IMPLEMENTATION_SPEC.md` §10 的独立实现与
功能对照要求替代，不作为后续实现任务或交付门禁。历史许可证风险不因此被抹除。

来源：https://github.com/HakanSeven12/OpenCADStudio

锁定 commit：`02c470aac7af1f912349ee7fd5ad612863e8b7da`（2026-10-02 由
`git ls-remote` 取得）。**工作副本现位于 `/home/zenglanmu/sources/opencadstudio`**
（经代理 `192.168.8.1:10809` 取的 tarball；**不入库**）。该副本是较新的上游主干
（`2026.39.0`，`src/` + `crates/` 布局），与锁定的 `02c470aa` 布局不同；引用具体文件
时必须核对实际存在的路径，文档路径不作为 API 证明。若需精确对齐锁定 commit，需另行
按该 SHA 取归档。
上游许可：**GPL-3.0**（`LICENSE`）。本项目 `LICENSE` 为 **AGPL-3.0**；GPL-3.0 代码可
并入 AGPL-3.0 项目，但 `Cargo.toml` 的 `license = "Apache-2.0 OR MIT"` 字段与 LICENSE
不一致，须在采用任何上游代码前修正（见文末“许可风险”）。

候选路径均以锁定 commit 的实际文件为准，文档路径不作为 API 证明。

## 本轮采用的参考（仅行为/边界参考，未逐字复制）

阅读上游源码以确认功能范围与边界条件，代码在本仓库独立重写：

| 上游文件 | 本项目落点 | 参考内容 | 采用方式 |
|---|---|---|---|
| `src/entities/curve.rs` | cad-geometry 曲线/NURBS | 曲线离散与容差语义、非均匀变换 | 行为参考 |
| `src/scene/convert/curve_tol.rs` | cad-geometry 离散容差 | 弦高/角度伺服 | 行为参考 |
| `src/scene/convert/solid3d_tess.rs` | cad-kernel-adapter | 3DSOLID 面离散边界 | 行为参考 |
| `src/scene/convert/acis_kernel.rs` | cad-import-acadrust ACIS 数据路径 | ACIS 载荷与坐标 | 行为参考 |
| `docs/cadkernel-body-path.md` | cad-kernel-adapter | body/face 路径与降级 | 行为参考 |

未移植 Iced UI、opencadcodec、opencadkernel 依赖树，也未修改 acadrust。

## 待办（采用更多代码前必须完成）

| 候选来源 | 目标 | 采用前证据 |
|---|---|---|
| scene/convert/tess.rs、tessellate.rs | kernel-adapter/representation | commit、真实调用链、最小输入输出 |
| 相机/投影 | app::Camera/Viewport | 去除 Iced、2D/3D 对照测试 |
| shader/GPU | render-wgpu | 同设备组合、wgpu 主版本、WebGL2 路径 |
| 字体/标注 | resources/representation | 资源许可、CAD 样式/排版一致性 |
| 缓存/性能 | scene | 移动内存基准、失效规则 |

每项采用后记录 commit/文件/函数 → 目标文件 → 依赖适配 → 保留行为 → 差异 → 测试。

## 许可风险（须维护者确认）

- `LICENSE`（AGPL-3.0）与 `Cargo.toml` `license = "Apache-2.0 OR MIT"` 冲突。
- 若采用 GPL-3.0 上游代码，必须统一为 AGPL-3.0（或去掉冲突声明），并在
  `THIRD_PARTY_NOTICES.md` 记录文件、版权与许可。
- 当前各 workstream 以上游为**行为参考、独立实现**，未复制源码；即便如此，
  许可证一致性仍应在发布前解决。
