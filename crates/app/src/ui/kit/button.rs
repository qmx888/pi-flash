//! 页面按钮（`Btn` 变体 + Chat 回调形态）。
//!
//! 只做变体/尺寸映射，视觉全在 [`crate::ui::button`]（§5.1 唯一按钮表）。
//! 设置页口径：`small` = MD(28)，默认 = LG(32)。

use gpui::{AnyElement, Context, SharedString};

use crate::Chat;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Btn {
    Primary,
    Secondary,
    Danger,
}

/// ConfigButton：variant × size，点击回调直接拿到 Chat（动作都在 Chat 上）。
/// 尺寸/配色委托 `ui::button`（§5.1 唯一按钮表）：small→MD(28)，default→LG(32)。
pub(crate) fn config_button(
    id: impl Into<SharedString>,
    weak: &gpui::WeakEntity<Chat>,
    label: &str,
    variant: Btn,
    small: bool,
    disabled: bool,
    on_click: impl Fn(&mut Chat, &mut Context<Chat>) + 'static,
) -> AnyElement {
    let size = if small {
        crate::ui::BtnSize::Md
    } else {
        crate::ui::BtnSize::Lg
    };
    let variant = match variant {
        Btn::Primary => crate::ui::BtnVariant::Primary,
        Btn::Secondary => crate::ui::BtnVariant::Secondary,
        Btn::Danger => crate::ui::BtnVariant::Danger,
    };
    let weak = weak.clone();
    crate::ui::button(
        id,
        label.to_string(),
        size,
        variant,
        disabled,
        move |_, _, cx| {
            let _ = weak.update(cx, |c, cx| on_click(c, cx));
        },
    )
}
