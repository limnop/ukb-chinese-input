# WowType 中文输入引擎源码移植报告

**分析对象：** `../wowtype`（上级目录中的 `Gloridust/WowType` 克隆）
**源码快照：** `main`，commit `38eb269989a4c54c96c4b5749a13294750498b5e`（2026-11-14）
**范围：** 拼音引擎、词库/语言模型、候选学习、输入会话及其向本 Android 项目移植的建议。

**当前状态：** 已完成 WowType 独立替换，最终结构和验证见第 9 节；第 4、8 节保留迁移过程记录。

## 1. 结论

WowType 的中文引擎主体是独立 Rust crate `core/`，不依赖 fcitx5；fcitx5 插件是 C++ 外壳，通过稳定化中的 C ABI 调用 Rust。核心能力已经覆盖全拼、简拼、音节切分歧义、长句束搜索、按键纠错、补全、模糊音、双拼、用户学习及可替换语言模型。输入状态和按键规则正从插件迁入 `core/src/session.rs`，并暴露 `wowtype_session_*` C ABI。

本项目已开始按**复用 Rust 核心，以 JNI 提供薄桥接，再由 Java `Composer` 接入现有候选 UI**的路线接入。不会移植 fcitx5 插件；它承担 Linux 输入法框架及桌面交互。当前接入保留 Android 的拼音编辑与候选 UI，将 WowType 整句候选排在前面，并保留原先逐音节候选作为后备。

## 2. 克隆与仓库结构

- 仓库已克隆到 `/home/limnop/Projects/wowtype`，使用 `git clone --recurse-submodules`。
- `.gitmodules` 不存在，`git submodule status --recursive` 无输出；该快照没有 Git 子模块需要补拉。
- 移植用的核心源码副本在当前仓库 `vendor/wowtype-core/`，含 ABI 头文件、核心模块和 Android 精简 Cargo 清单；上游快照仍保存在同级目录用于对照。
- WowType 本身的完整外部语料/词库不在工作树内。当前 APK 的词库由项目现有 `assets/zh_pinyin.dat` 生成，词频至少 300 的 340,274 条保留，4 条高频数据无法切分而跳过；生成的 TSV 约 8.98 MB，压缩后 APK 中约 3.78 MB。代码按词的汉字数恢复拼音边界，以最长合法音节优先；平拼代码的固有歧义仍可能导致个别条目分音节不同于原意。

核心目录：

| 路径 | 职责 |
|---|---|
| `core/src/lib.rs` | `Engine`、配置、候选生成总流程、用户历史和接口汇总 |
| `core/src/syllable.rs` | 拼音音节表、规范化/解析、音节 ID、模糊音规则 |
| `core/src/graph.rs` | 把输入字母建成有向无环音节图，表示多种合法切分及纠错/补全边 |
| `core/src/lexicon.rs` | 词典 trait、TSV 内存词典、前缀索引、模糊索引、用户/自定义词合并 |
| `core/src/beam.rs` | 按输入位置进行词格束搜索，拼出长句和句首词候选 |
| `core/src/lm.rs`, `wordlm.rs`, `nnlm.rs` | 一元/二元、词级语言模型与可选字级 NNLM |
| `core/src/rank.rs` | 候选特征与线性/LTR 排序接口 |
| `core/src/learn.rs` | 选词历史、用户词及词对搭配的持久化格式和学习逻辑 |
| `core/src/shuangpin.rs` | 七种双拼编码到全拼的转换 |
| `core/src/session.rs`, `session_ffi.rs` | 平台无关的输入编辑状态、按键/选词/分页与 C ABI 会话 |
| `core/src/ffi.rs`, `core/include/wowtype_core.h` | C ABI 引擎、候选、设置、配置接口 |
| `fcitx5-addon/src/wowtype_engine.cpp` | fcitx5 平台适配及尚留在外壳中的部分输入行为 |

## 3. 中文引擎的数据流

```text
键入拼音 / 双拼
      ↓
规范化与解析（syllable.rs；双拼先由 shuangpin.rs 解码）
      ↓
音节 DAG（graph.rs：歧义切分、简拼、儿化、补全、错键路径）
      ↓
词典查找 + 词格束搜索（lexicon.rs + beam.rs）
      ↓
词频代价 + LM 上下文分数 + 候选特征/历史重排（lm.rs + rank.rs + lib.rs）
      ↓
Candidate{text, pinyin, score, words, consumed}
      ↓
输入会话选择/提交/学习（session.rs）→ C ABI → 平台 UI
```

### 3.1 拼音解析与图搜索

`syllable.rs` 先将拼音映射到有限音节 ID。`graph.rs` 并非贪心地只选最长音节，而是在每个输入位置生成可能边，因而 `xian` 可以同时解释为 `xian` 或 `xi an`。边带有起止位置、类型和惩罚属性；支持全拼、简拼、儿化，按配置支持相邻键错误、换位、多/漏键和尾部未完成音节。

`beam.rs` 从输入位置 0 向后搜索。每个位置只保留一定宽度的高分状态，在每个状态上枚举可覆盖多个音节的词；词典没有命中时用未知音节兜底，避免输入断路。词典的 `has_longer` 前缀查询用于剪枝。该设计可组合长句，而不需要枚举所有分词组合。`Candidate.consumed` 记录候选消费了多少输入，支持先选句首词、保留剩余拼音继续输入。

### 3.2 词典与语言模型

`Lexicon` trait 将查询策略与具体存储分开；现有 `MemoryLexicon` 从 `拼音<TAB>词<TAB>频次` TSV 建索引，缓存编译后的快照，并实现精确、简拼、模糊音和前缀查询。`UserLexicon` 将基础词库、用户学习词与自定义词合并。替换生产词库可实现新的 `Lexicon`，不必动束搜索。

排序主分由词典概率/代价与语言模型分数组成，再由 `Feature` 和 `Ranker` 重排。LM 是 trait，可接 Bigram TSV、ARPA 或项目自己的实现；WowType 文档明确提到微信 `lmcnmac.bin` 只存 key 哈希，不能从数据文件直接枚举导出。可选 `nnlm.rs` 用于候选重排，默认不应作为 Android 首期依赖（内存、模型许可、冷启动和 ARM 性能均需独立评估）。

### 3.3 引擎已有能力

- 全拼整句、简拼回退、混合简拼、省略韵母、长句句首词。
- 尾音节补全；相邻键、相邻字母互换、多按/漏按及跨音节换位纠错。
- 12 组可单独配置的模糊音；双拼支持自然码、小鹤、微软、搜狗、智能 ABC、紫光、拼音加加。
- 依据词频、上下文 LM、用户历史/词对、覆盖率、纠错数等排序；选择可自动学习并可撤销/遗忘。
- 可选繁体转换、英文词、自定义编码词、日期时间、计算器、符号和 emoji。

这些是核心目录提供的能力，不意味着仓库内已经附带可直接发布的完整中文字典、训练语料或模型。README 中的数据效果和时延数值是 WowType 项目自己的评测口径；迁入 Android 后应使用本项目真实输入场景重新测量，不能直接视为设备保证。

## 4. 与当前 Android 项目的对接

现有中文路径主要在 `srcs/juloo.keyboard2/staged/Pinyin.java`（拼音组合和编辑行为）、`PinyinData.java`（音节/单字表）、`Syllables.java`（单音节候选）和 `dict/ChineseWordDictionary.java`（异步加载词典）。布局/候选呈现走既有 `Composer`、`Suggestions` 和候选视图。它目前偏向逐音节字词候选，WowType 则是拼音串→整句/词候选；两者候选语义不同，建议在 `Composer` 边界新增实现或受控替换，不要直接把 Rust 搜索塞入 UI/View 层。

推荐桥接分层：

1. **Rust core 保持平台无关。** 首期只编译 `wowtype-core`，关闭默认 `voice` feature；语音 crate、fcitx5 插件、GTK 设置和桌面服务不进入 APK。
2. **JNI 管生命周期和字符串。** 将词库、配置、历史载入封装为 native engine handle；输入时传拼音 UTF-8，返回候选文本、分音节拼音、消费长度和类型。候选字段批量取回，避免每项多次 JNI 往返。
3. **Java Composer 管键盘交互。** 把已有 Composer 收到的按键翻译成平台无关的会话键，接收 `preedit`、候选页、光标、提交文本；中文组合时由该 Composer 决定何时向编辑器提交。Android 的数字选词、方向键、退格、Esc、标点和光标移动需逐项映射并保留本项目既有手势与候选 UI 约定。
4. **输入会话接口先审计后接。** `wowtype_session_*` 当前标注“暂定/过渡”，源码表明有些输入行为仍在 fcitx5 C++ 插件里。移植 Android 前确认所需操作在 Session 完整覆盖；必要时在 Java 保留薄平台策略，不要假设 C 会话已经涵盖所有标点/模式键。
5. **资产放 app 私有存储或 APK 压缩资源。** 大词库首次释放到内部文件后再初始化；加载应放后台线程，确保尚未完成时输入响应不阻塞。可通过 `set_engine` 在词典就绪后连接会话（该 API 在 ABI 文档有说明）。用户词、历史和自定义词应走应用私有目录，避免外部存储权限。
6. **渐进接入。** 第一阶段只做全拼 + 词典 + 候选 + commit；第二阶段加入用户历史/上下文；第三阶段简拼/纠错/模糊音；第四阶段双拼和会话内选词。计算器、翻译、emoji、语音不属于中文解码核心，可分开决定。

### 本项目已实现的第一阶段

- `vendor/wowtype-core/` 固定在上面所列上游 commit；四种 Android Rust target 编出 `staticlib`，由 `vendor/Android.mk` 链入 `libwowtype_jni.so`。Rust 默认语音 feature 已关闭。
- `WowtypePinyinEngine` 在后台解压/落盘词库并创建 opaque engine handle。JNI 仅承载创建、输入候选、选词记录和释放；`Pinyin` 仍控制组合文本、手动隔音符、候选 UI 和提交。
- 无手动隔音符时，WowType 的全句、句首词、尾部补全及错键候选排在候选前部；原有逐音节候选补足长尾。输入了隔音符则跳过整句候选入口，并通过显式音节分隔查询，避免消掉用户的读音选择。
- 选词记录到应用私有目录的 `history.tsv` / `user.tsv`。开始新拼音串时，从编辑器读取光标前最多 64 个 UTF-16 字符传给 WowType；密码编辑框不读取。句首部分选词后，将已选文本作为后续输入上下文。
- 设置中已提供“纠正拼音输入错误”（默认开启）和“模糊音”（默认关闭），切换后应用到 WowType 核心。模糊音组采用 WowType 内置默认配置，当前没有逐组开关。
- 未带入 WowType 的通用语料语言模型；上下文排序只可利用核心现有排序逻辑及本地学习历史，不能视为完整的大模型/词级二元模型效果。完整 `session` 按键状态机仍未接入，Android 继续使用原 Composer 管理输入交互。
- 原 Java 大词库只在 native 初始化失败时异步加载为后备，避免两份大词库同时常驻内存。

本阶段编译检查结果：`./gradlew compileDebugJavaWithJavac externalNativeBuildDebug assembleDebug` 通过，四种 ABI 均成功构建。没有运行单元测试，也尚未在设备/模拟器上检查实际输入延迟、候选效果和内存峰值。

## 5. 主要移植风险与决策点

### 构建和 ABI

`core/Cargo.toml` 生成 `rlib`、`cdylib`、`staticlib`，crate 依赖以 `anyhow`、`serde_json`、`libc` 为主；默认 feature 引入语音依赖，Android 应用 `--no-default-features`。Rust crate 没有 Android 专用 JNI 包装，当前 C ABI 面向桌面调用；需要增加 JNI shim 或在现有 Java 侧通过 JNI 调 C ABI。Android ABI 至少需要按 `arm64-v8a`（视产品需求加 `armeabi-v7a`、`x86_64`）分别交叉编译并验证 NDK 链接、C 标准库和打包。不要把 `Engine` 的 Rust 内存布局直接暴露给 Java，只用头文件里的 C ABI/opaque handles。

ABI 文档说明引擎句柄不可并发调用，Session 是单线程对象，若要留住返回字符串需按其生命周期拷贝；JNI 层需明确线程归属、释放时机和错误传播。WowType 的 ABI 正在演进，当前头文件比早期“仅输入+候选”接口复杂，必须以同一 commit 的 `core/include/wowtype_core.h` 与 Rust 库一起集成，不要混用发布版头文件。

### 数据、许可和体积

- 词库 TSV 本身只是可加载格式，不包含生产词条。WowType README 介绍的雾凇词库、项目生成的词级 LM、OpenCC 转换数据和 NNLM 各有不同来源/许可，应逐个记录来源、版本、许可文本和随 APK 再分发条件。
- 现项目已有 Jieba 派生中文词库、Unihan 音调表和 cdict 编码格式，说明可考虑复用现有数据管线，或将其转换为 WowType TSV/定制 Lexicon；不要把两个项目的压缩格式混为一谈。
- 全量词库/LM/NNLM 会影响 APK 下载体积、RAM 和启动时间。建议先用可再分发且较小的词表打通，之后对候选质量、文件大小、内存峰值、初始化时间做设备实测。
- 加载生产词库可走 WowType 的 TSV parser，简单但可能增加内存；若不足，再实现直接查询现有 `cdict`/更紧凑的 trie 适配层，避免一次性把大量词条复制到 Java。

### 产品交互差异

WowType 面向桌面键盘，默认有候选页、全局方向键、展开面板和桌面快捷键。手机软键盘具有候选条宽度、滑动选词、触摸删除、长按等不同交互。应复用引擎“候选 + consumed + session 状态”概念，但对移动端候选页和键盘手势做单独映射；无须照搬 fcitx5 外观或所有快捷键。

### 项目清洁度

克隆后 `../wowtype` 工作树干净，当前主项目已有的改动（`res/xml/bottom_row.xml`、`res/xml/clipboard_bottom_row.xml`、`ClipboardSyncClient.java`）没有被本次操作触碰。报告仅新增在当前仓库 `doc/`。

## 6. 建议的实施顺序

1. 固定依赖快照，确定上游版本更新策略；先在本地 host 上通过 Rust C ABI 做 JNI 原型，不改现有中文默认行为。
2. 设计 Android 用 `Composer` 原型，输入 `nihao`、`xian`、`zhongg`、长句、退格/撤销/候选点击/提交；确认编辑区与候选 UI 对接。
3. 选定可合法分发的词库。先转换成 WowType TSV 并评估冷启动与内存；再评估现有 cdict 适配以降低资源成本。
4. 建立设备指标和人工标注用例，比较当前 Pinyin 与接入后的 top-k、逐键延迟、崩溃/ANR、RAM、包体；依据结果决定替换或并行保留两套引擎。
5. 当前已接入用户学习、上下文尾部、模糊音开关和拼音纠错开关；下一步应在设备上验证候选质量与编辑器上下文读取行为，再评估简拼、双拼和模型重排。

## 7. 关键源码索引

- 上游总流程：[`../wowtype/core/src/lib.rs`](../../wowtype/core/src/lib.rs)
- 音节与拼音图：[`../wowtype/core/src/syllable.rs`](../../wowtype/core/src/syllable.rs)、[`../wowtype/core/src/graph.rs`](../../wowtype/core/src/graph.rs)
- 词典与束搜索：[`../wowtype/core/src/lexicon.rs`](../../wowtype/core/src/lexicon.rs)、[`../wowtype/core/src/beam.rs`](../../wowtype/core/src/beam.rs)
- 排序、LM 与学习：[`../wowtype/core/src/rank.rs`](../../wowtype/core/src/rank.rs)、[`../wowtype/core/src/lm.rs`](../../wowtype/core/src/lm.rs)、[`../wowtype/core/src/learn.rs`](../../wowtype/core/src/learn.rs)
- 会话与 ABI：[`../wowtype/core/src/session.rs`](../../wowtype/core/src/session.rs)、[`../wowtype/core/src/session_ffi.rs`](../../wowtype/core/src/session_ffi.rs)、[`../wowtype/core/include/wowtype_core.h`](../../wowtype/core/include/wowtype_core.h)
- 上游已有移植/数据说明：[`../wowtype/docs/PORTING.md`](../../wowtype/docs/PORTING.md)、[`../wowtype/docs/ENGINE.md`](../../wowtype/docs/ENGINE.md)、[`../wowtype/docs/DATA.md`](../../wowtype/docs/DATA.md)、[`../wowtype/docs/ABI.md`](../../wowtype/docs/ABI.md)
- 本项目当前中文实现：[`../srcs/juloo.keyboard2/staged/Pinyin.java`](../srcs/juloo.keyboard2/staged/Pinyin.java)、[`../srcs/chinese/README.md`](../srcs/chinese/README.md)

## 8. WowType 独立运行与模拟器检查（2026-11-14）

本节取代上文关于旧候选补足及初始化失败后回退的描述：当前 `Pinyin.candidates()` 只返回 WowType 候选，旧逐音节候选生成路径已屏蔽，旧 Java 词库的异步回退入口已移除。WowType 尚未就绪或加载失败时不提供中文候选，初始化错误写入 Android 日志。旧辅助代码暂留，现有词库仍作为 WowType 构建数据源。

手动隔音符直接传入 WowType，例如 `xi'an` 得到“西安”。部分候选的消费长度按原始输入位置处理，学习记录也使用同一位置边界。

重新构建 `assembleDebug` 并安装到 `acode_test` 模拟器，以 Android Instrumentation 在应用进程运行 Composer → Java adapter → JNI → Rust 的真实调用链：

- 初始化前无旧引擎候选；初始化后来源为 `WowtypeWordSource`。
- `nihao`、`zhongguo`、`xi'an`、`woaini` 首选分别为“你好”“中国”“西安”“我爱你”。
- `nihap` 纠错首选为“你好”。
- 整句选词清空组合文本；`nihaozhongguo` 部分选择“你好”后保留 `zhongguo` 并继续产生候选；退格后可继续生成候选。
- 本轮查询耗时 0–4 ms，仅为模拟器少量样例，不代表性能基准。crash 日志为空。

测试入口与原始输出保存在 `doc/wowtype-smoke/`。此次为设备端引擎集成冒烟测试，未覆盖键盘触摸、候选条布局、不同应用编辑器或长时间压力测试。


## 9. 完成替换与交互回归

- 删除 `ChineseWordDictionary.java`，`Keyboard2` 直接初始化 `WowtypePinyinEngine`。
- `Pinyin.java` 仅保留原始输入编辑、光标、上下文及接受候选；删除 Java 拼音切分、候选合成、排序和 WordSource 兼容接口。`Pinyin.Engine` 仅定义候选查询与选词学习，不再支持旧词库接口。
- 字符表与 `Syllables` 仍供注音输入使用。拼音不依赖这些表进行解码。
- 隔音符在组合文本中显示，可逐字符编辑；消费长度、光标和学习输入采用一致的原始位置。学习时 JNI 查询候选的标准全拼，避免存入错拼或简拼编码。
- 原始词库移动至 `srcs/chinese/data/zh_pinyin.dat`，只在构建时生成 WowType TSV，确认 APK 中没有旧 `zh_pinyin.dat`。
- 更正词库来源描述：该原始拼音资产的构建脚本记录的是 RIME 词表合并流程；Jieba/Unihan 文档描述另一套词典，不能作为该资产来源证明。细节和未记录的版本信息见 `THIRD_PARTY_NOTICES.md`。

验证：55 项 JVM 测试全部通过；Debug / Release APK 构建均通过；Android instrumentation 覆盖真实 JNI/Rust 候选、纠错、隔音符、部分选词、退格与无旧候选回退，结果见 `wowtype-smoke/result.txt`。

在 x86_64 `acode_test` 模拟器实际操作软键盘：

1. 输入 `nihao`，空格上屏“你好”。
2. 输入 `nihao`，点击“你”，组合区剩余 `hao`，点击“好”后续写成功。
3. 滑动输入逗号，编辑框显示“你好你好,”。
4. 输入 `xi'an`，首选“西安”；滑动方向键将光标移到 `xi'a|n`，删除并重新输入 `a`，再用空格上屏，编辑框显示“你好你好,西安”。
5. 切换到系统设置搜索框，再输入 `nihao` 并空格上屏，得到“啊你好”（“啊”为该框原有内容），未携带上一编辑框的组合串。

未观察到本应用崩溃；系统 crash 缓冲区有模拟器蓝牙进程的历史崩溃，与本应用无关。尚未覆盖实体设备、多 Android 版本及长时间压力测试。


## 10. 对齐上游默认数据与候选覆盖

当前默认构建已改用 WowType 上游数据管线，不再使用裁剪后的原始拼音词库。

- 雾凇输入固定为 `927e1cf1d3eeb9bf4be85e3f684dba4dd485b6f6`；上游转换脚本和热词表固定到核心源码同一提交。生成 929,403 条词条/读音记录，其中大字表补充 37,516 个单字读音。保留原始音节边界和词频，不再从连写拼音猜边界。
- 使用上游 `lm-2026.11.09` 的词级模型，SHA-256 校验通过。模型约 38.46 MB，词库约 28.74 MB。来源和许可随 APK 分发，见 THIRD_PARTY_NOTICES.md。
- 与 fcitx5 外壳一样将候选上限设为 1000；束搜索宽度仍为上游默认 20。上游默认不展示的极生僻扩展区字符仍遵循核心规则。
- JNI 双向使用标准 UTF-8，支持非 BMP 汉字和带 emoji 的上下文；修复先前 modified UTF-8 与 Rust UTF-8 不兼容的问题。
- APK 中包含数据 SHA-256 清单；覆盖安装后按内容变化校验并原子替换词库/模型，清除旧词库缓存，保留 history.tsv / user.tsv。

验证结果（x86_64 模拟器、小样例，非基准）：

- 55 项 JVM 测试通过，Debug/Release 构建通过，Release APK 约 42 MB。
- 现有全拼、纠错、隔音符、部分选词及退格通过。
- `ji` 返回 817 个候选，包含扩展区汉字“𬶭”，本轮查询约 3 ms。
- `jintiantianqihenhao`、`woxiangqubeijing`、`zhonghuarenmingongheguo` 首选分别为“今天天气很好”“我想去北京”“中华人民共和国”，约 6–7 ms。
- 主动写入过期数据标记后重新初始化，确认重新释放词库、生成缓存，词库与模型在设备上的 SHA-256 均与 APK 清单一致，原有学习历史内容保持不变。
- 已有缓存时初始化约 1.3 秒；强制更新并重建词库缓存约 5 秒。
- 进程 PSS 约 458 MiB，其中 native heap 约 428 MiB。这是完整内存索引的明显移动端成本，不能当作已完成性能优化。

剩余质量工作：压缩常驻索引/降低初始化峰值与时延；建立更大且固定的输入评测集，与同数据的上游结果逐项比较；补齐上游会话中的分段组词学习、撤销/遗忘等交互。当前已对齐默认词库、模型和候选上限，但没有证明整体体验已达到上游水平。
