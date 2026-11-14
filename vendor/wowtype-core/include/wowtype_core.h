/*
 * WowType Rust 核心的 C ABI。
 *
 * 生命周期：
 * - wowtype_engine_new 成功返回句柄，必须用 wowtype_engine_free 释放；
 * - wowtype_engine_input 返回的候选句柄必须用 wowtype_candidates_free 释放；
 * - 返回的字符串指针在对应句柄释放前有效（错误串在下一次 API 调用前有效）。
 */
#ifndef WOWTYPE_CORE_H
#define WOWTYPE_CORE_H

#include <stddef.h>
#include <stdint.h>

/* 接口版本（不是 WowType 的版本）。同一个主版本里只加函数、只在结构体末尾加字段；
 * 删函数或改参数含义时升主版本。外壳启动时用 wowtype_abi_version() 比对主版本。
 * 规则见 docs/ABI.md。 */
#define WOWTYPE_ABI_VERSION_MAJOR 1
#define WOWTYPE_ABI_VERSION_MINOR 9

#ifdef __cplusplus
extern "C" {
#endif

/* 编进库里的接口版本：主版本 * 1000 + 次版本。 */
int wowtype_abi_version(void);

typedef struct WowtypeEngineHandle WowtypeEngineHandle;
typedef struct WowtypeCandidates WowtypeCandidates;

/* 创建引擎；bigram_tsv_path 可为 NULL 或空串。失败返回 NULL。 */
WowtypeEngineHandle *wowtype_engine_new(const char *lexicon_tsv_path,
                                        const char *bigram_tsv_path);

/* 释放引擎。 */
void wowtype_engine_free(WowtypeEngineHandle *engine);

/* 设置 beam size；成功返回 0，失败返回 -1。 */
int wowtype_engine_set_beam_size(WowtypeEngineHandle *engine, unsigned int beam_size);

/* 每次输入最多返回多少个整句候选（句首词候选另算）。返回 0 成功。 */
int wowtype_engine_set_max_candidates(WowtypeEngineHandle *engine, unsigned int max_candidates);

/* 开关按错键纠错（相邻键、两键按反、多按/漏按），默认开。返回 0 成功。 */
int wowtype_engine_set_typo_correction(WowtypeEngineHandle *engine, int on);

/* 是否也出 CJK 扩展 B 区及以后的单字（ABI 1.9 起），默认不出：常见中文字体都没有这些字，
 * 列出来是方块。通用规范汉字表里的扩展区字和含这些字的词不受影响。返回 0 成功。 */
int wowtype_engine_set_rare_ext_chars(WowtypeEngineHandle *engine, int on);

/* 载入英文词表（每行一个词，或 rime 的 en.dict.yaml）。
 * 载入后输入已知英文词（hello、github）时英文排在前面。返回累计词条数，失败返回 -1。 */
int wowtype_engine_load_english(WowtypeEngineHandle *engine, const char *path);

/* 载入微信输入法的字级神经语言模型（nlrgpt2_nnlm.bin，不随 WowType 分发），
 * 用来给整句候选重排。返回字表大小，失败返回 -1。 */
int wowtype_engine_load_nnlm(WowtypeEngineHandle *engine, const char *path);

/* 开关模糊音（z/zh、c/ch、s/sh、an/ang、en/eng、in/ing）。返回 0 成功。 */
int wowtype_engine_set_fuzzy(WowtypeEngineHandle *engine, int on);
/* 打开的模糊音分组（位掩码）：1 z/zh，2 c/ch，4 s/sh，8 n/l，16 f/h，32 r/l，
 * 64 an/ang，128 en/eng，256 in/ing，512 ian/iang，1024 uan/uang，2048 un/ong。
 * 默认 1|2|4|64|128|256|512|1024。 */
int wowtype_engine_set_fuzzy_groups(WowtypeEngineHandle *engine, unsigned int groups);

/* 开关双拼。返回 0 成功。 */
int wowtype_engine_set_shuangpin(WowtypeEngineHandle *engine, int on);

/* 选双拼方案：ziranma xiaohe microsoft sogou abc ziguang pinyinjiajia（也认中文名）。
 * 返回 0 成功，-1 不认识。 */
int wowtype_engine_set_shuangpin_scheme(WowtypeEngineHandle *engine, const char *name);

/* 这个双拼方案有没有用分号键（微软、搜狗、紫光的 ing）。 */
int wowtype_shuangpin_uses_semicolon(const char *name);

/* 双拼时编辑区显示的全拼：vsgo -> zhong'guo；没开双拼返回 NULL。
 * 返回的串在同一线程下一次调用本函数或 wowtype_segment_input 前有效。 */
const char *wowtype_engine_shuangpin_display(const WowtypeEngineHandle *engine, const char *input);

/* 开关繁体输出。返回 0 成功。 */
int wowtype_engine_set_traditional(WowtypeEngineHandle *engine, int on);

/* 载入简繁映射表（简体<TAB>繁体）。返回条目数，失败返回 -1。 */
int wowtype_engine_load_conversion(WowtypeEngineHandle *engine, const char *path);

/* ASCII 标点转中文标点；未收录返回 NULL。指针在下一次调用前有效。 */
const char *wowtype_fullwidth_punct(int ascii_codepoint);

/* 按标点样式转换：style 0 全角、1 半角；prev 是光标前一个字符（不知道传 0）。
 * 返回 NULL 表示原样上屏这个 ASCII 字符（半角样式、数字后面的符号）。 */
const char *wowtype_punct(int ascii_codepoint, int style, int prev_codepoint);

/* 中英文之间自动补空格（ABI 1.2 起）：prev 是光标前一个字，next 是要上屏的第一个字；
 * flags 1 = 汉字和字母之间、2 = 汉字和数字之间。要在两者之间补一个空格返回 1，否则 0。
 * 只看这一处边界，词里原有的字母（T恤、卡拉OK）不拆。 */
int wowtype_autospace(int prev_codepoint, int next_codepoint, int flags);

/* 全部内置符号（ABI 1.2 起），结果用 wowtype_candidates_* 读取并释放。 */
WowtypeCandidates *wowtype_symbol_list(void);

/* 剪贴板历史（ABI 1.2 起）：只在内存里，最多 20 条，最新的在前，重复的挪到最前。
 * remember 记下一条（空白、超过 16 KB 的不记），记下返回 1；history 用 wowtype_candidates_*
 * 读取并释放。输入法自己拿不到系统剪贴板，由外壳把看到的文字交进来。 */
int wowtype_clipboard_remember(const char *text);
WowtypeCandidates *wowtype_clipboard_history(void);
void wowtype_clipboard_clear(void);

/* 自定义词（常用语）按 custom.tsv 的顺序（ABI 1.2 起）：text 是词（转义原样显示），
 * pinyin 是编码，wowtype_candidates_commit 是上屏的文字（\n 换成了换行）。 */
WowtypeCandidates *wowtype_engine_custom_phrases(const WowtypeEngineHandle *engine);

/* 符号检索（拼音/中文名前缀），结果用 wowtype_candidates_* 读取并释放。 */
WowtypeCandidates *wowtype_symbol_search(const char *query, size_t limit);

/* 打字时的 emoji 推荐。text 是首选候选的文字；结果的 text 是显示文字，
 * 上屏用 wowtype_candidates_commit（整个词对上时只有 emoji，句尾对上时是整句 + emoji）。 */
WowtypeCandidates *wowtype_emoji_recommend(const char *text, size_t limit);

/* 候选正好是数字字（零到十）时对应的按键 emoji（「八」-> 8️⃣），别的返回 NULL。
 * 返回静态字符串，不用释放。 */
const char *wowtype_emoji_keycap(const char *text);

/* 日期、时间候选：key 是 riqi / shijian / xingqi / date / time（可带隔音符）；时间按本地时区，
 * weekday 0 = 星期日。不是触发词时返回空列表。 */
WowtypeCandidates *wowtype_datetime_candidates(const char *key, int year, int month, int day,
                                               int hour, int minute, int second, int weekday);

/* V 模式计算器：expr 是 v 后面的算式（1+2*3）。算得出来时给「结果」「算式=结果」，
 * 没写完时只给算式本身。 */
WowtypeCandidates *wowtype_calc_candidates(const char *expr);

/* 表情面板搜索。query 为空给全部 emoji；否则按拼音（经引擎转成中文词）、英文、
 * 符号名一起找。engine 可为 NULL。 */
WowtypeCandidates *wowtype_engine_emoji_search(const WowtypeEngineHandle *engine,
                                               const char *query, size_t limit);

/* 记录一次选词，参与历史排序特征。返回 0 成功，-1 失败。 */
int wowtype_engine_record_selection(WowtypeEngineHandle *engine, const char *word);

/* 记录选词并带上拼音：选够次数后自动加入用户词库。pinyin 可为 NULL。 */
int wowtype_engine_record_selection_ex(WowtypeEngineHandle *engine,
                                       const char *word, const char *pinyin);

/* 设置光标前文本，参与上下文 bigram 特征。返回 0 成功，-1 失败。 */
int wowtype_engine_set_context(WowtypeEngineHandle *engine, const char *text);

/* 用户词库 */
int wowtype_engine_add_user_word(WowtypeEngineHandle *engine, const char *pinyin,
                                 const char *word);
int wowtype_engine_remove_user_word(WowtypeEngineHandle *engine, const char *word);
/* 存用户词库，返回 0 成功，-1 失败。1.7 起空的也写（只有注释行），以前空的时候返回 -1。 */
int wowtype_engine_save_user_dict(WowtypeEngineHandle *engine, const char *path);
int wowtype_engine_load_user_dict(WowtypeEngineHandle *engine, const char *path);
/* 自定义词（设置里添加的，custom.tsv：拼音<TAB>词[<TAB>频次]）：整体替换上一次读到的，
 * 文件不存在就清空；不写回用户词库。返回收下的条数，-1 表示出错。 */
int wowtype_engine_load_custom_words(WowtypeEngineHandle *engine, const char *path);

/* 用户输入习惯：选过的词的热度（30 天半衰期）和词搭配（上一个词 -> 这个词），
 * 参与排序，存成 TSV。save 返回 0 成功；load 返回条数，-1 表示出错。 */
int wowtype_engine_save_history(WowtypeEngineHandle *engine, const char *path);
int wowtype_engine_load_history(WowtypeEngineHandle *engine, const char *path);

/* 输入拼音串；失败返回 NULL。 */
WowtypeCandidates *wowtype_engine_input(WowtypeEngineHandle *engine, const char *input);

/**
 * 基于上一词的预测（预测词库）。
 * 成功返回候选集合（text 为预测词，score 为对数概率），必须 wowtype_candidates_free；
 * 失败返回 NULL，用 wowtype_last_error 取原因。
 */
WowtypeCandidates *wowtype_engine_predict_next(const WowtypeEngineHandle *engine,
                                               const char *prev_word,
                                               size_t limit);

/* 候选数量。 */
size_t wowtype_candidates_len(const WowtypeCandidates *candidates);

/* 候选文本（UTF-8，NUL 结尾）；索引越界返回 NULL。 */
const char *wowtype_candidates_text(const WowtypeCandidates *candidates, size_t index);

/* 候选分数；索引越界返回 0。 */
double wowtype_candidates_score(const WowtypeCandidates *candidates, size_t index);

/* 候选拼音（空格分隔音节，纠错后的写法），可传给 record_selection_ex；
 * 预测/符号结果为空串，越界返回 NULL。 */
const char *wowtype_candidates_pinyin(const WowtypeCandidates *candidates, size_t index);

/* 候选用掉了输入的多少字节。等于输入长度是整句候选；小于时是句首词候选，
 * 上屏后应把 input + consumed 之后的部分留在编辑区继续输入。越界返回 0。 */
size_t wowtype_candidates_consumed(const WowtypeCandidates *candidates, size_t index);

/* 选中后上屏的文字（emoji 推荐可能和显示的不同），其他候选等于 text。越界返回 NULL。 */
const char *wowtype_candidates_commit(const WowtypeCandidates *candidates, size_t index);

/* 释放候选集合。 */
void wowtype_candidates_free(WowtypeCandidates *candidates);

/* 编辑区显示用：按候选拼音（空格分隔，wowtype_candidates_pinyin）给输入分词，
 * hengxiang + "heng xiang" -> heng'xiang。保留用户打的字母（大小写、按错的键）。
 * 切不了（英文、双拼、差得太多）返回 NULL；返回的串在同一线程下一次调用前有效。 */
const char *wowtype_segment_input(const char *input, const char *pinyin);

/* 最近一次错误信息，可能为空串。 */
const char *wowtype_last_error(void);

/* 版本号，例如 0.1.0。 */
const char *wowtype_version(void);

/*
 * 语音输入（核心带 voice 特性编译时才有，默认带）。
 *
 * 识别用微信输入法的离线模型：不随软件分发，由用户在设置界面（wowtype-config）里下载到
 * model_dir；这里只读不下载，第一次按热键时读进内存，之后常驻。没有模型时按热键，
 * poll 返回 WOWTYPE_VOICE_ERROR（文本是去设置里下载的提示），model_state 返回
 * WOWTYPE_VOICE_MODEL_MISSING。所有函数都不阻塞：开麦克风、读模型、解码都在后台线程。
 *
 * 用法：热键按下/松开调 key_down/key_up；录音期间每几十毫秒调一次 poll，
 * 按返回的状态显示提示或上屏 wowtype_voice_text。
 * 按住超过 300ms 松开即结束；更短算点按，继续听到静音 1.5 秒或再按一次热键。
 * 设了环境变量 WOWTYPE_VOICE_WAV 时不开麦克风，改读这个 WAV 文件（测试用）。
 */
typedef struct WowtypeVoice WowtypeVoice;

enum {
    WOWTYPE_VOICE_IDLE = 0,
    WOWTYPE_VOICE_LISTENING = 1,   /* 正在听；文本是中间结果（可能为空） */
    WOWTYPE_VOICE_RECOGNIZING = 2, /* 录完了，正在出最终结果 */
    WOWTYPE_VOICE_RESULT = 3,      /* 最终结果，文本即要上屏的字；只返回一次 */
    WOWTYPE_VOICE_ERROR = 4,       /* 出错或没听清，文本是原因；只返回一次 */
};

enum {
    WOWTYPE_VOICE_MODEL_NONE = 0,
    WOWTYPE_VOICE_MODEL_DOWNLOADING = 1, /* 不再返回：模型改在设置界面里下载 */
    WOWTYPE_VOICE_MODEL_LOADING = 2,
    WOWTYPE_VOICE_MODEL_READY = 3,
    WOWTYPE_VOICE_MODEL_FAILED = 4,
    WOWTYPE_VOICE_MODEL_MISSING = 5, /* 按了热键，但模型还没下载 */
};

/* model_dir 为 NULL/空串时用 ~/.local/share/wowtype/wtlocal。
 * 核心不带 voice 特性编译时返回 NULL（其余函数接受 NULL，什么也不做）。 */
WowtypeVoice *wowtype_voice_new(const char *model_dir);
void wowtype_voice_free(WowtypeVoice *voice);
void wowtype_voice_key_down(WowtypeVoice *voice);
void wowtype_voice_key_up(WowtypeVoice *voice);
/* 丢掉这次说的话（Esc、切走输入框）。 */
void wowtype_voice_cancel(WowtypeVoice *voice);
/* 返回 WOWTYPE_VOICE_*。 */
int wowtype_voice_poll(WowtypeVoice *voice);
/* 上一次 poll 对应的文本；下一次 poll 或释放前有效。 */
const char *wowtype_voice_text(const WowtypeVoice *voice);
/* 正在听时最近的音量，0–100（画波形用）。 */
int wowtype_voice_level(const WowtypeVoice *voice);
/* 返回 WOWTYPE_VOICE_MODEL_*。 */
int wowtype_voice_model_state(const WowtypeVoice *voice);
/* 松开热键后还在听（点按模式）时返回 1。 */
int wowtype_voice_is_tap(const WowtypeVoice *voice);

/*
 * 互译：调用户配置的大模型（OpenAI 格式 chat/completions，流式）把原文译成外文，
 * 原文不是中文时译成简体中文。网络请求交给系统的 curl，在后台线程里跑；
 * 下面的函数都不阻塞。用法：原文变了调 start，之后每几十毫秒 poll 一次，
 * 显示 wowtype_translate_text（流式输出时边写边变）。
 */
typedef struct WowtypeTranslator WowtypeTranslator;

enum {
    WOWTYPE_TRANSLATE_IDLE = 0,
    WOWTYPE_TRANSLATE_RUNNING = 1, /* 请求中；可能已有一部分译文 */
    WOWTYPE_TRANSLATE_DONE = 2,
    WOWTYPE_TRANSLATE_ERROR = 3,   /* wowtype_translate_error 是给用户看的原因 */
};

WowtypeTranslator *wowtype_translate_new(void);
void wowtype_translate_free(WowtypeTranslator *translator);
/* base_url 如 https://api.openai.com/v1；target 是语言代码（en、ja……）；
 * key_file 是放 API Key 的文件（一行），NULL/空串表示不带 Key。字符串都可为 NULL。 */
void wowtype_translate_configure(WowtypeTranslator *translator, const char *base_url,
                                 const char *model, const char *target, int formal,
                                 const char *key_file);
/* 用本地模型（ABI 1.1 起）：model 是 GGUF 文件；server 是 llama-server 的路径或名字（NULL/空串
 * 表示在 PATH 里找）；闲置 idle_secs 秒后停掉（0 表示不停）。model 为 NULL/空串时改回用上面配的
 * 接口，并马上停掉本地服务。llama-server 第一次翻译时才启动，只监听 127.0.0.1。 */
void wowtype_translate_configure_local(WowtypeTranslator *translator, const char *model,
                                       const char *server, unsigned int idle_secs);
/* 本地模型的 llama-server 正在跑时返回 1（ABI 1.1 起）。 */
int wowtype_translate_local_running(const WowtypeTranslator *translator);
/* 能翻译时返回 1：填了地址和模型，或者用本地模型且模型文件在。 */
int wowtype_translate_ready(const WowtypeTranslator *translator);
/* 开始翻译 text，取消别的请求；正在译或已经译好同一句话时什么也不做。 */
void wowtype_translate_start(WowtypeTranslator *translator, const char *text);
void wowtype_translate_cancel(WowtypeTranslator *translator);
/* 拍一份当前结果的快照，返回 WOWTYPE_TRANSLATE_*。下面取文字的函数读的都是这份快照，
 * 指针在下一次 poll 前有效。 */
int wowtype_translate_poll(WowtypeTranslator *translator);
/* 到目前的译文；状态是 RUNNING 时可能还没写完，没有就是空串。 */
const char *wowtype_translate_text(const WowtypeTranslator *translator);
/* 快照对应的原文。 */
const char *wowtype_translate_source(const WowtypeTranslator *translator);
const char *wowtype_translate_error(const WowtypeTranslator *translator);
/* 面板上的方向标签（「中→英」「英→中」），按 text 判断；下一次调用前有效。 */
const char *wowtype_translate_tag(WowtypeTranslator *translator, const char *text);

/* ---- 输入会话（ABI 1.3 起，1.4 加候选和分音节，1.5 加选词和学习，1.6 加选词键、翻页和展开面板，
 * 1.7 加 Ctrl+Delete 忘掉学错的词；
 * 见 docs/PORTING.md、docs/ABI.md）----
 *
 * 一个输入框一个会话：外壳把平台按键翻成 WowtypeKey 交给会话，按返回的 WowtypeFrame
 * 画编辑区、上屏。会话只能在一个线程里用。
 *
 * 暂定：输入行为还在一块一块从 fcitx5 外壳挪进来（docs/PORTING.md 第 2 步），挪完之前
 * 这组函数不在「只加不删」的承诺里。标了「过渡」的函数挪完就删；现在 handled = 0 的意思是
 * 「会话还不管这个键」，外壳接着用自己的逻辑处理（挪完以后就是「交给应用」）。
 *
 * 已经由会话处理的键：字母、隔音符（编辑区不空时）、退格（先撤销最后一次选词，没有选词
 * 才删字母）、Esc（清空）。候选列表（引擎候选 + 计算器、emoji 推荐、数字按键 emoji、日期时间）
 * 和编辑区分音节也由会话算（1.4 起），选词、上屏和学习也在会话里（1.5 起）；空格、回车、数字、
 * 分号引号选词、V 模式算式、双拼分号、翻页、移动候选、展开面板也由会话处理（1.6 起）。
 * 打字途中的 Ctrl+Delete、Ctrl+退格、Shift+Delete 忘掉光标处学过的词（1.7 起，frame->forgot）：
 * 带 Ctrl 的这几个键也要交给会话。
 * 还在外壳里的：打字途中的标点（先上屏再出标点）、工具栏、表情面板、互译。 */

typedef struct WowtypeSession WowtypeSession;

/* 修饰键位掩码（和 X11 / fcitx5 的 KeyState 取值一样）。 */
#define WOWTYPE_MOD_SHIFT (1u << 0)
#define WOWTYPE_MOD_CAPS (1u << 1)
#define WOWTYPE_MOD_CTRL (1u << 2)
#define WOWTYPE_MOD_ALT (1u << 3)
#define WOWTYPE_MOD_NUMLOCK (1u << 4) /* 1.6 起：和 CapsLock 一样不影响数字选词 */
#define WOWTYPE_MOD_SUPER (1u << 6)

/* 平台自己的翻页、移动候选热键（fcitx5 全局设置里的 [Hotkey/PrevPage] 等）或候选框上的
 * 翻页按钮：外壳翻成动作放进 WowtypeKey.action（1.6 起）。方向键带了动作也照横排竖排的规则走。
 * keysym 可以是 0（只有动作）。没在打字时动作交给应用。 */
#define WOWTYPE_KEY_ACTION_NONE 0
#define WOWTYPE_KEY_ACTION_PREV_PAGE 1
#define WOWTYPE_KEY_ACTION_NEXT_PAGE 2
#define WOWTYPE_KEY_ACTION_PREV_CANDIDATE 3
#define WOWTYPE_KEY_ACTION_NEXT_CANDIDATE 4

/* 一次按键。keysym 是 XKB keysym 的子集（字母、数字、标点、方向、功能键）。
 * size 填 sizeof(WowtypeKey)：以后在末尾加字段，旧外壳没填的字段当 0。 */
typedef struct {
    uint32_t size;
    uint32_t keysym;
    uint32_t modifiers; /* WOWTYPE_MOD_* */
    uint32_t release;   /* 0 按下，1 松开 */
    uint32_t prev_char; /* 光标前一个字符（Unicode 码位），不知道给 0 */
    uint32_t action;    /* WOWTYPE_KEY_ACTION_*（1.6 起） */
} WowtypeKey;

/* 设置里能选的翻页键（WowtypeSessionConfig.page_keys 的位掩码）。PgUp PgDn 一直能用；
 * 横排时上下键翻页、竖排时左右键翻页。 */
#define WOWTYPE_PAGE_KEYS_MINUS (1u << 0)   /* - = */
#define WOWTYPE_PAGE_KEYS_BRACKET (1u << 1) /* [ ] */
#define WOWTYPE_PAGE_KEYS_COMMA (1u << 2)   /* , . */
#define WOWTYPE_PAGE_KEYS_TAB (1u << 3)     /* Shift+Tab Tab */

/* 分号、引号选词（WowtypeSessionConfig.select_keys）。 */
#define WOWTYPE_SELECT_KEYS_OFF 0
#define WOWTYPE_SELECT_KEYS_SEMICOLON 1       /* 分号选第 2 个 */
#define WOWTYPE_SELECT_KEYS_SEMICOLON_QUOTE 2 /* 再加引号选第 3 个 */

/* 会话的设置。size 填 sizeof(WowtypeSessionConfig)；外壳没填的字段（size 短）用默认值。
 * 传 NULL 全用默认。双拼开不开看引擎（wowtype_engine_set_shuangpin）。 */
typedef struct {
    uint32_t size;
    uint32_t page_size; /* 一页几个候选，0 用默认（9）；数字按键 emoji 只在第一页里找 */
    int calculator;     /* V 模式计算器，默认 1（双拼时不开） */
    int date_time;      /* 日期时间候选，默认 1 */
    int emoji_suggest;  /* 打字推荐 emoji，默认 1 */
    int horizontal;     /* 1.6 起：候选横排（左右键移光标、上下键翻页、下键展开面板），默认 1；
                         * 0 竖排（上下键移光标、左右键翻页） */
    uint32_t page_keys; /* 1.6 起：WOWTYPE_PAGE_KEYS_*，默认 WOWTYPE_PAGE_KEYS_MINUS */
    int select_keys;    /* 1.6 起：WOWTYPE_SELECT_KEYS_*，默认 OFF */
    int pick_char;      /* 1.8 起：以词定字，打拼音途中 [ 上屏光标处候选的第一个字、] 最后一个字，
                         * 默认 1；page_keys 带 [ ] 时让给翻页 */
} WowtypeSessionConfig;

/* 一次调用之后外壳要做的事。归会话所有，和里面的字符串一起在同一个会话的下一次调用前有效。
 * 读 size 之外的字段前先看 size（旧核心给的结构体短）。 */
typedef struct {
    uint32_t size;
    int handled;           /* 会话处理了这个键；0 见上面「暂定」 */
    const char *commit;    /* 要上屏的字，没有为 NULL */
    const char *preedit;   /* 编辑区文字（不为 NULL）：已选的词 + 还没选的输入（分好音节，
                            * nihao 显示成 ni'hao，双拼显示成全拼；没有引擎时是原始输入） */
    int preedit_cursor;    /* 编辑区光标，UTF-8 字节偏移 */
    const char *aux;       /* 候选框上方的提示，没有为 NULL（还没用） */
    int panel;             /* 0 普通一页 1 展开面板（横排按下键，6 列，一屏 6 行）；2 表情面板还没用 */
    size_t cand_count;     /* 候选框里显示几个（一页，或展开面板的一屏）；用 wowtype_session_candidate 取 */
    int cand_cursor;       /* 光标在显示的这些里的位置；-1 不在这一页上（翻页按钮翻走以后） */
    int has_prev, has_next; /* 前面 / 后面还有一页（展开面板是一屏） */
    uint32_t wake_after_ms; /* >0：这么久后再调会话（还没用，恒为 0） */
    int forgot;             /* 1.7 起：这次按键（Ctrl+Delete）让引擎忘掉了一个学过的词，
                             * 外壳该马上存用户词库和输入习惯（和上屏后存的一样） */
} WowtypeFrame;

/* 建会话。engine 可以传 NULL（词库没载入好时也能打字，载入好了用 set_engine 补上）；
 * 传了的话要比会话活得久。config 可以传 NULL。 */
WowtypeSession *wowtype_session_new(WowtypeEngineHandle *engine, const WowtypeSessionConfig *config);
void wowtype_session_free(WowtypeSession *session);
/* 换引擎，可以传 NULL。 */
void wowtype_session_set_engine(WowtypeSession *session, WowtypeEngineHandle *engine);
/* 改设置；config 为 NULL 全用默认。 */
void wowtype_session_configure(WowtypeSession *session, const WowtypeSessionConfig *config);
/* 现在的编辑区（不处理按键，handled 为 0）。session 为 NULL 时返回 NULL。 */
const WowtypeFrame *wowtype_session_frame(WowtypeSession *session);
/* 处理一次按键（按下和松开都给）。session 或 key 为 NULL 时返回 NULL。 */
const WowtypeFrame *wowtype_session_key(WowtypeSession *session, const WowtypeKey *key);
/* 清空编辑区；commit_pending 非 0 时，已选的词 + 原始输入放进 frame->commit（切换输入法、
 * 打字途中切英文时，和回车一样），原始输入也记一次选词。 */
const WowtypeFrame *wowtype_session_reset(WowtypeSession *session, int commit_pending);
/* 点候选（1.6 起）：index 是显示的这些（frame 的 cand_count 个）里的序号。和按数字键一样：
 * 句首词进已选栈，用完输入就上屏并记选词。序号不对时 handled 为 0。 */
const WowtypeFrame *wowtype_session_select(WowtypeSession *session, size_t index);
/* 显示的第 index 个候选的字（1.6 起），index < cand_count，不然返回 NULL。
 * 在同一会话下一次调用前有效。 */
const char *wowtype_session_candidate(WowtypeSession *session, size_t index);

/* 过渡：选词键、标点等还在外壳时，外壳用这些读写编辑区、选词。返回的字符串在同一会话下一次调用前有效。 */
const char *wowtype_session_input(WowtypeSession *session);         /* 全部原始输入 */
const char *wowtype_session_remaining(WowtypeSession *session);     /* 还没选词的部分 */
const char *wowtype_session_selected_text(WowtypeSession *session); /* 已选的词连起来 */
/* 选第 index 个候选（全部候选里的序号，不分页）：emoji、计算结果、日期时间原样上屏不学习；
 * 句首词进已选栈、记一次选词；其余已选的词 + 它上屏，记选词（分几次选出来的整句也记一次）。
 * 序号不对时 handled 为 0。外壳打字途中按标点时，用它把当前候选上屏。 */
const WowtypeFrame *wowtype_session_choose(WowtypeSession *session, size_t index);
/* 往原始输入后面加字（计算器算式、双拼分号、表情面板搜索词）。 */
void wowtype_session_append(WowtypeSession *session, const char *text);
/* 光标前已经上屏的字（选了开头的词以后，拿它 + 已选的词当上文给剩下的部分排序）。 */
void wowtype_session_set_context(WowtypeSession *session, const char *text);

/* 过渡：选词键、翻页还在外壳时，外壳把整个候选列表（不分页）抄过去画候选框。 */
#define WOWTYPE_CAND_WORD 0  /* 引擎的候选：选中要学习 */
#define WOWTYPE_CAND_PLAIN 1 /* 计算结果、日期时间、带换行的自定义词：原样上屏，不学习 */
#define WOWTYPE_CAND_EMOJI 2 /* emoji：上屏 commit，不学习 */
typedef struct {
    uint32_t size;      /* 填 sizeof(WowtypeCandidateInfo) */
    const char *text;   /* 候选框里显示的字 */
    const char *pinyin; /* 空格分音节；计算结果、emoji 这类为空串 */
    size_t consumed;    /* 用掉了还没选词那部分输入的多少字节；比它短是句首词 */
    int kind;           /* WOWTYPE_CAND_* */
    const char *commit; /* 选中时上屏的字（emoji 推荐在句尾对上时是「整句 + emoji」） */
    const char *emoji;  /* emoji 本身；不是 emoji 为空串 */
} WowtypeCandidateInfo;
/* 候选个数。没有引擎、没在打字时为 0。 */
size_t wowtype_session_candidate_total(WowtypeSession *session);
/* 第 index 个候选写进 out（按 out->size 写）。返回 0 成功，-1 越界或参数为 NULL。
 * 字符串在同一会话下一次改了编辑区、上文、引擎或设置的调用前有效。 */
int wowtype_session_candidate_info(WowtypeSession *session, size_t index, WowtypeCandidateInfo *out);
/* 过渡（1.6 起）：上一次调用（_key、_select、_choose）上屏的是哪个候选，写进 out。
 * 返回 0 有，-1 这次上屏的不是候选（回车、没候选时的空格）或没上屏。外壳要它把上屏的 emoji
 * 记进最近用过、看这次有没有学到新词。 */
int wowtype_session_last_choice(WowtypeSession *session, WowtypeCandidateInfo *out);

/* 过渡（1.6 起）：fcitx5 外壳的候选框（CommonCandidateList）要整个列表和页码、光标，
 * 用它和 candidate_info 抄。 */
typedef struct {
    uint32_t size;        /* 填 sizeof(WowtypeSessionView) */
    size_t page;          /* 普通候选框在第几页 */
    size_t cursor;        /* 普通候选框的光标（全部候选里的序号，可以不在这一页上） */
    int expanded;         /* 展开面板开着 */
    size_t grid_cursor;   /* 展开面板的光标（全部候选里的序号） */
    uint64_t generation;  /* 候选重算过几次：变了就重新抄候选 */
} WowtypeSessionView;
/* 返回 0 成功，-1 参数为 NULL。 */
int wowtype_session_view(WowtypeSession *session, WowtypeSessionView *out);

#ifdef __cplusplus
}
#endif

#endif /* WOWTYPE_CORE_H */
