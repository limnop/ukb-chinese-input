use super::*;
use std::path::PathBuf;

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wowtype-stats-{}-{}-{}",
        std::process::id(),
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.join("stats.json")
}

fn local_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month as i32 - 1;
    tm.tm_mday = day as i32;
    tm.tm_hour = hour as i32;
    tm.tm_min = minute as i32;
    tm.tm_sec = second as i32;
    tm.tm_isdst = -1;
    let time = unsafe { libc::mktime(&mut tm) };
    time as i64 * 1000
}

/// 打开统计的记录器。
fn recorder(path: &Path, now: i64) -> Recorder {
    let mut recorder = Recorder::new(path);
    recorder.set_enabled(true, now);
    recorder
}

/// 交出去并等写完，再读文件。
fn saved(recorder: &mut Recorder, path: &Path, now: i64) -> Document {
    recorder.flush(now);
    recorder.wait(None).unwrap();
    Document::load(path).unwrap()
}

#[test]
fn classifies_characters() {
    let (count, kinds) = classify("你好 abc 123，。😀👍");
    assert_eq!(count, 12);
    assert_eq!(kinds.han, 2);
    assert_eq!(kinds.latin, 3);
    assert_eq!(kinds.number, 3);
    assert_eq!(kinds.punct, 2);
    assert_eq!(kinds.emoji, 2);
    assert_eq!(kinds.readable(), 5);
}

#[test]
fn whitespace_is_not_counted() {
    let (count, kinds) = classify("  \n\t ");
    assert_eq!(count, 0);
    assert_eq!(kinds, Kinds::default());
}

#[test]
fn joined_emoji_counts_once() {
    let (count, kinds) = classify("👨\u{200d}👩\u{200d}👧");
    assert_eq!(count, 1);
    assert_eq!(kinds.emoji, 1);
    // 没有连接符的两个表情仍然算两个。
    let (count, kinds) = classify("😀😀");
    assert_eq!(count, 2);
    assert_eq!(kinds.emoji, 2);
    // 变体选择符不另算。
    let (count, kinds) = classify("❤\u{fe0f}");
    assert_eq!(count, 1);
    assert_eq!(kinds.emoji, 1);
}

#[test]
fn key_ids_cover_both_platforms() {
    assert_eq!(key_id(0, 8 + 2), Some("Digit1"));
    assert_eq!(key_id(0, 8 + 30), Some("KeyA"));
    assert_eq!(key_id(0, 8 + 57), Some("Space"));
    assert_eq!(key_id(0, 8 + 464), Some("Fn"));
    assert_eq!(key_id(1, 0x12), Some("Digit1"));
    assert_eq!(key_id(1, 0x00), Some("KeyA"));
    assert_eq!(key_id(1, 0x7e), Some("ArrowUp"));
    assert_eq!(key_id(0, 8 + 200), None);
    assert_eq!(key_id(1, 0xff), None);
    assert_eq!(key_id(2, 1), None);
    // 低于 8 的 X11 键码没有对应的物理键。
    assert_eq!(key_id(0, 3), None);
}

#[test]
fn switched_off_counts_nothing_and_writes_nothing() {
    let path = temp_path("off");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = Recorder::new(&path);
    assert!(!recorder.enabled(), "默认关");
    recorder.key(0, 8 + 30, true, 0, now);
    recorder.key(1, 0x00, true, 0, now);
    recorder.commit("你好", "quanpin", "", 1, 0, now);
    assert!(recorder.pending.is_empty(), "关着时内存里也不记");
    assert_eq!(recorder.last_commit_ms, 0, "关着时不记上屏时刻");
    assert!(recorder.held.is_empty());
    recorder.flush(now);
    recorder.wait(None).unwrap();
    assert!(recorder.writer.handle.is_none(), "关着时不起写文件线程");
    drop(recorder);
    assert!(!path.exists(), "关着时不建文件");
    assert!(!clear_marker_path(&path).exists());
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 0, "目录里什么都没有");
}

#[test]
fn switching_off_drops_unsaved_counts() {
    let path = temp_path("switch-off");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.commit("你好", "quanpin", "", 1, 0, now);
    recorder.key(0, 8 + 30, true, 0, now);
    recorder.set_enabled(false, now);
    assert!(recorder.pending.is_empty());
    assert_eq!(recorder.last_commit_ms, 0);
    recorder.commit("世界", "quanpin", "", 1, 0, now + 1_000);
    recorder.flush(now + 1_000);
    recorder.wait(None).unwrap();
    drop(recorder);
    assert!(!path.exists());
}

#[test]
fn only_aggregates_are_written() {
    let path = temp_path("aggregates");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.key(0, 8 + 35, true, 0, now); // h
    recorder.key(0, 8 + 35, false, 0, now + 50);
    recorder.commit("秘密口令 hunter2", "quanpin", "xiaohe", 2, 0, now + 1_000);
    recorder.commit("下一句", "evil source 内容", "<script>", 0, 0, now + 2_000);
    let document = saved(&mut recorder, &path, now);
    let text = fs::read_to_string(&path).unwrap();
    for secret in ["秘密", "口令", "hunter", "下一句", "evil", "内容", "script"] {
        assert!(!text.contains(secret), "文件里不能有输入内容：{secret}");
    }
    assert!(!text.contains("lastCommitMs") && !text.contains("enabled"));
    // 只有这几个顶层键
    let value: Value = serde_json::from_str(&text).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["cleared", "days", "version"]);
    let day = &document.days[&local_day(now)];
    assert_eq!(day.chars, 4 + 6 + 1 + 3);
    assert_eq!(day.kinds.han, 7);
    assert_eq!(day.kinds.latin, 6);
    assert_eq!(day.kinds.number, 1);
    assert_eq!(day.keys.len(), 1);
    assert_eq!(day.keys["KeyH"], 1);
    assert_eq!(day.sources.len(), 1, "不认识的模式名不记");
    assert_eq!(day.sources["quanpin"], 11);
    assert_eq!(day.schemes.len(), 1, "不是字母数字的方案名不记");
    assert_eq!(day.selections[1], 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn sensitive_fields_are_never_counted() {
    let path = temp_path("sensitive");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.key(0, 8 + 30, true, SENSITIVE, now);
    recorder.key(0, 8 + 30, false, SENSITIVE, now + 10);
    recorder.key(1, 0x00, true, SENSITIVE, now);
    recorder.commit("password", "english", "", 0, SENSITIVE, now);
    assert!(recorder.pending.is_empty());
    assert_eq!(recorder.last_commit_ms, 0, "密码框里的上屏也不算进活跃时长");
    recorder.flush(now);
    recorder.wait(None).unwrap();
    assert!(!path.exists());
}

#[test]
fn large_keycodes_do_not_panic() {
    let path = temp_path("keycodes");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    // 以前用一个 u64 当位图，键码超过 63 在 debug 下 panic（extern "C" 里直接 abort 输入法）
    for code in [64, 65, 100, 127, 255, 472, 511, 512, 4096, u32::MAX] {
        recorder.key(0, code, true, 0, now);
        recorder.key(0, code, true, 0, now + 1);
        recorder.key(0, code, false, 0, now + 5);
        recorder.key(1, code, true, 0, now);
    }
    // 键码不会互相顶掉：以前 release 下 1 << 108 绕回成 1 << 44，按着右 Alt 时 J 不算
    recorder.key(0, 8 + 100, true, 0, now + 100); // AltRight（108）
    recorder.key(0, 8 + 36, true, 0, now + 100); // KeyJ（44）
    let document = saved(&mut recorder, &path, now);
    let day = &document.days[&local_day(now)];
    assert_eq!(day.keys["Fn"], 1, "8 + 464 长按只算一次");
    assert_eq!(day.keys["AltRight"], 1);
    assert_eq!(day.keys["KeyJ"], 1);
}

#[test]
fn long_press_counts_once() {
    let path = temp_path("hold");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.key(0, 8 + 30, true, 0, now);
    recorder.key(0, 8 + 30, true, 0, now + 30);
    recorder.key(0, 8 + 30, true, 0, now + 60);
    let document = saved(&mut recorder, &path, now);
    let day = &document.days[&local_day(now)];
    assert_eq!(day.keys.get("KeyA"), Some(&1));
    assert_eq!(day.keys.len(), 1);
}

#[test]
fn synthetic_repeat_after_release_is_ignored() {
    let path = temp_path("repeat");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.key(0, 8 + 30, true, 0, now);
    recorder.key(0, 8 + 30, false, 0, now + 1);
    recorder.key(0, 8 + 30, true, 0, now + 2); // X11 合成的重复
    recorder.key(0, 8 + 30, false, 0, now + 3);
    recorder.key(0, 8 + 30, true, 0, now + 100); // 真正又按了一次
    let document = saved(&mut recorder, &path, now);
    assert_eq!(document.days[&local_day(now)].keys["KeyA"], 2);
}

#[test]
fn active_time_uses_ten_second_gap() {
    let path = temp_path("active");
    let start = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, start);
    recorder.commit("你好", "quanpin", "", 0, 0, start);
    recorder.commit("世界", "quanpin", "", 0, 0, start + 5_000);
    recorder.commit("再见", "quanpin", "", 0, 0, start + 5_000 + 10_001);
    let document = saved(&mut recorder, &path, start);
    assert_eq!(document.days[&local_day(start)].active_ms, 5_000);
}

#[test]
fn selection_positions_are_counted() {
    let path = temp_path("selection");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.commit("甲", "quanpin", "", 1, 0, now);
    recorder.commit("乙", "quanpin", "", 3, 0, now + 1_000);
    recorder.commit("丙", "quanpin", "", 12, 0, now + 2_000);
    let document = saved(&mut recorder, &path, now);
    let day = &document.days[&local_day(now)];
    assert_eq!(day.selections[0], 1);
    assert_eq!(day.selections[2], 1);
    assert_eq!(day.beyond, 1);
    assert_eq!(day.chars, 3);
}

#[test]
fn sources_and_schemes_count_characters() {
    let path = temp_path("sources");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.commit("你好", "shuangpin", "XiaoHe", 0, 0, now);
    recorder.commit("ab", "english", "", 0, 0, now + 1_000);
    let document = saved(&mut recorder, &path, now);
    let day = &document.days[&local_day(now)];
    assert_eq!(day.sources["shuangpin"], 2);
    assert_eq!(day.sources["english"], 2);
    assert_eq!(day.schemes["xiaohe"], 2);
}

#[test]
fn counts_accumulate_across_writes_and_restarts() {
    let path = temp_path("accumulate");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut first = recorder(&path, now);
    first.commit("你好", "quanpin", "", 0, 0, now);
    saved(&mut first, &path, now);
    drop(first);
    let mut second = recorder(&path, now);
    second.commit("世界", "quanpin", "", 0, 0, now + 60_000);
    let document = saved(&mut second, &path, now + 60_000);
    assert_eq!(document.days[&local_day(now)].chars, 4);
}

#[test]
fn many_commits_are_handed_to_the_writer_without_explicit_flush() {
    let path = temp_path("auto");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    for i in 0..FLUSH_COMMITS {
        recorder.commit("字", "quanpin", "", 1, 0, now + i as i64);
    }
    assert!(recorder.pending.is_empty(), "攒够了交给写文件线程");
    recorder.wait(None).unwrap();
    assert_eq!(Document::load(&path).unwrap().days[&local_day(now)].chars, FLUSH_COMMITS as u64);
}

#[test]
fn clear_removes_file_and_drops_counts_collected_before_it() {
    let path = temp_path("clear");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.commit("旧", "quanpin", "", 1, 0, now);
    assert_eq!(saved(&mut recorder, &path, now).days.len(), 1);

    // 设置界面清空，输入法这时还攒着清空之前打的
    recorder.commit("还攒着", "quanpin", "", 1, 0, now + 1_000);
    clear(&path).unwrap();
    assert!(!path.exists());
    assert_eq!(read_clear_marker(&path), 1);
    recorder.flush(now + 1_000);
    recorder.wait(None).unwrap();
    assert!(!path.exists(), "攒着的旧计数整批丢掉");

    // 之后新打的照常记，文件记下这次的清空计数
    recorder.commit("新的", "quanpin", "", 1, 0, now + 2_000);
    let document = saved(&mut recorder, &path, now + 2_000);
    assert_eq!(document.days[&local_day(now)].chars, 2);
    assert_eq!(document.cleared, 1);
    assert_eq!(load_current(&path), document);

    // 再清空一次：计数是递增的整数，不会撞上
    clear(&path).unwrap();
    assert_eq!(read_clear_marker(&path), 2);
    assert!(load_current(&path).days.is_empty());
}

#[test]
fn stale_file_written_back_during_clear_is_ignored_and_removed() {
    let path = temp_path("race");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    recorder.commit("旧", "quanpin", "", 1, 0, now);
    saved(&mut recorder, &path, now);
    // 模拟清空的同时输入法把旧的写了回去：文件还在，但 cleared 对不上
    let stale = fs::read(&path).unwrap();
    clear(&path).unwrap();
    fs::write(&path, stale).unwrap();
    assert!(load_current(&path).days.is_empty(), "设置界面当它是空的");
    recorder.commit("又打", "quanpin", "", 1, 0, now + 1_000);
    recorder.flush(now + 1_000);
    recorder.wait(None).unwrap();
    assert!(!path.exists(), "写文件线程发现清空过，删掉旧的");
}

#[test]
fn clear_while_switched_off_still_deletes() {
    let path = temp_path("clear-off");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut on = recorder(&path, now);
    on.commit("你好", "quanpin", "", 1, 0, now);
    saved(&mut on, &path, now);
    on.set_enabled(false, now);
    clear(&path).unwrap();
    assert!(!path.exists());
    // 再打开：从清空以后的计数开始
    on.set_enabled(true, now);
    on.commit("世界", "quanpin", "", 1, 0, now + 1_000);
    let document = saved(&mut on, &path, now + 1_000);
    assert_eq!(document.days[&local_day(now)].chars, 2);
}

#[test]
fn old_days_are_pruned() {
    let path = temp_path("prune");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut document = Document::default();
    document.days.insert("2020-01-01".to_string(), Day { chars: 5, ..Day::default() });
    document.days.insert(local_day(now - 10 * DAY_MS), Day { chars: 7, ..Day::default() });
    fs::write(&path, document.to_bytes()).unwrap();
    let mut recorder = recorder(&path, now);
    recorder.commit("字", "quanpin", "", 0, 0, now);
    let saved = saved(&mut recorder, &path, now);
    assert!(!saved.days.contains_key("2020-01-01"));
    assert_eq!(saved.days[&local_day(now - 10 * DAY_MS)].chars, 7);
    assert_eq!(saved.days.len(), 2);
}

#[test]
fn damaged_or_oversized_file_is_moved_aside() {
    let path = temp_path("bad");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    fs::write(&path, b"{not json").unwrap();
    let mut recorder = recorder(&path, now);
    recorder.commit("字", "quanpin", "", 0, 0, now);
    let document = saved(&mut recorder, &path, now);
    assert_eq!(document.days[&local_day(now)].chars, 1);
    assert_eq!(fs::read(bad_path(&path)).unwrap(), b"{not json");

    // 超过 16 MB 的不整个读进来
    let big = vec![b' '; MAX_FILE_BYTES + 1];
    fs::write(&path, &big).unwrap();
    assert!(matches!(Document::load(&path), Err(LoadError::Bad(_))));
    recorder.commit("字", "quanpin", "", 0, 0, now + 1);
    let document = saved(&mut recorder, &path, now + 1);
    assert_eq!(document.days[&local_day(now)].chars, 1);
    assert_eq!(fs::metadata(bad_path(&path)).unwrap().len(), big.len() as u64);
    clear(&path).unwrap();
    assert!(!bad_path(&path).exists(), "清空也删掉挪开的那份");
}

#[test]
fn oversized_output_drops_oldest_days() {
    let mut document = Document::default();
    let mut day = Day::default();
    for i in 0..MAX_MAP_ENTRIES {
        day.keys.insert(format!("K{i:030}"), MAX_COUNT);
    }
    let one = day.to_json().to_string().len();
    let needed = MAX_FILE_BYTES / one + 10;
    for i in 0..needed {
        document.days.insert(format!("{:04}-01-01", 1000 + i), day.clone());
    }
    let bytes = document.to_bytes_capped();
    assert!(bytes.len() <= MAX_FILE_BYTES);
    assert!(!document.days.contains_key("1000-01-01"), "从最早的一天删起");
    assert!(document.days.contains_key(&format!("{:04}-01-01", 1000 + needed - 1)));
}

#[test]
fn document_round_trips_through_json() {
    let mut document = Document::default();
    let day = Day {
        chars: 12,
        kinds: Kinds {
            han: 10,
            latin: 1,
            punct: 1,
            ..Kinds::default()
        },
        active_ms: 3_000,
        ..Day::default()
    };
    document.days.insert("2026-11-14".to_string(), day);
    document.cleared = 9;
    let bytes = document.to_bytes();
    let parsed = Document::from_bytes(&bytes).unwrap();
    assert_eq!(parsed, document);
}

#[test]
fn damaged_days_and_names_are_skipped() {
    let value = br#"{"version":2,"days":{"2026-11-14":{"chars":2,"keys":{"KeyA":3,"some text":4}},"yesterday":{"chars":9}}}"#;
    let document = Document::from_bytes(value).unwrap();
    assert_eq!(document.days.len(), 1);
    assert_eq!(document.days["2026-11-14"].chars, 2);
    assert_eq!(document.days["2026-11-14"].keys.len(), 1);
}

#[test]
fn local_day_has_expected_shape() {
    let day = local_day(local_ms(2026, 11, 14, 0, 0, 1));
    assert_eq!(day.len(), 10);
    assert!(is_valid_day(&day));
}

#[test]
fn macos_presses_count_every_time() {
    let path = temp_path("macos");
    let now = local_ms(2026, 11, 14, 10, 0, 0);
    let mut recorder = recorder(&path, now);
    // InputMethodKit 只送按下，没有松开：连续两次按下都要算。
    recorder.key(1, 0x00, true, 0, now);
    recorder.key(1, 0x00, true, 0, now + 1);
    recorder.key(1, 0x00, false, 0, now + 2);
    let document = saved(&mut recorder, &path, now);
    assert_eq!(document.days[&local_day(now)].keys["KeyA"], 2);
}
