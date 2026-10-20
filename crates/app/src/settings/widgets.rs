//! 设置面板专属件（`select_section` 这类与 `SettingsPanel` 强绑的逻辑）。
//!
//! 公共组件**已全部上移到 [`crate::ui::kit`]**（2026-10-20 用户定夺：UI 组件
//! 归 UI 库，不再住在 settings 里）；这里的 `pub(crate) use` 只是兼容转发，
//! 存量 `widgets::xxx` 调用点不必改。新代码直接 `use crate::ui::kit::*;`。

use gpui::Window;

use super::*;

pub(crate) use crate::ui::kit::*;

/// 侧栏行点击（section 切换 + 清错误）。
pub(crate) fn select_section(
    weak: &gpui::WeakEntity<Chat>,
    section: String,
) -> impl Fn(&mut Window, &mut gpui::App) + 'static {
    let weak = weak.clone();
    move |_, cx| {
        let _ = weak.update(cx, |c, cx| {
            if let Some(st) = c.dlg.settings.clone() {
                st.update(cx, |s, cx| {
                    s.section = section.clone();
                    s.error = None;
                    cx.notify();
                });
            }
        });
    }
}
