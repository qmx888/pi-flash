//! 终端快照纯逻辑：选区删除计划（deletion_plan，终端选区可编辑）、
//! 网格快照（snapshot）、选区文本提取。不持 PTY 锁的读路径。

use super::*;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Color;
use gpui::Hsla;

// ---------------------------------------------------------------------------
// grid snapshot: (cells, styles) per visible row
/// 选区删除计划：删掉选区需要在 shell 的行编辑里按几次「左移」与
/// 「前向删除」。终端的事件缓冲区在 shell 手里，没有「直接删某段文本」的
/// 接口，只能像 xterm 的 `delete-selection` 一样自己算键：
///
/// 光标（PTY 的光标 = 行尾）左移 `left` 次落到选区首字符上，再前向删除
/// `del` 次，后面的内容会左移填补——删完光标停在接缝处（与 GUI 文本框
/// 语义一致）。
///
/// 以下情况返回 `None`（调用方按原语义把键交给 shell）：
/// - alt screen（vim/less 那类全屏程序的行模型由应用自己管）；
/// - 选区跨屏幕行，或不在光标所在行；
/// - 选区整体落在光标右侧/与光标重叠（键数无法定义）；
/// - 选区裁掉尾部空白后没有可删字符（比如只选了空格）；
/// - 需要按的键数超过 512（保守门槛，避免对奇怪的屏内容狂发键）。
pub fn deletion_plan(term: &Term<Proxy>, sel: (SelPt, SelPt)) -> Option<(usize, usize)> {
    // alt screen：行模型由全屏应用自己管，别去猜它的光标语义
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return None;
    }
    let (a, b) = if sel.0 <= sel.1 { sel } else { (sel.1, sel.0) };
    if a.line != b.line {
        return None;
    }
    let (cursor_line, cursor_col) = {
        let content = term.renderable_content();
        (content.cursor.point.line.0, content.cursor.point.column.0)
    };
    if cursor_line != a.line || cursor_col <= a.col {
        return None;
    }
    // 收集 [选区左端, 光标) 这段格子：选区部分用来算删几个，选区右端到
    // 光标之间用来算左移几次（光标指在「待写格」上，本身不算字符）
    let cells: Vec<(usize, char, bool)> = {
        let grid = term.grid();
        let cols = grid.columns();
        let hi = cursor_col.min(cols);
        let lo = a.col.min(hi);
        let row = &grid[Line(a.line)];
        (lo..hi)
            .map(|c| {
                let cell = &row[Column(c)];
                (c, cell.c, cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            })
            .collect()
    };
    let sel_count = cells.iter().take_while(|(c, _, _)| *c <= b.col).count();
    let sel_cells = &cells[..sel_count];
    // 选区尾部常带 alacritty 多给的一格空白（词尾/行尾）；只删到最后一段
    // 可见字符，免得把后面的真实字符一起吃掉
    let last_ix = sel_cells
        .iter()
        .rposition(|(_, ch, spacer)| !*spacer && *ch != ' ' && *ch != '\0')?;
    let last_col = sel_cells[last_ix].0;
    let del = sel_cells[..=last_ix]
        .iter()
        .filter(|(_, _, spacer)| !*spacer)
        .count();
    // 选区右端到光标之间的字符数（折行的行尾、宽字符的 spacer 都不算）
    let after = cells
        .iter()
        .filter(|(c, _, spacer)| *c > last_col && !*spacer)
        .count();
    if del == 0 || del + after > 512 {
        return None;
    }
    Some((del + after, del))
}

// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub struct CellStyle {
    pub(super) fg: Hsla,
    pub(super) bg: Option<Hsla>,
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) underline: bool,
    pub(super) strike: bool,
}

pub struct SnapRow {
    pub text: String,
    pub styles: Vec<CellStyle>,
}

/// Cell is part of the selection range (row-wise band between normalized pts).
fn cell_selected(sel: (SelPt, SelPt), line: i32, col: usize) -> bool {
    let (a, b) = if sel.0 <= sel.1 { sel } else { (sel.1, sel.0) };
    if line < a.line || line > b.line {
        return false;
    }
    if a.line == b.line {
        return col >= a.col && col <= b.col;
    }
    if line == a.line {
        col >= a.col
    } else if line == b.line {
        col <= b.col
    } else {
        true
    }
}

/// Snapshot the visible grid (rows tall) into styled rows.
/// The cursor is folded in as a cell style (block = bg swap, unfocused = dim).
/// Snapshot the visible grid (rows tall) into styled rows. The cursor is
/// NOT folded in here — it paints as a caret bar overlay in the element.
pub fn snapshot(
    term: &Term<Proxy>,
    rows: usize,
    selection: Option<(SelPt, SelPt)>,
) -> Vec<SnapRow> {
    let grid = term.grid();
    // 行数钳制（crash-1792490920）：调用方（paint）的行数戳若与网格脱节
    // （同帧 resize 后的旧值/weak.update 失败），越过 screen_lines 会直
    // 接踩 storage 的越界断言；这里按网格真实行数收口，多出的行不画。
    let rows = rows.min(grid.screen_lines());
    let offset = grid.display_offset() as i32;
    let cols = grid.columns();

    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let line = r as i32 - offset;
        let row = &grid[Line(line)];
        let mut text = String::new();
        let mut styles: Vec<CellStyle> = Vec::new();
        for c in 0..cols {
            let cell = &row[Column(c)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue; // consumed by the wide char itself
            }
            let mut ch = if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            if ch == '\0' || ch == '\n' || ch == '\r' {
                ch = ' ';
            }

            // resolve fg (bold -> bright variant for indexed 0..8, xterm parity)
            let mut fg_rgb = color_to_rgb(&cell.fg);
            if cell.flags.contains(Flags::BOLD) {
                if let Color::Indexed(i) = cell.fg {
                    if i < 8 {
                        fg_rgb = palette256(i as usize + 8);
                    }
                }
            }
            let bg_rgb = color_to_rgb(&cell.bg);
            let (fg_rgb, mut bg_rgb) = if cell.flags.contains(Flags::INVERSE) {
                (bg_rgb, fg_rgb)
            } else {
                (fg_rgb, bg_rgb)
            };

            let selected = selection.is_some_and(|s| cell_selected(s, line, c));
            if selected {
                bg_rgb = TERM_SEL;
            }

            let mut fg = hsl(fg_rgb);
            if cell.flags.contains(Flags::DIM) {
                fg = hsl_a(fg_rgb, 0.55);
            }
            let bg: Option<Hsla> = if bg_rgb == TERM_BG && !selected {
                None
            } else {
                Some(hsl(bg_rgb))
            };

            let style = CellStyle {
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.contains(Flags::ALL_UNDERLINES),
                strike: cell.flags.contains(Flags::STRIKEOUT),
            };
            text.push(ch);
            styles.push(style);
        }
        out.push(SnapRow { text, styles });
    }
    out
}

/// 裸 `exit` 判定：光标所在输入行剥掉提示符（cmd/PS 的 `>`、bash/fish
/// 的 `$`、zsh 的 `%`、root 的 `#`）后正好是 `exit`（大小写不敏感、允许
/// 首尾空白）→ true。alt screen 由调用方先行排除。`exit 0` / `exit()`
/// 这类带尾巴的不算裸 exit。命中时由调用方决定动作（当前 = 关终端 tab）。
pub fn is_bare_exit(term: &Term<Proxy>) -> bool {
    let content = term.renderable_content();
    let line = content.cursor.point.line.0;
    let grid = term.grid();
    let row = &grid[Line(line)];
    let cols = grid.columns();
    let mut text = String::with_capacity(cols);
    for c in 0..cols {
        let cell = &row[Column(c)];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        text.push(if cell.c == '\0' { ' ' } else { cell.c });
    }
    let trimmed = text.trim_end();
    let after_prompt = match trimmed.rfind(['>', '$', '%', '#']) {
        Some(ix) => &trimmed[ix + 1..],
        None => trimmed,
    };
    after_prompt.trim().eq_ignore_ascii_case("exit")
}

/// Extract the selected text from the grid (xterm `getString` parity: trailing
/// whitespace trimmed, lines joined with \n, wide spacers skipped).
pub fn selection_text(term: &Term<Proxy>, sel: (SelPt, SelPt)) -> String {
    let (a, b) = if sel.0 <= sel.1 { sel } else { (sel.1, sel.0) };
    let grid = term.grid();
    let screen = grid.screen_lines() as i32;
    let min_line = -(grid.total_lines() as i32 - screen);
    let a = SelPt {
        line: a.line.clamp(min_line, screen - 1),
        ..a
    };
    let b = SelPt {
        line: b.line.clamp(min_line, screen - 1),
        ..b
    };
    let mut lines = Vec::new();
    for line in a.line..=b.line {
        let row = &grid[Line(line)];
        let (c0, c1) = if a.line == b.line {
            (a.col, b.col)
        } else if line == a.line {
            (a.col, grid.columns() - 1)
        } else if line == b.line {
            (0, b.col)
        } else {
            (0, grid.columns() - 1)
        };
        let mut s = String::new();
        for c in c0..=c1.min(grid.columns() - 1) {
            let cell = &row[Column(c)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            s.push(cell.c);
        }
        lines.push(s.trim_end().to_string());
    }
    lines.join("\n")
}
