# 第三方来源与许可

- acadrust：正式 0.5.5，唯一依赖入口 cad-import-acadrust；未修改上游，未使用 Cargo patch。
  当前导入仍占位；采用 API 与对外分发前审计锁定包许可及其传递依赖。
- Slint/wgpu：技术方向保留，尚未实际接线/锁定兼容组合，不能宣称集成完成。
- OpenCADStudio：仅功能规格与 UI 交互参考，见 `docs/ui-requirements/00-INDEX.md`；不作为源码采用来源。此前调查记录保留在 `docs/migration-map.md`，其中记录未复制代码；不构成新的源码/资源复制授权。
- glam/serde/serde_json/thiserror 等实际依赖见 Cargo.lock；依赖许可证需建立分发审计。
- QCAD `flange` 测试样本：`fixtures/dxf/qcad-flange/{flange.dxf,flange.png,flange.pdf}`，
  来自 <https://github.com/qcad/qcad> 的 `examples/`（`master` `dcf5754b`，2026-10-03 经
  操作者代理下载；字节数与 SHA-256 见同目录 `SOURCE.md`）。QCAD `LICENSE.txt` 声明 QCAD 3
  源码为 GPLv3（附加例外）、图标与文档为 CC BY 3.0；`examples/` 无逐文件声明，按上述条款
  使用并保留本署名。仅作渲染回归输入与人工参考，不表示与 QCAD/RibbonSoft 存在关联或背书。
- 未捆绑用户图纸、厂商插件或专有字体；已捆绑的开源 QCAD 样本来源与许可独立核对，
  不由主仓库许可推断。
- `fonts/osifont.ttf`：QCAD（<https://github.com/qcad/qcad>）`fonts/osifont.ttf` 的副本，
  原始字体项目 osifont（<https://github.com/hikikomori82/osifont>），GNU GPL v3 附字体例外。
  下载日期 2026-10-09 经 jsDelivr；来源 URL、字节数与 SHA-256 见 `fonts/SOURCE.md`。
  本仓库为 AGPL-3，可与 GPL 组件组合；仅作为 CAD 文字默认轮廓回退面随发布包分发。
