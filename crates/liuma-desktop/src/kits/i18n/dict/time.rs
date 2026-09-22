//! 时间与数量格式文案词典(时长/日期/相对时间/额度窗口)。
//! `kits::fmt` 与 `shell::reducer` 的格式函数经 `_l(.., Lang)` 显式语言
//! 核走这里的模板;公开名留读语言盘的薄包装。

use crate::kits::i18n::entries;

entries! {
    /// 时长(秒;紧凑/轮尾/运行时长共用形态)
    duration_s(v) => ["{v}秒", "{v}s"],
    /// 时长(分 + 秒;秒位由调用方按需零填充)
    duration_ms(mins, secs) => ["{mins}分{secs}秒", "{mins}m {secs}s"],
    /// 时长(时 + 分 + 秒)
    duration_hms(hours, mins, secs) => ["{hours}时{mins}分{secs}秒", "{hours}h {mins}m {secs}s"],
    /// 消息时钟(同年跨日)
    clock_md(month, day, time) => ["{month}月{day}日 {time}", "{month}/{day} {time}"],
    /// 日期(无时刻;轨迹 Started 段用)
    clock_md_plain(month, day) => ["{month}月{day}日", "{month}/{day}"],
    /// 消息时钟(跨年)
    clock_ymd(year, month, day, time) => ["{year}年{month}月{day}日 {time}", "{year}/{month}/{day} {time}"],

    /// 相对时间:刚刚
    rel_just_now => ["刚刚", "Just now"],
    /// 相对时间:N 分钟前
    rel_mins_ago(n) => ["{n} 分钟前", "{n} minutes ago"],
    /// 相对时间:N 小时前
    rel_hours_ago(n) => ["{n} 小时前", "{n} hours ago"],
    /// 相对时间:N 天前
    rel_days_ago(n) => ["{n} 天前", "{n} days ago"],
    /// 相对时间:更早(一周开外)
    rel_earlier => ["更早", "Earlier"],

    /// 额度窗口名:5 小时(紧凑;状态栏窗口切换钮)
    window_5h => ["5小时", "5h"],
    /// 额度窗口名:1 周(紧凑)
    window_1w => ["1周", "1w"],
    /// 额度窗口名:5 小时(统计卡行标)
    window_5h_long => ["5 小时", "5 hours"],
    /// 额度窗口名:1 周(统计卡行标)
    window_1w_long => ["1 周", "1 week"],
}
