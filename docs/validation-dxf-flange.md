# QCAD `flange` DXF 离屏渲染验证（2026-10-03）

本轮按用户要求把 QCAD 的 `flange.dxf` / `flange.png` / `flange.pdf` 作为渲染回归
样本提交进仓库，记录来源，并补测试代码后跑 Linux 原生离屏渲染。通用步骤见
[testing-dwg.md](testing-dwg.md)；本文只记录本轮实际执行的证据。

## 1. 样本与来源

提交位置 `fixtures/dxf/qcad-flange/`，来源与 SHA-256 详见同目录
[SOURCE.md](../fixtures/dxf/qcad-flange/SOURCE.md)，并登记在 `fixtures/manifest`。

| 文件 | 字节 | SHA-256 | 说明 |
|---|---|---|---|
| `flange.dxf` | 287223 | `df469e7e91e38c901fced51c9621c8d2b6b6568678803e563b29c9ecf8dec84b` | AC1027 ASCII DXF，被测图纸 |
| `flange.png` | 95087 | `26d730aa0a758adbe9cab8e2b05a17266a715649442c49452f53f5098e3cea36` | QCAD 上游 1024×768 参考图，仅人工参考 |
| `flange.pdf` | 199962 | `b33726c83edf31710268b9b143574d91520770d2fe704ffbb00ced468c6f830b` | QCAD 上游单页矢量参考 |

来源：<https://github.com/qcad/qcad> `examples/`，`master` `dcf5754b0a19d8e57eddd467d18bf802ac12c2e2`，
2026-10-03 经代理 `http://192.168.8.1:10809` 下载，字节未改写。许可见 SOURCE.md
（QCAD LICENSE.txt：源码 GPLv3 附加例外，图标/文档 CC BY 3.0；`examples/` 无逐文件声明）。

## 2. 测试代码

- `scripts/check-dxf-reference.py`（仅标准库）：依次运行 `scan` →
  `build-representation` → `render`，断言导入/表示/帧 smoke 与适配器类型，解码渲染
  PNG 与参考 PNG，输出 `summary.json` 及 `review-side-by-side.png` /
  `review-overlay.png` 供人工对照。它**不**断言 SSIM/IoU 等保真分数。
- `crates/cad-cli-tools/tests/dxf_fixture.rs`（无需 GPU，进入默认 `cargo test`）：固定
  已提交样本仍可导入、报告毫米与 `Partial`、表示非空。
- DXF 的实际渲染仍需 GPU，脚本保持独立、可重复运行，不把它塞进无 GPU 的 CI job。

## 3. 实际执行

环境：`target/release/cad-cli-tools`；`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`；
真实 wgpu/Vulkan，适配器 **CPU / llvmpipe**，Mesa `26.0.8-1ubuntu0.3`（LLVM `21.1.8`）。
无 X11/Wayland、无 Slint、无浏览器。

```bash
cargo build -p cad-cli-tools --release --locked
python3 scripts/check-dxf-reference.py --out /tmp/opencode/yacr-dxf-reference
cargo test -p cad-cli-tools --test dxf_fixture --locked
```

结果（`summary.json`）：

| 阶段 | 实测 | 墙钟 |
|---|---|---|
| `scan` | 419 entities，223 model entities，13 layers，Millimeter，`status=partial`（`no display representation for: AcDbDimension, AcDbMText`） | 0.171 s |
| `build-representation` | 298 primitives，1060 vertices，297 lines + 1 text，0 failures | 0.173 s |
| `render` 1024×768 | 12226 非背景像素（1.55%），4 色，297 draws / 1060 vertices，PNG 18202 B | 0.478 s |
| `cargo test dxf_fixture` | **2 passed / 0 failed** | 0.20 s |

退出码 0、非空 PNG、`error=None` 只说明出图 smoke 通过，不单独证明视觉正确。

## 4. 三层结论

1. **打开通过**：真实导入该 DXF，实体数/空间/单位合理；`Partial` 明确列出
   `AcDbDimension`、`AcDbMText` 无显示表示。
2. **出图 smoke 通过**：真实 lavapipe 适配器提交并回读出非空帧（1.55% 覆盖）。
3. **视觉验收：部分通过（几何），文字/标注未通过**。渲染图（`flange.png`）与 QCAD
   参考图对照：四个视图（主视图、剖视、俯视、轴测）、中心线、剖面线、尺寸界线轮廓的
   位置与参考一致；但**尺寸、标注与标题栏文字缺失**，因为对应实体仍是 `Partial`。
   参考图另有图框/标题栏，渲染图没有，且两者背景、视口边距与线宽不同，因此粗粒度
   64×48 IoU 仅 0.155、渲染前景 37.9% 落在参考前景内——这些数字只作定位，
   **不是保真度评分**。

人工对照图：`/tmp/opencode/yacr-dxf-reference/review-side-by-side.png`（左参考、右渲染）、
`review-overlay.png`（参考=红、渲染=蓝、重合=黑）。

## 5. 未完成 / 不宣称

- 未验证尺寸/标注/标题栏文字绘制：本图对应能力仍 `Unsupported`/`Partial`，本轮只
  记录缺项，未改成空成功。
- 未验证跨 GPU/后端像素一致、真实 GPU、WebGPU、Android；100% 软件渲染。
- 单样本 smoke 不等于 DXF 兼容性；`check-dxf-reference.py` 的参考对照是人工辅助，
  不是授权黄金图矩阵。
- 仓库其它历史验证文档按当时事实保留（那时样本未入库），只更新了当前政策类文字。
