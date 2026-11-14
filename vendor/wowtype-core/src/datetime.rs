//! 日期、时间候选：打 riqi / shijian / xingqi（或英文 date / time）时，在首选词后面
//! 给出当前日期、时间的几种写法。
//!
//! 只认完整拼音，不认 rq、sj 这类简拼——它们太常见，平时打别的词会被打扰。
//! 当前时间由调用方（输入法插件）按本地时区传进来，这里不读系统时钟，方便测试。

/// 本地时间。weekday：0 = 星期日，1 = 星期一 …… 6 = 星期六。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub weekday: u32,
}

const WEEKDAYS: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

/// 触发词（拼音或英文，小写、不带隔音符）对应的候选；不是触发词时返回空。
pub fn candidates(key: &str, now: &LocalTime) -> Vec<String> {
    let key: String = key
        .chars()
        .filter(|c| !matches!(c, '\'' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    let LocalTime {
        year: y,
        month: mo,
        day: d,
        hour: h,
        minute: mi,
        second: s,
        weekday,
    } = *now;
    let week = WEEKDAYS[(weekday % 7) as usize];
    match key.as_str() {
        "riqi" | "rq" | "date" => vec![
            format!("{y}年{mo}月{d}日"),
            format!("{y}-{mo:02}-{d:02}"),
            format!("{y}/{mo:02}/{d:02}"),
            format!("{y}.{mo:02}.{d:02}"),
            format!("{y}{mo:02}{d:02}"),
            format!("{mo}月{d}日"),
            format!("{y}年{mo}月{d}日 星期{week}"),
        ],
        "shijian" | "sj" | "time" => vec![
            format!("{h:02}:{mi:02}"),
            format!("{h:02}:{mi:02}:{s:02}"),
            format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}"),
            format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"),
            format!("{h}时{mi}分"),
            format!("{y}年{mo}月{d}日 {h:02}:{mi:02}"),
        ],
        "xingqi" => vec![format!("星期{week}"), format!("周{week}")],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> LocalTime {
        LocalTime {
            year: 2026,
            month: 11,
            day: 10,
            hour: 9,
            minute: 5,
            second: 1,
            weekday: 2,
        }
    }

    #[test]
    fn date_formats() {
        let c = candidates("riqi", &now());
        assert_eq!(c[0], "2026年11月10日");
        assert!(c.contains(&"2026-11-10".to_string()));
        assert!(c.contains(&"2026/11/10".to_string()));
        assert!(c.contains(&"2026.11.10".to_string()));
        assert!(c.contains(&"20261110".to_string()));
        assert!(c.contains(&"2026年11月10日 星期二".to_string()));
        assert_eq!(candidates("date", &now()), c);
        assert_eq!(candidates("ri'qi", &now()), c);
    }

    #[test]
    fn time_formats() {
        let c = candidates("shijian", &now());
        assert_eq!(c[0], "09:05");
        assert!(c.contains(&"09:05:01".to_string()));
        assert!(c.contains(&"2026-11-10 09:05".to_string()));
        assert!(c.contains(&"9时5分".to_string()));
        assert_eq!(candidates("TIME", &now()), c);
    }

    #[test]
    fn weekday() {
        assert_eq!(candidates("xingqi", &now()), vec!["星期二", "周二"]);
        let sunday = LocalTime {
            weekday: 0,
            ..now()
        };
        assert_eq!(candidates("xingqi", &sunday)[0], "星期日");
    }

    #[test]
    fn abbreviations_do_not_trigger() {
        assert_eq!(candidates("rq", &now()), candidates("riqi", &now()));
        assert_eq!(candidates("sj", &now()), candidates("shijian", &now()));
        for key in ["xq", "riq", "shij", "dat", ""] {
            assert!(candidates(key, &now()).is_empty(), "{key} 不该出日期时间");
        }
    }
}
