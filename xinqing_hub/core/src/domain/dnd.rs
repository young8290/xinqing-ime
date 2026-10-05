//! 勿扰的判断（05 FR-CMF-01 第 5 条、06 FR-RST 的勿扰）：勿扰应用列表与安静时段。
//!
//! 两样都是用户可改的设置（`care.dnd_apps`、`care.quiet_hours`，ADR 0019）；这里只有解析、校验和判断，
//! 读设置、拿前台应用在调用方。前台全屏与系统专注助手由外壳探测，不在这里。

use chrono::{NaiveTime, Timelike};

/// 出厂的勿扰应用（FR-CMF-01 第 5 条），也是 `care.dnd_apps` 的默认值。PowerPoint 放映属于全屏，由外壳的全屏探测覆盖。
pub const DEFAULT_APPS: &[&str] = &["wemeetapp.exe", "Zoom.exe", "ms-teams.exe"];
/// 一个进程名最多多少字符。
pub const APP_MAX_CHARS: usize = 64;

/// 应用是否在勿扰列表里：按进程名比较，忽略大小写。
pub fn is_dnd_app<S: AsRef<str>>(app: &str, list: &[S]) -> bool {
    list.iter().any(|d| d.as_ref().eq_ignore_ascii_case(app))
}

/// 进程名是否合法：去掉首尾空白后非空、不超过 [`APP_MAX_CHARS`] 字，不含路径分隔符和控制字符。
/// 只认进程名（`Zoom.exe`），不认路径：XQP `focus` 事件里只有进程名。
pub fn valid_app(s: &str) -> bool {
    let t = s.trim();
    t == s
        && !t.is_empty()
        && t.chars().count() <= APP_MAX_CHARS
        && !t
            .chars()
            .any(|c| matches!(c, '\\' | '/' | ':') || c.is_control())
}

/// 一段安静时段，文本形如 `"22:30-07:00"`（24 小时制，可跨午夜，包含起点、不含终点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietRange {
    /// 起止时刻，从 0 点起的分钟数（0..1440）
    pub start: u16,
    pub end: u16,
}

impl QuietRange {
    /// 解析 `"HH:MM-HH:MM"`；起止相同（时长为 0 或一整天）算不合法。
    pub fn parse(s: &str) -> Option<Self> {
        let (a, b) = s.split_once('-')?;
        let (start, end) = (minute_of_day(a)?, minute_of_day(b)?);
        (start != end).then_some(Self { start, end })
    }

    /// 某个时刻是否落在这段里。
    pub fn contains(&self, t: NaiveTime) -> bool {
        let m = (t.hour() * 60 + t.minute()) as u16;
        if self.start < self.end {
            (self.start..self.end).contains(&m)
        } else {
            m >= self.start || m < self.end
        }
    }
}

/// `"HH:MM"`，两位小时、两位分钟。
fn minute_of_day(s: &str) -> Option<u16> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !(h.bytes().chain(m.bytes())).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// 是否处于任一安静时段。解析不了的条目跳过（写入时已校验，这里只防旧数据）。
pub fn in_quiet_hours<S: AsRef<str>>(ranges: &[S], t: NaiveTime) -> bool {
    ranges
        .iter()
        .filter_map(|r| QuietRange::parse(r.as_ref()))
        .any(|r| r.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn apps_compare_case_insensitively() {
        assert!(is_dnd_app("WeMeetApp.exe", DEFAULT_APPS));
        assert!(is_dnd_app("zoom.exe", DEFAULT_APPS));
        assert!(!is_dnd_app("WeChat.exe", DEFAULT_APPS));
        assert!(!is_dnd_app("Zoom.exe", &[] as &[&str]));
    }

    #[test]
    fn app_names_only() {
        for ok in DEFAULT_APPS
            .iter()
            .chain(&["POWERPNT.EXE", "企业微信.exe", "obs64"])
        {
            assert!(valid_app(ok), "{ok}");
        }
        for bad in [
            "",
            " Zoom.exe",
            "C:\\Zoom.exe",
            "bin/zoom",
            "a\tb",
            &"x".repeat(65),
        ] {
            assert!(!valid_app(bad), "{bad:?}");
        }
    }

    #[test]
    fn quiet_range_parsing() {
        assert_eq!(
            QuietRange::parse("22:30-07:00"),
            Some(QuietRange {
                start: 1350,
                end: 420
            })
        );
        assert!(QuietRange::parse("00:00-23:59").is_some());
        for bad in [
            "",
            "22:30",
            "22:30-22:30",
            "24:00-07:00",
            "7:00-08:00",
            "07:60-08:00",
            "07:00 - 08:00",
            "07:00-08:00-09:00",
            "+7:00-08:00",
        ] {
            assert_eq!(QuietRange::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn ranges_include_start_exclude_end_and_wrap_midnight() {
        let day = QuietRange::parse("12:00-14:00").unwrap();
        assert!(day.contains(at(12, 0)));
        assert!(day.contains(at(13, 59)));
        assert!(!day.contains(at(14, 0)));
        assert!(!day.contains(at(11, 59)));

        let night = QuietRange::parse("22:30-07:00").unwrap();
        assert!(night.contains(at(22, 30)));
        assert!(night.contains(at(0, 0)));
        assert!(night.contains(at(6, 59)));
        assert!(!night.contains(at(7, 0)));
        assert!(!night.contains(at(22, 29)));
    }

    #[test]
    fn any_range_counts_and_junk_is_ignored() {
        let ranges = ["bad", "12:00-13:00", "22:00-06:00"];
        assert!(in_quiet_hours(&ranges, at(12, 30)));
        assert!(in_quiet_hours(&ranges, at(23, 0)));
        assert!(!in_quiet_hours(&ranges, at(9, 0)));
        assert!(!in_quiet_hours(&[] as &[&str], at(23, 0)));
    }
}
