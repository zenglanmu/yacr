# 第三方来源与许可

- acadrust：正式 0.6.8，唯一依赖入口 cad-import-acadrust；未修改上游，未使用 Cargo patch。
  上游 <https://github.com/hakanaktt/acadrust>。DWG/DXF 导入与实验性 DWG 导出都经该适配层；
  采用 API 与对外分发前审计锁定包许可及其传递依赖。
- MiniCAD：轻量开源 2D/3D CAD，仅作为功能与实现参考；未复制其源码，也不是构建或运行时依赖。
- wgpu / Tauri / GTK：桌面 UI 与渲染技术方向。wgpu 仅由 `cad-render-wgpu` 依赖；桌面宿主使用
  Tauri 2（WebView）+ GTK3/WebKitGTK 4.1 + 原生 GTK/wgpu CAD 分区。Slint 依赖已随桌面 UI 迁移
  退役（见 `docs/adr/0010-react-linux-host-replaces-slint.md`）。具体版本与传递依赖见 Cargo.lock；
  依赖许可证需建立分发审计。
- OpenCADStudio：历史上曾作为功能规格与 UI 交互参考；该参考练习已于 2026-10-15 结束，本地参考目录 `docs/ui-requirements/` 已移除。其不作为源码采用来源，未复制其代码；此前调查记录保留在 `docs/migration-map.md`，其中记录未复制代码；不构成新的源码/资源复制授权。
- glam/serde/serde_json/thiserror 等实际依赖见 Cargo.lock；依赖许可证需建立分发审计。
- QCAD `flange` 测试样本：`fixtures/dxf/qcad-flange/{flange.dxf,flange.png,flange.pdf}`，
  来自 <https://github.com/qcad/qcad> 的 `examples/`（`master` `dcf5754b`，2026-10-03 经
  操作者代理下载；字节数与 SHA-256 见同目录 `SOURCE.md`）。QCAD `LICENSE.txt` 声明 QCAD 3
  源码为 GPLv3（附加例外）、图标与文档为 CC BY 3.0；`examples/` 无逐文件声明，按上述条款
  使用并保留本署名。仅作渲染回归输入与人工参考，不表示与 QCAD/RibbonSoft 存在关联或背书。
- `fonts/NotoSansCJKsc-Regular.otf`：Noto Sans CJK SC Regular，来自
  <https://github.com/notofonts/noto-cjk> 固定提交
  `f8d157532fbfaeda587e826d4cd5b21a49186f7c` 的
  `Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf`，原样分发、未修改。
  字节数 16437364，SHA-256
  `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`，
  SIL Open Font License 1.1（文本见 `fonts/NotoSansCJK-OFL.txt`），版权行
  `Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'.`。
  未修改即无需按 RFN 改名；仅作为缺字时的默认 CJK 回退轮廓面随发布包分发。
- 未捆绑用户图纸、厂商插件或专有字体；已捆绑的开源 QCAD 样本来源与许可独立核对，
  不由主仓库许可推断。
- `fonts/osifont.ttf`：QCAD（<https://github.com/qcad/qcad>）`fonts/osifont.ttf` 的副本，
  原始字体项目 osifont（<https://github.com/hikikomori82/osifont>），GNU GPL v3 附字体例外。
  下载日期 2026-10-09 经 jsDelivr；来源 URL、字节数与 SHA-256 见 `fonts/SOURCE.md`。
  本仓库为 AGPL-3，可与 GPL 组件组合；仅作为 CAD 文字默认轮廓回退面随发布包分发。
