# 批注边车文件与批注管理策略（F07/F08/F09，核心层）

本文只覆盖 `cad-annotations` 核心层实现的契约：边车（sidecar）JSON 编解码、
指纹/映射策略、批注命令与事务、导出原子性。UI/编辑器/宿主 I/O 不在本层，
相关缺口在文末标 OPEN。

权威需求：`CAD_IMPLEMENTATION_SPEC.md` §3.4、§16.3；追踪：`docs/requirements.md`
F07/F08/F09；审计：`docs/code-audit-and-agent-handoff.md` B09/B10/B11/B12、
`docs/core-invariants.md` B12。

## 1. 格式（版本化边车）

批注保存在独立于 DWG 的版本化 JSON 中，`SCHEMA_VERSION` 当前为 1。

顶层字段（`KNOWN_TOP_LEVEL`）：

| 字段 | 类型 | 说明 |
|---|---|---|
| `schema_version` | 正整数 | 必填。0 非法；大于本构建支持版本 → `Unsupported`。 |
| `application_version` | 字符串 | 写入方版本，缺失视为空串。 |
| `document_fingerprint` | UUID 字符串或 32 字节数组 | 必填。数组必须是 32 个 0..=255 整数；字符串必须是合法 UUID；缺失/畸形 → `CorruptData`，绝不回退为 `Temporary(0)`。 |
| `document_name_hint` | 字符串 | 文件名提示，缺失视为空串。 |
| `unit_context` | 对象 | `source`/`display`/`display_per_source`/`decimal_places`。 |
| `annotations` | 数组 | 必填，可为空数组。 |
| `view_bookmarks` | 数组 | 可选；非数组 → `CorruptData`。 |

未知的顶层字段按原样保留在 `AnnotationFile::extensions_json`
（parse→serialize→parse 结构一致），使新写入方的数据能被旧读取方存活。

**未知字段策略**：未知顶层字段保留；未知/未知枚举值（几何 `kind`、算法、
来源、精度、单位、空间、锚点状态、指纹类型）一律 `CorruptData`，不做近似
或默认；未知注解内部子字段当前不保留（见 OPEN）。

### 1.1 丢弃/损毁防护（审计 B09/B11）

旧实现存在多处静默强制转换，均已改为显式报错：

- `decimal_places` 不再用 `as u8` 截断，超 8 位 → `CorruptData`。
- `display_per_source` 若出现必须有限。
- 未知单位字符串不再回退 `DrawingUnits`；**缺失**单位仍按文档约定视为
  Unknown（`DrawingUnits`）。
- 畸形空间对象不再变成 `Paper(0)`/`Model`；`Paper`/`Block` id 必须可解析。
- 缺失的锚点实例路径不再变成空路径。
- 缺失/畸形的 `text`、`space`、`precision`、`style` 字段不再取默认值。
- 重复注解 id 被拒绝，避免导入时静默合并。
- `encode` 不再忽略 `file.schema_version`：声称更新版本 → `Unsupported`。
- `extensions_json` 中与 schema 字段冲突的键不再被静默丢弃 → `InvalidInput`。

## 2. 指纹与映射策略（审计 B10）

`FingerprintPolicy`：

| 策略 | 指纹不匹配时的行为 |
|---|---|
| `RejectMismatch` | 返回 `InvalidInput`，不产生任何内容。默认。 |
| `ImportUnanchored` | 接受不匹配；清空全部锚点（`anchor = None`），返回的文件重定绑定到当前图纸身份。 |
| `ExplicitCoordinateMapping(Transform3)` | 校验映射（非有限、奇异 → `InvalidInput`），对全部几何、锚点回退点、测量工作平面、书签相机变换施加映射；锚点标 `Unresolved`（未经验证的绑定不得声称 `Valid`）；返回的文件重定绑定到当前图纸身份。 |

确定性：同一输入 + 同一策略总是得到相同结果；不匹配永不被静默忽略。

可诊断：`AnnotationService::decode_with_report` 返回 `DecodeOutcome`，其中
`FingerprintReport` 记录 `file_fingerprint`、`open_fingerprint`、`matched`、
`rebound`、`policy`、`anchors_detached`、`geometry_transformed`、
`bookmarks_transformed`。`decode` 是丢弃该报告的薄封装。

**标量测量值语义**：显式映射只移动坐标，不对测量的标量 `value` 做面积/长度
换算，因为映射可能非均匀（相似变换才可换算）。调用方需在映射后重新计算测量；
当前实现保留原值，属于已知语义限制（见 OPEN）。

## 3. 命令与事务（F07/F08，B12）

`AnnotationService::apply` 把单个 `AnnotationCommand`（`Create`/`Update`/
`Delete`）作为一次事务提交，返回 `ChangeSet`。所有写入经
`cad-db` 的单一受控路径 `AnnotationDatabase::apply_annotation_changes`，该校验
id/key 一致、几何有限性与有效性、样式边界、时间顺序、锚点引用；任一非法则整
事务回滚、不推进 revision、不产生部分写入（B12）。精确 `ChangeMask` 由
`cad-db` 生成。

## 4. 导出原子性（F09/B07）

- `AnnotationService::encode` 为纯函数：失败返回 `Err`，不触碰数据库。
- `AnnotationService::export_sidecar`：先编码，只有编码完全成功才把调用时捕获
  的 revision 标记为已保存；失败则保持 dirty、revision 不变、既有内容不变。
- 需要等待宿主持久写入确认的宿主应使用 `encode`，写入成功后再调用
  `AnnotationDatabase::mark_exported(revision)`。`cad-app` 已提供
  prepare/confirm 两段式（不在本 crate）。

## 5. 隐藏/可见状态

`cad-db` 的 `Annotation` 目前**没有** hidden/visible 字段，边车格式也没有该
状态。因此本层无法往返一个不存在的字段，未伪造。列表 UI 的隐藏状态属
F09，见 OPEN。

## 6. OPEN（需要应用/宿主层或 schema 变更）

- **隐藏/可见状态**：需要 `cad-db::Annotation` 增加字段（本 worktree 禁止修改
  `cad-db`）或由宿主侧维护逐注解可见性映射；列表 UI 的显示/隐藏入口属
  `cad-app`/`cad-ui-slint`。
- **注解内部未知子字段保留**：需要扩展 `cad-db::Annotation` 或其边带；当前只
  保留顶层未知字段，未声称支持注解内未知字段。
- **显式映射的标量测量换算**：需要对相似映射判定并换算，或由上层重新测量。
- **迁移历史样本**：需要一个更早 `schema_version` 的真实样本才能验收迁移路径；
  当前只有 v1。
- **宿主文件导入/导出**（Android SAF、Web 下载确认、IndexedDB 恢复）：属
  `cad-app`/宿主，不在本层。
