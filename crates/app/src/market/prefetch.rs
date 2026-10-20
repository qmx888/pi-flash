//! 市场预热（启动【延迟加载】阶段，010-启动.md）：**只预取首页**。
//!
//! 为什么放延迟加载：进市场页的体验主要由「首页有没有现成数据」决定；揭幕
//! 后再拉一次首页（1 次搜索 ~1.0s + ≤50 次清单 ~2.6s）就能把首页快照刷新，
//! 用户点开菜单时直接命中快照（0 网络）。
//!
//! 三条硬约束（040-1 §3.5）：
//! 1. **只在已有快照时预热**——没有快照 = 用户从没打开过市场页 = 零请求
//!    （不给不用的人偷跑流量 / 不留后台网络行为）；
//! 2. 1 小时内的快照不重复拉（省流量也省 CPU）；
//! 3. 只写磁盘快照，**不碰 UI 状态**（页面没开就没有状态；页面开着时它自己
//!    的刷新链路负责），失败静默、不重试；`PI_OFFLINE=1` 直接跳过。
//!
//! 内存侧几乎零成本：临时 Vec 在后台线程里用完即弃，落盘只有一个 <100KB
//! JSON。

use std::collections::HashMap;

use pi_link::market::pi_ext_market::{HomeSnapshot, PAGE_SIZE, SNAPSHOT_MAX};

use crate::Chat;

/// 快照多久算「值得再刷一次」。
const REFRESH_AFTER_SECS: i64 = 3600;

impl Chat {
    /// 揭幕后延迟加载钩子（startup.rs 调用一次）。
    pub(crate) fn spawn_pi_ext_market_prefetch(&mut self, cx: &mut gpui::Context<Self>) {
        // 入口关闭（dev-flags `pi_ext_market`，默认关）→ 连后台预热也不跑：
        // 没人能看到市场页，就别为用户偷跑网络。
        if !crate::services::workspace::pi_ext_market_enabled() {
            return;
        }
        // 页面已装载/打开 → 让它自己的刷新链路负责，别抢
        if self.aux.market.pi_ext.loaded_once || self.aux.market.pi_ext.loading {
            return;
        }
        let Some(snap) = HomeSnapshot::load() else {
            return; // 从未用过市场页：零请求
        };
        if snap.age_secs() < REFRESH_AFTER_SECS {
            return;
        }
        let query = snap.query.clone();
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_executor()
                .spawn(async move {
                    let page = pi_link::market::pi_ext_market::search(&query, 0, PAGE_SIZE).ok()?;
                    let mut entries = page.entries;
                    let mut kinds = HashMap::new();
                    pi_link::market::pi_ext_market::fill_kinds(&mut entries, &mut kinds);
                    let ext: Vec<_> = entries
                        .into_iter()
                        .filter(|e| e.is_extension())
                        .take(SNAPSHOT_MAX)
                        .collect();
                    if ext.is_empty() {
                        return None;
                    }
                    let snap = HomeSnapshot::new_now(&query, ext);
                    snap.save();
                    Some(())
                })
                .await;
            // 预热不改页面状态：只在成功且页面还没装载时通知一次（没别的副作用）
            if done.is_some() {
                let _ = this.update(cx, |_c, cx| cx.notify());
            }
        })
        .detach();
    }
}
