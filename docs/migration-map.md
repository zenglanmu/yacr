# OpenCADStudio 迁移映射

来源：https://github.com/HakanSeven12/OpenCADStudio 。尚未锁定 commit、审计许可或抽取任何代码。
文档候选路径不能作为实际 API 证明。

| 候选来源 | 目标 | 采用前证据 |
|---|---|---|
| scene/convert/tess.rs、tessellate.rs | kernel-adapter/representation | commit、真实调用链、最小输入输出、Android/Wasm |
| entities/curve.rs、scene/convert/curve_tol.rs | geometry | 许可、容差语义、大坐标/非均匀变换 |
| 相机/投影 | app::Camera/Viewport | 去除 Iced、2D/3D 对照测试 |
| shader/GPU | render-wgpu | 同设备组合、wgpu 主版本、WebGL2 路径 |
| 字体/标注 | resources/representation | 资源许可、CAD 样式/排版一致性 |
| 缓存/性能 | scene | 移动内存基准、失效规则 |

每项采用后记录 commit/文件/函数 → 目标文件 → 依赖适配 → 保留行为 → 差异 → 测试。
不迁移 Iced UI 或完整建模依赖树。mlightcad 仅交互组织对照，不作为渲染核心来源。
