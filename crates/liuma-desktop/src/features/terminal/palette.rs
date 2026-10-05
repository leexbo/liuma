//! 网格单元 → 渲染 runs(纯逻辑,先行可测):xterm 256 色解析 +
//! `Cell` 序列按 (前景, 背景, 样式旗标) 分组为 `StyledText` 高亮段。
//!
//! 色源解析顺序(高优先在前):OSC 覆盖表(`Colors`)→ 真彩 `Spec`
//! → 256 色索引 → `Named`(主题色);`INVERSE` 在分组键层面交换前
//! 景/背景。CJK 宽字符跳过 spacer 单元(等宽字体下字形自然占两格)。

use alacritty_terminal::grid::Indexed;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui_kit::{
    FontStyle, FontWeight, HighlightStyle, Hsla, StrikethroughStyle, UnderlineStyle, px,
};

/// 16 基础色(VS Code 终端盘;明暗两模式通用)
const BASE16: [u32; 16] = [
    0x2b2b2b, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xbbbbbb, 0x666666,
    0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xffffff,
];

/// 256 色索引 → RGB(xterm 标准立方 + 灰阶)
pub fn indexed_rgb(index: u8) -> [u8; 3] {
    match index {
        0..=15 => {
            let c = BASE16[usize::from(index)];
            [(c >> 16) as u8, (c >> 8) as u8, c as u8]
        }
        16..=231 => {
            let i = u32::from(index - 16);
            let steps = [0u32, 95, 135, 175, 215, 255];
            let (r, g, b) = (
                steps[(i / 36) as usize],
                steps[((i / 6) % 6) as usize],
                steps[(i % 6) as usize],
            );
            [r as u8, g as u8, b as u8]
        }
        _ => {
            // 232..=255 灰阶:8 + 10*i
            let v = 8 + 10 * u32::from(index - 232);
            [v as u8, v as u8, v as u8]
        }
    }
}

fn rgb_to_hsla(rgb: Rgb) -> Hsla {
    gpui_kit::rgb(u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b)).into()
}

fn u32_to_hsla(c: u32) -> Hsla {
    gpui_kit::rgb(c).into()
}

/// Named 语义色 → 主题色(`Foreground/Background` 用终端底/字主题对,
/// dim 系降透明;0–15/Bright 系查调色板)
fn named_hsla(named: NamedColor, term_fg: Hsla, term_bg: Hsla) -> Option<Hsla> {
    use NamedColor as N;
    Some(match named {
        N::Foreground => term_fg,
        N::Background => term_bg,
        N::Cursor => term_fg,
        N::DimForeground => term_fg.opacity(0.6),
        N::BrightForeground => term_fg,
        // Dim/Bright 黑白系之外按调色板索引回退(Bright* = +8)
        N::Black | N::DimBlack => u32_to_hsla(BASE16[0]),
        N::Red | N::DimRed => u32_to_hsla(BASE16[1]),
        N::Green | N::DimGreen => u32_to_hsla(BASE16[2]),
        N::Yellow | N::DimYellow => u32_to_hsla(BASE16[3]),
        N::Blue | N::DimBlue => u32_to_hsla(BASE16[4]),
        N::Magenta | N::DimMagenta => u32_to_hsla(BASE16[5]),
        N::Cyan | N::DimCyan => u32_to_hsla(BASE16[6]),
        N::White | N::DimWhite => u32_to_hsla(BASE16[7]),
        N::BrightBlack => u32_to_hsla(BASE16[8]),
        N::BrightRed => u32_to_hsla(BASE16[9]),
        N::BrightGreen => u32_to_hsla(BASE16[10]),
        N::BrightYellow => u32_to_hsla(BASE16[11]),
        N::BrightBlue => u32_to_hsla(BASE16[12]),
        N::BrightMagenta => u32_to_hsla(BASE16[13]),
        N::BrightCyan => u32_to_hsla(BASE16[14]),
        N::BrightWhite => u32_to_hsla(BASE16[15]),
    })
}

/// 解析单元颜色为渲染色;None = 用对侧默认(背景语义)。OSC 覆盖
/// (`Colors`,含 256 序号与 Named 序号)优先
pub fn resolve_color(color: Color, colors: &Colors, term_fg: Hsla, term_bg: Hsla) -> Option<Hsla> {
    if let Some(over) = match color {
        Color::Named(named) => colors[named],
        Color::Indexed(i) => colors[usize::from(i)],
        Color::Spec(_) => None,
    } {
        return Some(rgb_to_hsla(over));
    }
    match color {
        Color::Spec(rgb) => Some(rgb_to_hsla(rgb)),
        Color::Indexed(i) => {
            let [r, g, b] = indexed_rgb(i);
            Some(rgb_to_hsla(Rgb { r, g, b }))
        }
        Color::Named(named) => named_hsla(named, term_fg, term_bg),
    }
}

/// 单个渲染 run 的样式键(分组用)
#[derive(Clone, Copy, PartialEq)]
struct RunKey {
    fg: Hsla,
    bg: Hsla,
    bold: bool,
    italic: bool,
    underline: u8,
    strike: bool,
    dim: bool,
    selected: bool,
}

impl RunKey {
    fn style(self) -> HighlightStyle {
        let mut style = HighlightStyle {
            color: Some(self.fg),
            background_color: Some(self.bg),
            ..Default::default()
        };
        if self.bold {
            style.font_weight = Some(FontWeight::BOLD);
        }
        if self.italic {
            style.font_style = Some(FontStyle::Italic);
        }
        if self.dim {
            style.fade_out = Some(0.5);
        }
        // 1 = 直线,2 = 波浪(undercurl);gpui 无双线下划线形态,归并为直线
        style.underline = match self.underline {
            0 => None,
            thickness => Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: thickness == 2,
            }),
        };
        if self.strike {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        style
    }
}

/// 视口分桶:`display_iter`(产出网格行 `-(offset)..`)按行桶装为
/// `rows` 行的单元引用表(索引 0 = 视口顶)。视图渲染与测试读格共用
/// 同一换算,避免两处偏移语义漂移
pub fn bucket_lines<'a>(
    iter: impl Iterator<Item = Indexed<&'a alacritty_terminal::term::cell::Cell>>,
    offset: i32,
    rows: usize,
) -> Vec<Vec<&'a alacritty_terminal::term::cell::Cell>> {
    let mut lines: Vec<Vec<&alacritty_terminal::term::cell::Cell>> = vec![Vec::new(); rows];
    for Indexed { point, cell } in iter {
        let row = point.line.0 + offset;
        if row >= 0 && (row as usize) < rows {
            lines[row as usize].push(cell);
        }
    }
    lines
}

/// 选区行区间:网格坐标的 `SelectionRange` → 每视口行的**含端点**列区间
/// `(起列, 末列)`;整行覆盖记 `usize::MAX`(行尾裁剪处按 cells 长度收口)。
/// 非 block:首行自 `start.column` 起到行尾,末行到 `end.column` 止,
/// 中间行整行;block:全部行取 `start.column..=end.column`
pub fn selection_row_ranges(
    range: &alacritty_terminal::selection::SelectionRange,
    offset: i32,
    rows: usize,
) -> Vec<Option<(usize, usize)>> {
    let mut ranges: Vec<Option<(usize, usize)>> = vec![None; rows];
    for (row, slot) in ranges.iter_mut().enumerate() {
        let line = row as i32 - offset;
        if line < range.start.line.0 || line > range.end.line.0 {
            continue;
        }
        let cols = match range.is_block {
            true => (range.start.column.0, range.end.column.0),
            false => {
                let start = if line == range.start.line.0 {
                    range.start.column.0
                } else {
                    0
                };
                let end = if line == range.end.line.0 {
                    range.end.column.0
                } else {
                    usize::MAX
                };
                (start, end)
            }
        };
        *slot = Some(cols);
    }
    ranges
}

/// 一行网格单元 → (行文本, 高亮段)。`cursor_col` = 光标所在列(块状
/// 光标:该单元背景反转为光标色,压过选区底色)。`selection` = 该行
/// 含端点选区列区间(选中格背景换 `selection_bg`)。行尾连续默认单元
/// (空格 + 默认底、不在选区内)不产段,由容器底色承担。`HIDDEN`
/// 单元按空格落位保列对齐
pub fn row_runs(
    cells: &[&alacritty_terminal::term::cell::Cell],
    colors: &Colors,
    term_fg: Hsla,
    term_bg: Hsla,
    cursor_col: Option<usize>,
    selection: Option<(usize, usize)>,
    selection_bg: Hsla,
) -> (String, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
    let mut text = String::with_capacity(cells.len());
    let mut runs: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut current: Option<(RunKey, usize)> = None; // (key, run 起始字节)

    let default_fg = term_fg;
    let default_bg = term_bg;
    // 行尾裁剪点:最后一个「非默认单元」之后;光标列恒算非默认
    // (空行光标也要成块,否则提示行只有光标时无视觉锚)
    let mut visible = 0usize;
    for (ix, cell) in cells.iter().enumerate() {
        let is_spacer = cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
        let is_default = !is_spacer
            && cell.c == ' '
            && cell.bg == Color::Named(NamedColor::Background)
            && !cell
                .flags
                .intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
        let selected = selection.is_some_and(|(s, e)| ix >= s && ix <= e);
        if !is_default || cursor_col == Some(ix) || selected {
            visible = ix + 1;
        }
    }

    for (ix, cell) in cells.iter().take(visible).enumerate() {
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            // spacer 占位但不产出字符;若其下划线/背景与主体不同,等宽
            // 渲染下无法单独成段——忽略(v1 妥协,见模块注释)
            continue;
        }
        let inverse = cell.flags.contains(Flags::INVERSE);
        let mut fg = resolve_color(cell.fg, colors, term_fg, term_bg).unwrap_or(default_fg);
        let mut bg = resolve_color(cell.bg, colors, term_fg, term_bg).unwrap_or(default_bg);
        if inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        // 背景语义色(默认底)不落 run 背景,避免打断默认底容器
        if bg == default_bg {
            bg = gpui_kit::transparent_black();
        }
        let selected = selection.is_some_and(|(s, e)| ix >= s && ix <= e);
        if selected {
            bg = selection_bg;
        }
        let cursor_here = cursor_col == Some(ix);
        if cursor_here {
            bg = fg;
            fg = default_bg;
        }
        let key = RunKey {
            fg,
            bg,
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: if cell.flags.contains(Flags::UNDERCURL) {
                2
            } else if cell.flags.intersects(
                Flags::UNDERLINE
                    | Flags::DOUBLE_UNDERLINE
                    | Flags::DOTTED_UNDERLINE
                    | Flags::DASHED_UNDERLINE,
            ) {
                1
            } else {
                0
            },
            strike: cell.flags.contains(Flags::STRIKEOUT),
            dim: cell.flags.contains(Flags::DIM),
            selected,
        };
        let ch = if cell.flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        let start_byte = text.len();
        text.push(ch);
        let changed = !matches!(&current, Some((open_key, _)) if *open_key == key);
        if changed {
            if let Some((open_key, open_byte)) = current.take() {
                runs.push((open_byte..start_byte, open_key.style()));
            }
            current = Some((key, start_byte));
        }
    }
    if let Some((key, byte)) = current {
        runs.push((byte..text.len(), key.style()));
    }
    (text, runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::cell::Cell;

    fn view(cells: &[Cell]) -> Vec<&Cell> {
        cells.iter().collect()
    }

    fn cell(c: char) -> Cell {
        Cell {
            c,
            ..Cell::default()
        }
    }

    fn styled(c: char, fg: Color, bg: Color, flags: Flags) -> Cell {
        Cell {
            c,
            fg,
            bg,
            flags,
            ..Cell::default()
        }
    }

    const FG: Hsla = Hsla {
        h: 0.,
        s: 0.,
        l: 0.9,
        a: 1.,
    };
    const BG: Hsla = Hsla {
        h: 0.,
        s: 0.,
        l: 0.1,
        a: 1.,
    };

    #[test]
    fn indexed_cube_and_grayscale() {
        assert_eq!(indexed_rgb(1), [0xcd, 0x31, 0x31]);
        assert_eq!(indexed_rgb(16), [0, 0, 0]);
        assert_eq!(indexed_rgb(231), [255, 255, 255]);
        assert_eq!(indexed_rgb(196), [255, 0, 0]);
        assert_eq!(indexed_rgb(232), [8, 8, 8]);
        assert_eq!(indexed_rgb(255), [238, 238, 238]);
    }

    #[test]
    fn runs_group_by_style_and_trim_trailing() {
        let cells = vec![
            styled(
                'r',
                Color::Named(NamedColor::Red),
                Color::Named(NamedColor::Background),
                Flags::BOLD,
            ),
            styled(
                'e',
                Color::Named(NamedColor::Red),
                Color::Named(NamedColor::Background),
                Flags::BOLD,
            ),
            styled(
                ' ',
                Color::Named(NamedColor::Foreground),
                Color::Named(NamedColor::Background),
                Flags::empty(),
            ),
            styled(
                ' ',
                Color::Named(NamedColor::Foreground),
                Color::Named(NamedColor::Background),
                Flags::empty(),
            ),
        ];
        let colors = Colors::default();
        let (text, runs) = row_runs(&view(&cells), &colors, FG, BG, None, None, FG);
        assert_eq!(text, "re");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, 0..2);
        assert_eq!(
            runs[0].1.color,
            resolve_color(Color::Named(NamedColor::Red), &colors, FG, BG)
        );
        assert_eq!(runs[0].1.font_weight, Some(FontWeight::BOLD));
    }

    #[test]
    fn inverse_swaps_and_cursor_inverts() {
        let cells = vec![styled(
            'x',
            Color::Named(NamedColor::Red),
            Color::Named(NamedColor::Background),
            Flags::INVERSE,
        )];
        let (text, runs) = row_runs(&view(&cells), &Colors::default(), FG, BG, None, None, FG);
        assert_eq!(text, "x");
        // INVERSE:fg 变背景(默认底),bg 变红
        assert_eq!(
            runs[0].1.background_color,
            resolve_color(Color::Named(NamedColor::Red), &Colors::default(), FG, BG)
        );
        // 光标列:前景/背景互换(块状光标)
        let (_, runs) = row_runs(&view(&cells), &Colors::default(), FG, BG, Some(0), None, FG);
        assert_eq!(runs[0].1.color, Some(BG));
    }

    #[test]
    fn wide_char_spacer_skipped() {
        let cells = vec![
            styled(
                '汉',
                Color::Named(NamedColor::Foreground),
                Color::Named(NamedColor::Background),
                Flags::WIDE_CHAR,
            ),
            styled(
                ' ',
                Color::Named(NamedColor::Foreground),
                Color::Named(NamedColor::Background),
                Flags::WIDE_CHAR_SPACER,
            ),
            cell('a'),
        ];
        let (text, _) = row_runs(&view(&cells), &Colors::default(), FG, BG, None, None, FG);
        assert_eq!(text, "汉a");
    }

    #[test]
    fn cursor_block_renders_on_blank_row() {
        // 整行只有光标(空提示行):光标列必须产出可视块
        let cells: Vec<Cell> = (0..4).map(|_| cell(' ')).collect();
        let (text, runs) = row_runs(&view(&cells), &Colors::default(), FG, BG, Some(0), None, FG);
        assert_eq!(text, " ");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].1.color, Some(BG));
        assert_eq!(runs[0].1.background_color, Some(FG));
    }

    #[test]
    fn bucket_lines_maps_offset_to_viewport_rows() {
        use alacritty_terminal::grid::Indexed;
        use alacritty_terminal::index::{Column, Line, Point};
        // 5 单元:网格行 -1..=3,每行一个标识字符
        let cells: Vec<Cell> = "abcde".chars().map(cell).collect();
        let iter = cells.iter().enumerate().map(|(i, cell)| Indexed {
            point: Point::new(Line(i as i32 - 1), Column(0)),
            cell,
        });
        // offset=2,rows=3:网格行 L → 视口行 L+2;'a' 在网格 -1 → 视口 1
        let lines = bucket_lines(iter, 2, 3);
        assert!(lines[0].is_empty());
        assert_eq!(lines[1][0].c, 'a');
        assert_eq!(lines[2][0].c, 'b');
    }

    #[test]
    fn osc_override_and_truecolor_win() {
        let mut colors = Colors::default();
        colors[NamedColor::Red] = Some(Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56,
        });
        let resolved = resolve_color(Color::Named(NamedColor::Red), &colors, FG, BG);
        assert_eq!(resolved, Some(u32_to_hsla(0x123456)));
        let spec = resolve_color(
            Color::Spec(Rgb {
                r: 0xaa,
                g: 0xbb,
                b: 0xcc,
            }),
            &colors,
            FG,
            BG,
        );
        assert_eq!(spec, Some(u32_to_hsla(0xaabbcc)));
    }

    #[test]
    fn selection_ranges_map_lines_and_columns() {
        use alacritty_terminal::index::{Column, Line, Point};
        use alacritty_terminal::selection::SelectionRange;
        // 视口 4 行,offset=1:视口行 r ↔ 网格行 r-1
        let point = |line: i32, col: usize| Point::new(Line(line), Column(col));
        // 单行选区:网格行 1(视口行 2),列 2..=4
        let range = SelectionRange::new(point(1, 2), point(1, 4), false);
        let ranges = selection_row_ranges(&range, 1, 4);
        assert_eq!(ranges, vec![None, None, Some((2, 4)), None]);
        // 跨行选区:网格行 0..=2 → 视口行 1..=3,首行 3 列起、中间整行、
        // 末行到列 5
        let range = SelectionRange::new(point(0, 3), point(2, 5), false);
        let ranges = selection_row_ranges(&range, 1, 4);
        assert_eq!(
            ranges,
            vec![
                None,
                Some((3, usize::MAX)),
                Some((0, usize::MAX)),
                Some((0, 5))
            ]
        );
        // block:各行同列区间
        let range = SelectionRange::new(point(0, 3), point(2, 5), true);
        let ranges = selection_row_ranges(&range, 1, 4);
        assert_eq!(ranges, vec![None, Some((3, 5)), Some((3, 5)), Some((3, 5))]);
    }

    #[test]
    fn selected_cells_take_selection_background() {
        let cells = vec![cell('a'), cell('b'), cell('c')];
        let sel = u32_to_hsla(0x3355ff);
        let (text, runs) = row_runs(
            &view(&cells),
            &Colors::default(),
            FG,
            BG,
            None,
            Some((0, 1)),
            sel,
        );
        assert_eq!(text, "abc");
        assert_eq!(runs.len(), 2, "选区边界应切开 run");
        assert_eq!(runs[0].1.background_color, Some(sel));
        assert_eq!(
            runs[1].1.background_color,
            Some(gpui_kit::transparent_black())
        );
        // 光标压过选区:光标列前景/背景互换
        let (_, runs) = row_runs(
            &view(&cells),
            &Colors::default(),
            FG,
            BG,
            Some(0),
            Some((0, 1)),
            sel,
        );
        assert_eq!(runs[0].1.color, Some(BG));
    }
}
