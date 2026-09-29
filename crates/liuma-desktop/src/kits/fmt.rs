//! 数值/时长/时钟格式化,状态栏统计 pill 与轮尾统计卡共用;纯函数零状态。
//!
//! 文案性输出(时长单位/日期词)经 rust-i18n(`locales/time.yml`);公开名
//! 是读当前 locale 的薄包装,`_l(.., locale)` 显式档核供双语言测试断言
//! (`locale =` 形参,不触全局)。

use crate::kits::i18n::{current_locale, t};

/// token 缩写:`517 / 12.2K / 517K / 75M`(K/M 进位;百位以下一位小数,
/// 整数位去尾零)
pub fn fmt_tokens_abbrev(v: u64) -> String {
    if v < 1_000 {
        v.to_string()
    } else if v < 1_000_000 {
        format!("{}K", scaled(v as f64 / 1_000.0))
    } else {
        format!("{}M", scaled(v as f64 / 1_000_000.0))
    }
}

/// 千分位精确值(`75,048,740`)
pub fn fmt_exact_count(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 缓存命中百分比(诚实 <100):billed=0 → None(不显示);四舍五入
/// 会到 100 时递增小数位保持真值(`99.6% → "99.6"`,`99.97% → "99.97"`)
pub fn fmt_cache_hit(read: u64, billed: u64) -> Option<String> {
    if billed == 0 {
        return None;
    }
    if read >= billed {
        return Some("100".to_string());
    }
    let pct = read as f64 / billed as f64 * 100.0;
    for dp in 0..=2u32 {
        let m = 10f64.powi(dp as i32);
        let q = (pct * m).round() as u64;
        if q < 100 * 10u64.pow(dp) {
            return Some(dec_string(q, dp));
        }
    }
    Some("99.99".to_string())
}

/// 紧凑时长(会话统计卡):亚分钟 0.1s 粒度(`1.3秒/30秒`),分钟封顶
/// (`18分1秒`;超 1 小时累计分钟数)
pub fn fmt_duration_compact(ms: i64) -> String {
    fmt_duration_compact_l(ms, current_locale())
}

/// [`fmt_duration_compact`] 显式档核(测试双语言断言用)
pub fn fmt_duration_compact_l(ms: i64, locale: &str) -> String {
    let s = ms.max(0) as f64 / 1000.0;
    if s < 60.0 {
        t!(
            "time.duration_s",
            locale = locale,
            v = dec_string((s * 10.0).round() as u64, 1)
        )
        .into_owned()
    } else {
        let whole = s.round() as u64;
        t!(
            "time.duration_ms",
            locale = locale,
            mins = whole / 60,
            secs = whole % 60
        )
        .into_owned()
    }
}

/// 轮尾时长(整秒三档):`30秒 / 18分1秒 / 1时2分3秒`
pub fn fmt_duration_run(ms: i64) -> String {
    fmt_duration_run_l(ms, current_locale())
}

/// [`fmt_duration_run`] 显式档核(测试双语言断言用)
pub fn fmt_duration_run_l(ms: i64, locale: &str) -> String {
    let whole = (ms.max(0) as f64 / 1000.0).round() as u64;
    if whole < 60 {
        t!("time.duration_s", locale = locale, v = whole).into_owned()
    } else if whole < 3_600 {
        t!(
            "time.duration_ms",
            locale = locale,
            mins = whole / 60,
            secs = whole % 60
        )
        .into_owned()
    } else {
        t!(
            "time.duration_hms",
            locale = locale,
            hours = whole / 3_600,
            mins = whole % 3_600 / 60,
            secs = whole % 60
        )
        .into_owned()
    }
}

/// 输出速度数值(`262 / 1.5`;调用方自拼 ` tok/s`)
pub fn fmt_tps(tps: f64) -> String {
    let v = tps.max(0.0);
    if v >= 10.0 {
        format!("{}", v.round() as u64)
    } else {
        dec_string((v * 10.0).round() as u64, 1)
    }
}

/// 消息时钟(本地时区):同日 `HH:mm`;同年 `M月D日 HH:mm`;跨年全日期
pub fn fmt_clock_md(ms: i64) -> String {
    fmt_clock_md_l(ms, current_locale())
}

/// [`fmt_clock_md`] 显式档核(测试双语言断言用)
pub fn fmt_clock_md_l(ms: i64, locale: &str) -> String {
    fmt_clock_md_at_l(ms, chrono::Local::now(), locale)
}

fn fmt_clock_md_at_l(ms: i64, now: chrono::DateTime<chrono::Local>, locale: &str) -> String {
    use chrono::{Datelike, TimeZone};
    let Some(dt) = chrono::Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let time = dt.format("%H:%M").to_string();
    if dt.date_naive() == now.date_naive() {
        time
    } else if dt.year() == now.year() {
        t!(
            "time.clock_md",
            locale = locale,
            month = dt.month(),
            day = dt.day(),
            time = time
        )
        .into_owned()
    } else {
        t!(
            "time.clock_ymd",
            locale = locale,
            year = dt.year(),
            month = dt.month(),
            day = dt.day(),
            time = time
        )
        .into_owned()
    }
}

/// 缩写段(百位以下一位小数,整数去尾零):`12.2 / 517 / 75`
fn scaled(x: f64) -> String {
    if x >= 100.0 {
        format!("{}", x.round() as u64)
    } else {
        dec_string((x * 10.0).round() as u64, 1)
    }
}

/// 定点小数(`q=997, dp=1 → "99.7"`;`q=990, dp=1 → "99"` 去尾零)
fn dec_string(q: u64, dp: u32) -> String {
    if dp == 0 {
        return q.to_string();
    }
    let m = 10u64.pow(dp);
    let (int, frac) = (q / m, q % m);
    if frac == 0 {
        int.to_string()
    } else {
        let mut fs = format!("{:0>w$}", frac, w = dp as usize);
        while fs.ends_with('0') {
            fs.pop();
        }
        format!("{int}.{fs}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_abbrev_buckets() {
        assert_eq!(fmt_tokens_abbrev(0), "0");
        assert_eq!(fmt_tokens_abbrev(517), "517");
        assert_eq!(fmt_tokens_abbrev(860), "860");
        assert_eq!(fmt_tokens_abbrev(12_200), "12.2K");
        assert_eq!(fmt_tokens_abbrev(12400), "12.4K");
        assert_eq!(fmt_tokens_abbrev(517_000), "517K");
        assert_eq!(fmt_tokens_abbrev(2_482_113), "2.5M");
        assert_eq!(fmt_tokens_abbrev(75_048_740), "75M");
    }

    #[test]
    fn exact_count_groups_thousands() {
        assert_eq!(fmt_exact_count(0), "0");
        assert_eq!(fmt_exact_count(999), "999");
        assert_eq!(fmt_exact_count(1_001), "1,001");
        assert_eq!(fmt_exact_count(75_048_740), "75,048,740");
        assert_eq!(fmt_exact_count(161_652), "161,652");
    }

    #[test]
    fn cache_hit_honest_under_100() {
        assert_eq!(fmt_cache_hit(0, 0), None);
        assert_eq!(fmt_cache_hit(100, 100), Some("100".to_string()));
        assert_eq!(fmt_cache_hit(200, 100), Some("100".to_string()));
        assert_eq!(fmt_cache_hit(0, 1_000), Some("0".to_string()));
        assert_eq!(fmt_cache_hit(500, 1_000), Some("50".to_string()));
        // 99.6 整数化会到 100 → 自动一位小数
        assert_eq!(fmt_cache_hit(996, 1_000), Some("99.6".to_string()));
        assert_eq!(fmt_cache_hit(9_997, 10_000), Some("99.97".to_string()));
        // 常规:整数足够诚实
        assert_eq!(
            fmt_cache_hit(74_651_648, 75_048_740),
            Some("99".to_string())
        );
    }

    #[test]
    fn duration_compact_tiers() {
        assert_eq!(fmt_duration_compact_l(0, crate::kits::i18n::DEFAULT), "0秒");
        assert_eq!(
            fmt_duration_compact_l(1_300, crate::kits::i18n::DEFAULT),
            "1.3秒"
        );
        assert_eq!(
            fmt_duration_compact_l(30_000, crate::kits::i18n::DEFAULT),
            "30秒"
        );
        assert_eq!(
            fmt_duration_compact_l(1_081_000, crate::kits::i18n::DEFAULT),
            "18分1秒"
        );
        assert_eq!(
            fmt_duration_compact_l(1_281_000, crate::kits::i18n::DEFAULT),
            "21分21秒"
        );
        assert_eq!(fmt_duration_compact_l(0, "en"), "0s");
        assert_eq!(fmt_duration_compact_l(1_300, "en"), "1.3s");
        assert_eq!(fmt_duration_compact_l(1_081_000, "en"), "18m 1s");
    }

    #[test]
    fn duration_run_tiers() {
        assert_eq!(
            fmt_duration_run_l(29_400, crate::kits::i18n::DEFAULT),
            "29秒"
        );
        assert_eq!(
            fmt_duration_run_l(30_400, crate::kits::i18n::DEFAULT),
            "30秒"
        );
        assert_eq!(
            fmt_duration_run_l(1_081_000, crate::kits::i18n::DEFAULT),
            "18分1秒"
        );
        assert_eq!(
            fmt_duration_run_l(3_723_000, crate::kits::i18n::DEFAULT),
            "1时2分3秒"
        );
        assert_eq!(fmt_duration_run_l(29_400, "en"), "29s");
        assert_eq!(fmt_duration_run_l(1_081_000, "en"), "18m 1s");
        assert_eq!(fmt_duration_run_l(3_723_000, "en"), "1h 2m 3s");
    }

    #[test]
    fn tps_precision() {
        assert_eq!(fmt_tps(262.4), "262");
        assert_eq!(fmt_tps(9.96), "10");
        assert_eq!(fmt_tps(1.46), "1.5");
        assert_eq!(fmt_tps(0.0), "0");
    }

    #[test]
    fn clock_md_tiers() {
        use chrono::TimeZone;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 9, 15, 22, 57, 0)
            .single()
            .unwrap();
        let same_day = chrono::Local
            .with_ymd_and_hms(2026, 9, 15, 8, 5, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let same_year = chrono::Local
            .with_ymd_and_hms(2026, 3, 2, 9, 30, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let cross_year = chrono::Local
            .with_ymd_and_hms(2025, 12, 31, 23, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            fmt_clock_md_at_l(same_day, now, crate::kits::i18n::DEFAULT),
            "08:05"
        );
        assert_eq!(
            fmt_clock_md_at_l(same_year, now, crate::kits::i18n::DEFAULT),
            "3月2日 09:30"
        );
        assert_eq!(
            fmt_clock_md_at_l(cross_year, now, crate::kits::i18n::DEFAULT),
            "2025年12月31日 23:00"
        );
        assert_eq!(fmt_clock_md_at_l(same_year, now, "en"), "3/2 09:30");
        assert_eq!(fmt_clock_md_at_l(cross_year, now, "en"), "2025/12/31 23:00");
    }
}
