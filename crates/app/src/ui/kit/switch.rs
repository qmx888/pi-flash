//! 开关（三态 Off/Mixed/On）：视觉 `switch_el_mark`、独立开关 `switch_base`、
//! 行内 `config_switch`、组头带计数的 `group_switch`。
//! 尺寸 SW_MD 28×16 / knob 10（`ui::tokens::switch`），见 §5.3 / §10.4。

use gpui::prelude::*;
use gpui::{AnyElement, Context, MouseButton, SharedString, div, px, rgb};

use crate::Chat;
use crate::appearance::theme::theme as T;

/// 开关三态：关 / 部分选中 / 开。
///
/// 「部分选中」是组头（技能分组、扩展全局）与模型页整商开关的必需态：一组里
/// 只启用了一部分时，开关不能装成全关（会被误读成「一项都没开」），也不该
/// 和全选一模一样——轨道用减淡的 accent，knob 停在开位。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchMark {
    Off,
    Mixed,
    On,
}

impl SwitchMark {
    /// 从计数算态：0 = 关，全中 = 开，其余 = 部分。
    pub(crate) fn from_counts(on: usize, total: usize) -> Self {
        if total == 0 || on == 0 {
            Self::Off
        } else if on >= total {
            Self::On
        } else {
            Self::Mixed
        }
    }

    pub(crate) fn is_on(self) -> bool {
        matches!(self, Self::On | Self::Mixed)
    }
}

/// 开关视觉（SW_MD 28×16·knob10，`ui::tokens::switch`）：无 id/无事件，
/// [`switch_base`]（独立开关）与 [`group_switch`]（组头行内）共用。
fn switch_el_mark(mark: SwitchMark) -> gpui::Div {
    use crate::ui::tokens::switch as sw;
    let t = T();
    let lit = mark.is_on();
    // 部分选中：轨道填色减淡（accent 半透明），knob 照旧停在开位
    let track: gpui::Hsla = match mark {
        SwitchMark::Off => rgb(t.bg_selected).into(),
        SwitchMark::On => rgb(t.accent).into(),
        SwitchMark::Mixed => {
            let a: gpui::Hsla = rgb(t.accent).into();
            gpui::hsla(a.h, a.s, a.l, sw::MD_MIXED_ALPHA)
        }
    };
    let border: gpui::Hsla = match mark {
        SwitchMark::Off => rgb(t.border).into(),
        SwitchMark::On => rgb(t.accent).into(),
        SwitchMark::Mixed => {
            let a: gpui::Hsla = rgb(t.accent).into();
            gpui::hsla(a.h, a.s, a.l, sw::MD_MIXED_ALPHA + 0.25)
        }
    };
    div()
        .w(px(sw::MD_W))
        .h(px(sw::MD_H))
        .flex_shrink_0()
        .rounded(px(sw::MD_H / 2.))
        .border_1()
        .border_color(border)
        .bg(track)
        .flex()
        .items_center()
        .child(
            div()
                .ml(px(if lit { sw::MD_ON_ML } else { sw::MD_OFF_ML }))
                .size(px(sw::MD_KNOB))
                .rounded_full()
                .bg(if lit { rgb(t.bg) } else { rgb(t.text_muted) }),
        )
}

fn switch_el(on: bool) -> gpui::Div {
    switch_el_mark(if on { SwitchMark::On } else { SwitchMark::Off })
}

/// 开关视觉唯一入口（独立开关）。`apply` 收到新状态 bool 由调用方自行
/// 解释；Chat 场景走 [`config_switch`]。
pub(crate) fn switch_base(
    id: impl Into<SharedString>,
    on: bool,
    disabled: bool,
    apply: impl Fn(bool, &mut gpui::App) + 'static,
) -> AnyElement {
    let el = switch_el(on).id(id.into());
    if disabled {
        return el.opacity(0.5).into_any_element();
    }
    el.cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            apply(!on, cx);
        })
        .into_any_element()
}

/// ConfigSwitch：设置页行内开关（Chat 回调形态）。
pub(crate) fn config_switch(
    id: impl Into<SharedString>,
    weak: &gpui::WeakEntity<Chat>,
    on: bool,
    disabled: bool,
    on_toggle: impl Fn(&mut Chat, &mut Context<Chat>) + 'static,
) -> AnyElement {
    let weak = weak.clone();
    switch_base(id, on, disabled, move |_, cx| {
        let _ = weak.update(cx, |c, cx| on_toggle(c, cx));
    })
}

/// 组头小开关行（ConfigSidebarGroupSwitch）：{enabled}/{total} + 开关。
/// 开关尺寸随 2026-10-09 定稿与全局统一为 SW_MD 28×16（原 24×14 小档废除）；
/// 三态（[`SwitchMark`]）——部分启用必须看得出来，不能装成全关。
pub(crate) fn group_switch(
    id: impl Into<SharedString>,
    weak: &gpui::WeakEntity<Chat>,
    count_text: String,
    mark: SwitchMark,
    disabled: bool,
    on_toggle: impl Fn(&mut Chat, &mut Context<Chat>) + 'static,
) -> AnyElement {
    let t = T();
    let row = div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .font_family(crate::editor::markdown::MONO_FAMILY)
                .text_size(crate::appearance::ui_size(10.))
                .text_color(rgb(t.text_dim))
                .child(SharedString::from(count_text)),
        )
        .child({
            let mut sw = switch_el_mark(mark);
            if !disabled {
                sw = sw.cursor_pointer();
            } else {
                sw = sw.opacity(0.5);
            }
            sw
        });
    if disabled {
        return row.into_any_element();
    }
    let weak = weak.clone();
    row.on_mouse_down(MouseButton::Left, move |_, _, cx| {
        cx.stop_propagation();
        let _ = weak.update(cx, |c, cx| on_toggle(c, cx));
    })
    .into_any_element()
}
