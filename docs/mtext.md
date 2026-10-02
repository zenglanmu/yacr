# MTEXT / TEXT 格式解析与排版

本文记录 `cad-representation::text` 对 MTEXT/TEXT 内联格式码的解析与排版行为：
哪些是精确实现，哪些是部分实现（并在渲染时显式报告），哪些尚未支持。
规范依据：§3.2、§7.3、§11.1（complex MTEXT/中文）。

## 入口

- `parse_mtext(raw, base_height) -> ParsedText`：把控制内容解析成**结构化 run 列表**
  （`TextLine { runs: Vec<TextRun> }`），不再只是剥离码。`base_height` 是实体标称字高
  （世界单位），用于解析 `\H`。
- `FontEngine::shape(font_key, raw, origin, height, rotation, h_align, v_align) -> ShapedText`
  ：按 run 逐段排版，返回 `polylines`（世界坐标折线）、`runs`（扁平化后的带样式 run，
  供宿主上色）与 `issues`（近似/未支持项的显式报告）。
- `FontEngine::outline(...)`：`shape` 的薄封装，仅返回 `polylines`，行为与旧版兼容。
- `sanitize_text(raw)`：保留的文本净化函数（供不需要 run 结构的调用方）；与 `shape`
  使用同一套代码知识，但把内容压成纯字符串，堆叠分数渲染为两行。
- `DefaultRepresentationProvider` 的文本分支改为调用 `shape`，把 `issues` 合入
  `Completeness::Partial` 并逐条写入 `Diagnostic`（code 即 `text_issue::*`）。

## 逐符解析行为

解析器为**上下文栈**结构：`{` 压入当前样式快照，`}` 弹出恢复；`\P`、字面 `\n`、`^J`
结束当前段并开始新段。同一段内相邻且样式相同的 run 会合并。CJK 按 Unicode 标量处理，
不做字节切分；run 的切分来自样式变化，不来自字符宽度。

### 精确实现（exact）

| 码 | 含义 | 行为 |
|---|---|---|
| `\\` `\{` `\}` `\;` | 字面转义 | 输出对应字符 |
| `\P`、`\n`、`^J` | 段落/换行 | 新起一段，行距见下 |
| `{` `}` | 分组作用域 | 压栈/弹栈完整样式 |
| `\~` | 不换行空格 | U+00A0（本渲染器断行不受影响，但语义保留） |
| `\U+XXXX` / `\u+XXXX` | Unicode 转义 | 合法码点输出该字符，非法报 `MALFORMED_CODE` |
| `\f<font>|b0|i0|c<n>|p<n>;` / `\F<font|flags>;` / `\FN<name>.shx;` | 字体 | 取首段作为字体名，逐 run 解析字形 |
| `\H<v>;` | 绝对字高 | 该 run 字高 = v；行距按该行最高 run 计算 |
| `\H<v>x;` | 相对字高 | 对当前有效字高乘 v（可叠加，如 `\H5;\H2x;` → 10）|
| `\C<aci>[;<aci2>];` | ACI 颜色 | 0..=256 记入 run；`<aci2>`（渐变）解析但不使用 |
| `\c<packed>;` | 真彩色 | 字节序 R=低 8 位，记入 run |
| `\W<v>;` / `\W<v>x;` | 宽度因子 | 缩放字形 x 与 pen advance |
| `\Q<deg>;` | 倾斜角 | 字形 x 剪切 `y*tan(deg)`；基线推进按 unskewed advance |
| `\A<n>;` | 行内对齐 | 0..=2 解析进 run，但**排版未应用**（见 Partial）|
| `\S<num><sep><den>;` | 堆叠 | 结构保留在 `TextRun::fraction`；**内联渲染**（见 Partial） |
| `\L\l \O\o \K\k \b0/\b1` | 下划线/上划线/删除线 | 解析进 run 标志；**不绘制**（见 Partial） |
| `%%d` `%%p` `%%c` `%%%%` | 特殊符号 | °、±、⌀、% |
| `%%<number>` | 十进制字符码 | 输出对应字符 |
| `\t`、`^I` | 制表 | 展开为 `TAB_STEP`（4）个空格（近似，非真正制表位）|
| `\X` | 换行标记 | 忽略（无输出）|
| `^M` | 回车 | 忽略 |

### 部分实现（partial，渲染时显式报告）

以下格式码被正确解析并保留在 `TextRun` 中，但当前折线渲染器无法忠实呈现，`shape`
会返回 `TextFormatIssue`，`DefaultRepresentationProvider` 会据此把文本标为
`Completeness::Partial` 并写诊断：

- **堆叠分数 `\S`**：`num/den` 内联绘制为 `num<sep>den`（`/`、`#`、`^` 均为分隔符），
  没有分数线、没有上下堆叠、没有缩字。code = `mtext.stacked_fraction_flat`。
- **内联颜色 `\C`/`\c`**：run 携带颜色，但 `DisplayPrimitive::Lines` 无颜色通道。
  code = `mtext.color_not_applied`。
- **下划线/上划线/删除线**：解析进 run，但未生成装饰线。
  code = `mtext.decoration_not_applied`。
- **`\A` 行内对齐**：解析进 `run.line_align`，排版未应用。
  code = `mtext.line_alignment_not_applied`。
- **`\T` 字符间距**：解析但未应用。code = `mtext.tracking_not_applied`。
- **`\p...;` 段落属性**（缩进/对齐/制表位）：整体忽略。code = `mtext.paragraph_properties`。
- **`\N` 分栏符**：当作普通换行。code = `mtext.column_break`。
- **`\B` 背景遮罩**：解析但不绘制。code = `mtext.background_mask_not_applied`。

### 未支持 / 降级

- **未知码 `\Z`**：按 DXF 规则**原样输出**（`\Z` 两个字符），报 `mtext.unknown_code`，
  绝不 panic。
- **值缺失/非法**（如 `\Hb`、`\Z` 结尾无 `;`）：按码语义消费，报 `mtext.malformed_code`。
  结尾的孤立 `\` 原样输出并报 `mtext.malformed_code`。
- **MTEXT 列、真正制表位、逐段对齐**：未实现。
- **bidi / 复杂文字整形 / 上下文连字**：未实现（沿用模块既有边界）。
- **每字形回退**：run 指定的字体未注册且回退链也没有可用面时，该 run 不产生几何，
  报 `mtext.font_unavailable`；其余文本照常渲染。

## 字高与行距

- run 有效字高：有绝对 `\H` 用绝对值；否则 `base_height × 相对因子`（默认 1.0）。
- 行距：`LINE_SPACING`（1.2）× **该行最高 run 的字高**。这是对字体垂直度量的近似，
  不同字高混排时逐行下移。
- `\H` 相对因子会**复合**当前有效字高（绝对×相对、相对×相对）。

## 对齐锚点

- TEXT 的 `horizontal_alignment`/`vertical_alignment`（含 `alignment_point`）与 MTEXT 的
  `attachment_point` 在导入器映射为 `TextAlignH/TextAlignV`（导入器已是现状，未改）。
- `finalize_lines` 逐行按行宽做水平对齐（左/中/右），保持多行块贴齐锚点；垂直按
  `Baseline/Bottom/Middle/Top` 偏移。多行块的垂直放置仍是近似（见 `docs/fonts.md`）。

## 测试

`cad-representation/src/text.rs` 的单元测试覆盖：分组/字高/颜色/换行的 run 列表、
行数与行距推进、宽度因子与倾斜对 advance/字形 x 的影响、非法输入降级、CJK run、
对齐锚点；以及可选的 `YACR_TEST_FONT` 真实字体集成测试。合成 `mock_measure` 度量
（每字符宽 `0.5×height`、高 `height`）使推进/剪切断言不依赖具体字体。

## 与其他模块的关系

- `cad-import-acadrust` 已存储 TEXT/MTEXT 的样式字段（`Text` 几何的 `style`/`font`/
  `h_align`/`v_align`）；本次**未修改导入器**。
- `cad-scene` 的批注文本仍走简单 `outline`（批注不是 MTEXT）。
- `FontEngine::shape` 复用现有回退链与 kern 逻辑；未注册字体的诊断行为不变。

## 参考（仅行为，独立重实现）

MTEXT 码语义参照公开 DXF 规范；并查阅了 GPL-3.0 的 OpenCADStudio 克隆
（`/tmp/opencode/ocs` @ `02c470a`）的 `src/entities/text_support.rs`
（rich MTEXT 解析器/`RunState`）与 `src/app/mtext_editor.rs` 以及上游 acadrust
`src/entities/mtext_format/parser.rs`（`parse_mtext`）作为行为对照。
代码为独立实现，未复制其源码。
