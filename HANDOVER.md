# 中文输入分支交接文档

本仓库是 [Unexpected Keyboard](https://github.com/Julow/Unexpected-Keyboard) 的一个
fork，目标是在这块轻量键盘上做出可用的中文输入（拼音 + 注音），并顺手修掉候选栏
在中文场景下的交互问题。当前工作分支：`feature/chinese-input`。

- 本地路径：`~/Projects/ukb-chinese-input`（2026-11-11 从 `~/wb-workspaces/wb- 14/` 迁出，原因见下）
- 包名：`juloo.keyboard2`（版本 2.1.0）。`applicationId` 与上游相同，签名也沿用上游的
  release.keystore，所以它和上游版**不能共存**，装上去就是覆盖升级
- 目标设备：Redmi K70（HyperOS 3 / Android 16）
- 上游：`https://github.com/bqv/Unexpected-Keyboard.git`（由 `origin` 指向，带 ~110 个远端分支）
- 本地提交：`feature/chinese-input` 比 `origin/feature/chinese-input` 多 4 个提交

---

## 1. 快速上手

```bash
git clone <本地或远端地址> && cd ukb-chinese-input
# 构建（Gradle 8.14.3 + AGP 8.13.2，首次约 15s）
./gradlew test assembleDebug assembleRelease
# 产物在 build/outputs/apk/{debug,release}/

# 测试单个算法类
./gradlew testDebugUnitTest
# 报告：build/reports/tests/testDebugUnitTest/index.html
```

> **注意 1：`~/.gradle/init.gradle` 里有注入阿里云 maven 的初始化脚本。** 本项目在
> `settings.gradle.kts` 里用了 `RepositoriesMode.FAIL_ON_PROJECT_REPOS`，初始化脚本
> 往项目里塞 repository 会导致构建直接失败：
>
>     Build was configured to prefer settings repositories over project repositories
>     but repository 'maven' was added by initialization script '~/.gradle/init.gradle'
>
> 已在 `~/.gradle/init.gradle` 里按 `projectDir.path.startsWith(...)` 给本仓库路径加了
> 豁免（与 trime 项目同样的处理）。**换机器或再次搬路径后必须更新这条豁免**，
> 否则构建第一批就失败。

> **注意 2：路径不能带空格。** 仓库原先在 `~/wb-workspaces/wb- 14/` 下（"wb- 14" 里有
> 个空格），AAPT 处理带空格路径会出错，当时靠 rsync 同步到 `/tmp/ukb-build/` 再编译。
> 现已迁到 `~/Projects/ukb-chinese-input`，**可以直接原地构建**。如果日后再挪到带空格的
> 路径，记得恢复"同步到无空格目录再构建"的做法。

---

## 2. 安装与调试（真机）

设备：Redmi K70（`23113RKC6C` / vermeer），HyperOS 3 / Android 16，无线调试。

```bash
adb connect <IP>:5555
adb -s <IP>:5555 install -r -g build/outputs/apk/release/*.apk
adb -s <IP>:5555 shell am force-stop juloo.keyboard2
adb -s <IP>:5555 shell ime enable juloo.keyboard2/.Keyboard2
adb -s <IP>:5555 shell ime set    juloo.keyboard2/.Keyboard2
adb -s <IP>:5555 shell settings get secure default_input_method   # 应回显 juloo.keyboard2/.Keyboard2
```

**顺序很重要**：`安装 → force-stop → ime set`。先 `ime set` 再装新包，系统会把默认
输入法退回搜狗。`ime set` 必须用 `ime list -a -s` 里的短组件名
（`juloo.keyboard2/.Keyboard2`），不能用全名。

**HyperOS 的一个坑**：`pm install` 新包名可能被"纯净模式"拦下报
`INSTALL_FAILED_USER_RESTRICTED`，用 `adb install -r -g` 一般能过。

设备上另装了 `com.unexpect.keyboard`（trime 分叉），包名不同，互不覆盖，别搞混。

---

## 3. 代码地图

### 3.1 核心：`srcs/juloo.keyboard2/staged/`

这个目录是中文输入的算法层，**纯 Java、不依赖 Android 框架**，所以能在 JVM 上直接
跑单测（不需要设备、不需要 Robolectric）。这是本项目最有价值的设计约束，改动时请
守住它。

| 文件 | 职责 |
|---|---|
| `Composer.java` | 输入法合成器接口。`type/backspace/text/cursor/move_cursor/candidates/reset/accept`。`Candidate{text, annotation, consumed}` |
| `Pinyin.java` | 拼音合成器：切分音节、查词表、生成候选、分段（`'`）、去重排序。**改动最频繁的文件** |
| `Zhuyin.java` | 注音合成器，复用 `Syllables`，但候选会同时给繁体 |
| `Syllables.java` | 把"音节列表"变成候选字/词：每个音节取常用字，只替换最后一个音节做组合 |
| `PinyinData.java` | **生成文件，不要手改**。1820 个音节的字表 + 声调 + 繁简映射 + 注音拼写 |

`PinyinData.java` 由 `srcs/pinyin/compile.py` 从 `srcs/pinyin/{syllables,variants}.txt`
生成。改字表要改那三个 txt 然后重新生成：

```bash
python3 -B srcs/pinyin/compile.py
```

`from_unihan.py` 从 Unicode Unihan 库重建 tsv，需要联网下载 Unihan.zip，一般用不上。

### 3.2 词表：`srcs/juloo.keyboard2/dict/ChineseWordDictionary.java`

- 数据：`assets/zh_pinyin.dat`，**54MB，187 万行**，已提交进 git（不提交就构建不出功能）。
- 格式：每行 `code<TAB>score<TAB>word`，按 code 升序、score 降序。整个文件读成
  `byte[]` + 行偏移数组，查询是二分 + 短扫描，不做 HashMap。百万词占用约等于文件大小。
- 由 `srcs/chinese/build_pinyin_words.py` 合并 RIME 词库生成（rime-ice + luna_pinyin
  + 万象），权重要跨源归一（`100*log10(1+w)`），细节见脚本 docstring。
- 后台线程加载，加载完之前拼音退回只查内置字表；通过 `Pinyin.word_source` 静态字段挂上去。
- **关键限制：词表只存无分隔符的拼写。** `xian` 有 120 条、`fangan` 有 2 条，
  `xi'an` / `fang'an` 是 0 条。所有涉及分词的设计都必须绕开这一点（见 4.3）。

### 3.3 与键盘主体的接缝

| 位置 | 说明 |
|---|---|
| `KeyEventHandler.java` | 合成器生命周期的唯一管理者。`set_layout_script()` 选合成器；`send_text()` 先喂给 `_composer.type()`；`suggestion_entered(Candidate)` 走 `_composer.accept()` 处理分段 |
| `Config.java` | `IKeyEventHandler` 接口里有一个默认方法 `suggestion_entered(Composer.Candidate)` 重载，默认转调字符串版 |
| `suggestions/CandidatesView.java` | 候选行。条带区和网格区的点击都把 `Candidate` 对象传回去（不是字符串），分段才可能实现 |
| `suggestions/Suggestions.java` | `_enabled` 现在只由 `editor_config.should_show_candidates_view` 决定 |
| `EditorConfig.java` | `password_editor` 字段 + `_is_password()` 静态判定 |
| `EmojiGridView.java` | 自己实现点击识别，所有事件仍转发给 `super.onTouchEvent` |
| `srcs/layouts/latn_pinyin_zh.xml` | 拼音布局，`script="pinyin"`。引号键绑在 `m` 的 `sw` 手势上了（`m` 左滑 = 半角单引号），这是分词符的实际入口 |
| `srcs/layouts/zhuyin_standard_tw.xml` | 注音布局，`script="zhuyin"` |

`Keyboard2.refresh_candidates_view()` 决定候选行显不显示，现在的条件是：

```java
boolean should_show =
  _config.suggestions_enabled
  && !_config.split_layout
  && !_config.editor_config.password_editor
  && !_config.editor_config.numeric_layout;
```

**只有密码框和纯数字框会隐藏候选行。** 之前还会因为
`TYPE_TEXT_FLAG_NO_SUGGESTIONS`（QQ 聊天框）和 `TYPE_NULL`（Termux）隐藏，那是
"候选栏在某些 App 里消失"的根因，已经去掉了。

---

## 4. 已实现的功能与实现要点

### 4.1 候选行不再自动隐藏（`949c876`）

原本 `CandidatesView.should_show()` 会按编辑器类型判断，QQ 这类声明了
`NO_SUGGESTIONS` 的编辑器就把整行藏了。中文输入的拼音串、光标、候选全在这一行里，
藏了就没法用。现在只保留密码框和纯数字两个例外。

顺带修的两件：
- **数字键在拼音态下选候选**，越界时吞掉该键（拼音里数字无意义，否则会打断合成）。
- **上下键翻页**，左右键移动拼音光标。

### 4.2 Emoji 首次点击失效（`949c876`）

根因在 `AbsListView` 的触摸状态机：`onTouchEvent` 只在 `if (!mDataChanged)` 时才进
`TOUCH_MODE_DOWN`，而 `setAdapter` 之后到第一次 layout 之前 `mDataChanged` 是 true，
这一下 tap 就被丢了。滑一下会触发重新布局，所以"滑动一下就好了"。

修法是 `EmojiGridView` 自己识别 tap：记下 `ACTION_DOWN` 的命中位置，`ACTION_MOVE`
超过 touch slop 就取消，`ACTION_UP` 在长按时限内就当作点击。**所有事件仍然
`super.onTouchEvent(ev)`**，滚动、惯性、按下反馈都没动。

### 4.3 拼音分词符 `'`（`2e815e4`）

需求：`fan'gan` = 反 + 感，`fang'an` = 方 + 安，而 `fangan` 走自动切分。

**实现要点（这几条都是踩过坑之后定下来的，改之前先读）**

1. **引号进 `_keys` 但不进 `text()`。** `_keys` 里存的是带引号的原始串（`xi'an`），
   `text()` 过滤掉引号返回 `xian`——这是显示给用户、以及按 Enter 原样上屏的内容。
   `cursor()` 同样按字母计数，光标不会停在引号后面。

2. **`type()` 决定引号收不收。** 串首、连续两个、以及前一个字母是 `n` 或 `r` 时丢弃
   （没有音节以 n/r 接元音续写，这个引号不可能改变读音）。`type()` **总是返回 true**，
   否则 `send_text()` 会把 `'` 当普通字符提进编辑器。

3. **`backspace()` 先跳过分隔符。** `xi'an` 退一次是 `xi'a`（显示 `xia`），再退才到
   `xi`。循环写法，删到字母才返回。

4. **查词表必须把各段拼回去。** 因为词表没有带引号的条目（见 3.2）。`xi'an` 用
   `xian` 去查，再用**"词的字数 == 整串音节数"**过滤：西安 2 字配 2 音节 → 保留；
   先 1 字 → 过滤掉，因为那是 `xian` 一个音节的读法。

5. **分词时不做前缀展开。** `prefix_candidates()` 只在非分词时调用。否则第一段的
   候选会和"未输入的音节"组合，把整行占满。这是 `n` 只出"嗯"、`nih` 直接为空的回归
   根源。

6. **分词时每段单字限 10 个**（`PART_CHARACTER_LIMIT`）。`xi` 有 47 个可用字
   （西系细习席息喜吸希洗戏袭悉嘻稀熙夕…），不限制的话后段的词全被挤没。

7. **完成整串的那一段排最前。** 排序在 `candidates()` 里做（`completes && before > 0`
   时把这段整体 `addAll(0, ...)`），不要在 `add_part()` 里搞。

8. **`accept()` 跳过开头的分隔符**，这样点"西"之后剩 `an` 而不是 `'an`。

**已知取舍**
- `fan'gan` 和 `fangan` 读音完全相同（都是 fan+gan），引号在 `n` 后会被丢弃，所以
  两者候选一致，会出现"反感"和"方案"并列，按词频排。
- `PinyinData.simplified` 的映射不全（如 鐘→钟 缺失），个别繁体碎片（`鐘国`）仍会漏出。

### 4.4 候选去重与分段候选（`6da306f`）

**"重复"不是字面重复**：`nihao` 里 0 条完全相同的候选，但有 20/34 条以"你好"开头；
`zhongguo` 有 10/45 条是繁体变体。处理在 `Pinyin.tidy()`：

- **同首字限流**：回答整串的候选，同首字最多 4 条（`SAME_FIRST_LIMIT`）。
  分段候选（`consumed > 0`）不计数——"你"就是 `nihao` 拆成 你 + 好 的方式。
- **简繁去重**：简体已在列表中时丢掉繁体。**副作用：拼音布局不再出繁体字**
  （注音布局仍出）。这个取舍还挂着，见第 7 节。

**分段候选**：`Candidate.consumed` 记录它覆盖了合成串的多少个字符，0 表示覆盖全部。
每段音节边界都会给出一批 cuts（最多 2 层、每层 12 个），点选后
`accept()` 删掉已覆盖的部分、返回剩余字母，键盘继续合成。

实测规模：`nihao` 34→20 条，`shi` 28→60，`hao` 25→59，`ni` 52→79。

---

## 5. 测试

`test/juloo.keyboard2/staged/` 下有 3 个测试类，不依赖 Android：

- `PinyinTest`（22 个 `@Test`）——音节切分、候选、光标、分词符、分段
- `ZhuyinTest` ——注音
- `ComposerContractTest` ——接口契约

`test/juloo.keyboard2/EditorConfigTest.java` 覆盖 `password_editor` 判定。

**两个坑：**

1. **测试源集是 `java.srcDirs("test")`，但测试文件放在字面的 `test/juloo.keyboard2/`
   点号目录里**（不是 `test/juloo/keyboard2/`）。放错目录编译不会报错，测试静默不执行。
2. **增量编译可能跳过新移入的测试类。** 出现"新加的测试没跑"时，删掉
   `build/intermediates/javac/debugUnitTest` 和 `build/tmp/compileDebugUnitTestJavaWithJavac`
   再跑。

**JVM 探针（强烈建议保留这个调试手段）**：`/tmp/ukb-probe/` 下有一对工具，
直接对着 `android.jar` + 编译产物跑，不需要设备就能看候选列表：

```bash
AJ=~/Android/Sdk/platforms/android-35/android.jar   # 单元测试用 35 即可
CLS=build/intermediates/javac/debug/compileDebugJavaWithJavac/classes
javac -cp "$AJ:$CLS" -d out juloo/keyboard2/dict/Probe.java
java -cp "$AJ:out:$CLS" juloo.keyboard2.dict.Probe assets/zh_pinyin.dat "xi'an" "nihao"
# 输出每条候选，分段候选带 (n) 标记覆盖字数，并自动走一遍 accept 路径
```

`Bench.java` 测每次按键耗时（x86 JVM 上 93–242 µs，真机另说）。

> 注意：`Probe.class` 会缓存旧版 `Pinyin`，改了算法后要 `rm -rf out` 重新编译，
> 否则看到的是旧结果（这次排查中被它误导过一次）。

---

## 6. 构建与环境

| 项 | 值 |
|---|---|
| Gradle | 8.14.3（`gradlew`，缓存于 `~/.gradle`） |
| AGP | 8.13.2 |
| compileSdk / targetSdk | android-36；minSdk 21 |
| JDK | 系统 JDK，有 `source 8 is obsolete` 警告，正常 |
| 签名 | `debug.keystore` / `release.keystore` 在仓库根，已在 `.gitignore` |
| 产物 | `build/outputs/apk/{debug,release}/Unexpected-Keyboard-*.apk`，约 22–29MB |

`.gitignore` 里 `/apks` 被忽略（50MB 构建产物不进历史），但 `assets/*.dat`（54MB）
是提交的——没有它构建出来是空壳。

**release APK 校验和**：
- 装到 10.81.132.233:5555 的那版（迁路径前构建）：
  `8d852f0bfd608c7a0c3e48131a7cc7da34aa684eab4395eb16d04761dff6bb45`
- 迁到 `~/Projects` 后原地重构建的版本：
  `3c453976683013a53cfd1f59e85b400fb84366dd3cabae87216cfc0bef17ee4d`
- 两者源码相同，哈希不同来自 ZIP 内嵌时间戳，功能无差别。

---

## 7. 未决事项 / 下一步

按优先级：

1. **拼音布局要不要保留繁体字。** 现在去重时会把繁体丢掉（简体已列出时）。如果
   需要保留，改成"不在前 N 条并列、但可翻到"。
2. **拼音模糊音。** 注音的模糊输入也还没做。
3. **`PinyinData.simplified` 映射补全**（`variants.txt` 里缺的条目），修 `鐘国` 这类残留。
4. **"仅候选窗模式"**（外接键盘 / scrcpy 场景），从早期就挂着没动。
5. 分段候选在超长串上的性能没有专门测过（`MAX_PARTS = 8` 是拍的）。

---

## 8. 给接手人的几句实话

- 这个分支改动集中在 `staged/` 一个包里，**保持它的纯 Java 特性**是最大的工程资产：
  所有算法都能在 JVM 上秒级验证。往里面塞 Android API 会让调试成本翻十倍。
- 拼音的 `candidates()` 是整个项目最绕的方法：它要同时处理自动切分、分词、词表查询、
  分段、前缀展开、去重。改之前**先用探针把当前输出打出来**，改完再打一遍对比。
- 候选"看起来重复"这类问题，直觉经常是错的（本次 0 条字面重复却看着像重复）。
  先量化再动手。
- `type()` 的返回值语义是"这个键归我消费"，不是"缓冲区非空"。搞混会把分隔符漏进
  编辑器。
