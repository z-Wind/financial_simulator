use leptos::prelude::*;
use std::cell::Cell;
use std::collections::HashMap;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

use crate::utils::{format_twd_financial, generate_plot};

// =====================================================================
// # 1. 全域常數設定
// =====================================================================
/// 最大年齡
pub(crate) const MAX_AGE: usize = 150;

/// 使用者未輸入歷史資產時，系統假設的預設年化報酬率基準。
pub(crate) const DEFAULT_ANCHOR_ROI_PCT: f64 = 7.0;

/// 視窗寬度小於此值時，切換為窄版排版（精簡圖例/文字）。
pub(crate) const NARROW_WIDTH_BREAKPOINT: u32 = 640;

/// 使用者輸入變動後，延遲多久才觸發重新計算/繪圖（避免每個字元都重算）。
#[cfg(target_family = "wasm")]
pub(crate) const DEBOUNCE_MS: i32 = 300;

/// Y 軸（對數座標）視覺下限：低於此金額一律顯示在 1 萬的位置，避免 log(0) 爆炸。
pub(crate) const CHART_Y_VISUAL_FLOOR: f64 = 10_000.0;

/// 所有軌道資產都不到視覺下限時，預設給的可視上限（10 萬）。
pub(crate) const CHART_Y_DEFAULT_MAX: f64 = 100_000.0;

/// Y 軸上限抓「資料最大值」再乘上的留白倍數。
pub(crate) const CHART_Y_HEADROOM_MULTIPLIER: f64 = 1.5;

/// 資產規模均未超過視覺下限時，Y 軸對數上限的預設值（= log10(10 萬)）。
pub(crate) const CHART_Y_DEFAULT_LOG_MAX: f64 = 5.0;

/// 由現有資產反推隱含年化報酬率時，視為「合理範圍」的上下界；超出此範圍會在 UI 顯示警告。
pub(crate) const ROI_WARNING_MIN: f64 = -25.0;
pub(crate) const ROI_WARNING_MAX: f64 = 25.0;

/// 輸入框單位換算：使用者以「千元」輸入的欄位（h_inv_k / f_inv_k）換算成元。
pub(crate) const THOUSAND_TO_TWD: f64 = 1_000.0;

/// 輸入框單位換算：使用者以「萬元」輸入的欄位（asset_wan）換算成元。
pub(crate) const WAN_TO_TWD: f64 = 10_000.0;

/// localStorage 持久化欄位 key（集中管理，避免讀取/寫入兩處字串打錯導致對不上）。
pub(crate) const LS_KEY_START_AGE: &str = "fs_start_age";
pub(crate) const LS_KEY_CURRENT_AGE: &str = "fs_current_age";
pub(crate) const LS_KEY_TARGET_AGE: &str = "fs_target_age";
pub(crate) const LS_KEY_HIST_INV_MODE: &str = "fs_hist_inv_mode";
pub(crate) const LS_KEY_H_INV_K: &str = "fs_h_inv_k";
pub(crate) const LS_KEY_HIST_TOTAL_WAN: &str = "fs_hist_total_wan";
pub(crate) const LS_KEY_ASSET_WAN: &str = "fs_asset_wan";
pub(crate) const LS_KEY_FUTURE_MODE: &str = "fs_future_mode";
pub(crate) const LS_KEY_F_INV_K: &str = "fs_f_inv_k";
pub(crate) const LS_KEY_INFLATION: &str = "fs_inflation";

// =====================================================================
// # 2. 全域狀態與資料結構定義
// =====================================================================
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum FutureMode {
    Stop,
    Invest,
    Withdraw,
}

/// 歷史投入的輸入模式：直接填每月投入，或改填累積總投入成本（由系統換算回每月）。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum HistInvMode {
    Monthly,
    Total,
}

#[derive(Clone, PartialEq)]
pub(crate) struct ChartInput {
    pub(crate) start_age: usize,
    pub(crate) total_years: usize,
    pub(crate) hist_years: usize,
    pub(crate) h_inv: f64,
    pub(crate) anchor_roi_pct: Option<f64>,
    pub(crate) lump_sum: f64,
    pub(crate) f_inv: f64,
    pub(crate) inflation_rate: f64,
    pub(crate) window_width: u32,
}

impl ChartInput {
    pub(crate) fn anchor_roi_pct(&self) -> f64 {
        self.anchor_roi_pct.unwrap_or(DEFAULT_ANCHOR_ROI_PCT)
    }

    pub(crate) fn h_inv_sum(&self) -> f64 {
        self.h_inv * (self.hist_years * 12) as f64
    }

    pub(crate) fn target_age(&self) -> usize {
        self.start_age + self.total_years
    }
}

/// 代表單一條資產成長軌道
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TrendRoute {
    /// 該條軌道所使用的精確年化報酬率 (例如 0.0, 5.4, 20.0)
    pub(crate) roi_pct: f64,
    /// 是否為使用者指定的主線錨定點
    pub(crate) is_anchor: bool,
    /// 軌道上每個月的名目與實質資產價值: Vec<(名目資產, 實質資產)>
    pub(crate) data: Vec<(f64, f64)>,
}

// =====================================================================
// # 3. 核心金融數學計算引擎
// =====================================================================

/// 由「目前資產 / 歷史月投 / 已投月數」以二分搜尋反推隱含年化報酬率（%）。
/// 搜尋範圍 -99% ~ 50%，迭代 100 次，精確到約 0.01%。
///
/// - `current_asset` 允許為負（虧損或帶債起步）
/// - 回傳 `None` 代表無法計算（`hist_months == 0` 或 `h_inv <= 0`）
pub(crate) fn infer_roi_pct(current_asset: f64, h_inv: f64, hist_months: usize) -> Option<f64> {
    if hist_months == 0 || h_inv <= 0.0 {
        return None;
    }
    let mut lo = -99.0_f64;
    let mut hi = 50.0_f64;
    for _ in 0..64 {
        let mid = (lo + hi) * 0.5;
        let r = (1.0 + mid * 0.01).powf(1.0 / 12.0) - 1.0;
        let mut bal = 0.0;
        let r_plus_1 = 1.0 + r;
        for _ in 0..hist_months {
            bal = (bal + h_inv) * r_plus_1;
        }
        if bal < current_asset {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let final_roi = (lo + hi) * 0.5;
    Some(if final_roi.abs() < 0.005 {
        0.0
    } else {
        final_roi
    })
}

pub(crate) fn calculate_true_pivot_trends(
    h_inv: f64,          // 歷史每月投入（元）
    f_inv: f64,          // 未來每月投入/提領（元）
    anchor_roi_pct: f64, // 外部傳入的精確浮點數年化 ROI
    inflation_rate: f64, // 未來通膨率
    hist_years: usize,   // 歷史年期
    total_years: usize,  // 總模擬年期
    lump_sum: f64,       // 現有資產結算點（元）
) -> Vec<TrendRoute> {
    let hist_months = hist_years * 12;
    let total_months = total_years * 12;
    let future_months = total_months.saturating_sub(hist_months);

    // 換算月化複合利率
    let anchor_monthly_rate = (1.0 + (anchor_roi_pct / 100.0)).powf(1.0 / 12.0) - 1.0;
    let inflation_monthly_rate = (1.0 + (inflation_rate / 100.0)).powf(1.0 / 12.0) - 1.0;

    // 1. 歷史階段
    let mut hist_route = Vec::with_capacity(hist_months + 1);

    if hist_months == 0 {
        hist_route.push((lump_sum, lump_sum));
    } else {
        let mut curr_balance = 0.0;
        hist_route.push((curr_balance, curr_balance));
        for _ in 1..=hist_months {
            curr_balance = (curr_balance + h_inv) * (1.0 + anchor_monthly_rate);
            hist_route.push((curr_balance, curr_balance));
        }

        // 末點鎖定為使用者輸入的現有資產
        if let Some(last_node) = hist_route.last_mut() {
            *last_node = (lump_sum, lump_sum);
        }
    }

    let initial_asset = hist_route.last().map(|&(_, real)| real).unwrap_or(0.0);

    // 2. 收集所有年化報酬率（HashMap 去重：anchor 為整數時不增加額外軌道）
    let mut target_rois: HashMap<i32, f64> = (0..=20)
        .map(|r| (r * 1000, r as f64)) // 放大 1000 倍作為 Key 規避浮點數 Hash 問題
        .collect();

    // 主線 ROI 放大 1000 倍取整數作為 key，避免浮點 hash 碰撞
    let anchor_key = (anchor_roi_pct * 1000.0).round() as i32;
    target_rois.insert(anchor_key, anchor_roi_pct);

    let mut routes = Vec::with_capacity(target_rois.len());

    // 3. 逐條計算複利軌道
    for (&_key, &roi) in target_rois.iter() {
        let is_anchor = (roi - anchor_roi_pct).abs() < f64::EPSILON;
        let monthly_rate = (1.0 + (roi / 100.0)).powf(1.0 / 12.0) - 1.0;

        let mut full_route = Vec::with_capacity(total_months + 1);
        full_route.extend_from_slice(&hist_route);

        let mut curr_nominal = initial_asset;
        let mut future_inflation_factor = 1.0;

        for _ in 1..=future_months {
            future_inflation_factor *= 1.0 + inflation_monthly_rate;

            // 投入：名目固定；提領：名目隨通膨調整以維持實質購買力
            let actual_nominal_cashflow = if f_inv >= 0.0 {
                f_inv
            } else {
                f_inv * future_inflation_factor
            };

            curr_nominal = if curr_nominal > 0.0 {
                (curr_nominal + actual_nominal_cashflow) * (1.0 + monthly_rate)
            } else {
                curr_nominal + actual_nominal_cashflow
            };
            let curr_real = curr_nominal / future_inflation_factor;

            full_route.push((curr_nominal, curr_real));
        }

        routes.push(TrendRoute {
            roi_pct: roi,
            is_anchor,
            data: full_route,
        });
    }

    // 4. 依報酬率升序排列
    routes.sort_by(|a, b| {
        a.roi_pct
            .partial_cmp(&b.roi_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    routes
}

thread_local! {
    static DEBOUNCE_TIMER: Cell<i32> = const { Cell::new(-1) };
}

// =====================================================================
// # 1. localStorage 持久化輔助工具
// =====================================================================

fn ls_get(key: &str) -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|ls| ls.get_item(key).ok().flatten())
}

fn ls_set(key: &str, value: &str) {
    if let Some(ls) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = ls.set_item(key, value);
    }
}

fn ls_usize(key: &str, default: usize) -> usize {
    ls_get(key).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn ls_f64(key: &str, default: f64) -> f64 {
    ls_get(key).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn ls_str(key: &str, default: &str) -> String {
    ls_get(key).unwrap_or_else(|| default.to_string())
}

// =====================================================================
// # 2. 歷史文字摘要衍生計算
// =====================================================================

fn derive_future_summary(
    ci: &ChartInput,
    sorted_trends: &[TrendRoute],
    asset_wan_val: f64,
    future_mode_val: FutureMode,
) -> (String, String, String) {
    let anchor_route = sorted_trends.iter().find(|route| route.is_anchor);
    let total_months = ci.total_years * 12;
    let (nom, real) = anchor_route
        .and_then(|route| route.data.get(total_months))
        .copied()
        .unwrap_or((0.0, 0.0));

    let future_desc = match future_mode_val {
        FutureMode::Stop => "未來不再投入".to_string(),
        FutureMode::Invest => format!("未來每月投入 {}", format_twd_financial(ci.f_inv.abs())),
        FutureMode::Withdraw => format!("未來每月提領 {}", format_twd_financial(ci.f_inv.abs())),
    };

    let end_str = if nom <= 0.0 {
        let mut bankruptcy_text =
            format!("⚠️ 資產恐將耗盡（名目終值 {}）", format_twd_financial(nom));

        if let Some(route) = anchor_route
            && let Some(b_idx) = route
                .data
                .iter()
                .position(|&(nominal_bal, _)| nominal_bal < 0.0)
        {
            let exact_age = ci.start_age + (b_idx / 12);
            let b_months = b_idx % 12;

            bankruptcy_text = if b_months > 0 {
                format!(
                    "🚨 警告：資產預計將在 <strong style='color:#F43F5E;'>{} 歲 {} 個月</strong> 時提早耗盡歸零！",
                    exact_age, b_months
                )
            } else {
                format!(
                    "🚨 警告：資產預計將在 <strong style='color:#F43F5E;'>{} 歲整</strong> 時提早耗盡歸零！",
                    exact_age
                )
            };
        }

        bankruptcy_text
    } else {
        let real_str = if ci.inflation_rate > 0.0 {
            format!(
                "，實質購買力約 <strong>{}</strong>",
                format_twd_financial(real)
            )
        } else {
            String::new()
        };
        format!(
            "資產累積名目約 <strong>{}</strong>{}",
            format_twd_financial(nom),
            real_str
        )
    };

    let av = asset_wan_val * WAN_TO_TWD;
    let roi_info = if av != 0.0 {
        if let Some(actual_roi) = ci.anchor_roi_pct {
            let roi_style = if actual_roi < 0.0 {
                "style='color: #F43F5E; font-weight: bold;'"
            } else {
                "style='color: #FFFFFF; font-weight: bold;'"
            };

            format!(
                "，現有資產 <strong>{}</strong> (≈隱含年化 <span {}>{:.2}%</span>)",
                format_twd_financial(av),
                roi_style,
                actual_roi
            )
        } else {
            format!("，現有資產 <strong>{}</strong>", format_twd_financial(av))
        }
    } else {
        String::new()
    };

    (future_desc, end_str, roi_info)
}

// =====================================================================
// # 3. Leptos App 元件主結構
// =====================================================================
#[component]
pub(crate) fn App() -> impl IntoView {
    // 從 localStorage 讀取上次設定
    let init_start_age = ls_usize(LS_KEY_START_AGE, 25).clamp(0, MAX_AGE);
    let init_current_age = ls_usize(LS_KEY_CURRENT_AGE, 35).clamp(init_start_age, MAX_AGE);
    let init_target_age =
        ls_usize(LS_KEY_TARGET_AGE, 65).clamp((init_current_age + 1).min(MAX_AGE + 1), MAX_AGE + 1);
    let init_hist_inv_mode = match ls_str(LS_KEY_HIST_INV_MODE, "monthly").as_str() {
        "total" => HistInvMode::Total,
        _ => HistInvMode::Monthly,
    };
    let init_h_inv_k = ls_f64(LS_KEY_H_INV_K, 10.0).max(0.0);
    let init_hist_total_wan = ls_f64(LS_KEY_HIST_TOTAL_WAN, 0.0).max(0.0);
    let init_asset_wan = ls_f64(LS_KEY_ASSET_WAN, 150.0).max(0.0);
    let init_future_mode = match ls_str(LS_KEY_FUTURE_MODE, "stop").as_str() {
        "invest" => FutureMode::Invest,
        "withdraw" => FutureMode::Withdraw,
        _ => FutureMode::Stop,
    };
    let init_f_inv_k = ls_f64(LS_KEY_F_INV_K, 0.0).max(0.0);
    let init_inflation = ls_f64(LS_KEY_INFLATION, 2.0).clamp(0.0, 10.0);

    // Signals
    let (start_age, set_start_age) = signal(init_start_age);
    let (current_age, set_current_age) = signal(init_current_age);
    let (target_age, set_target_age) = signal(init_target_age);
    let (hist_inv_mode, set_hist_inv_mode) = signal(init_hist_inv_mode);
    let (h_inv_k, set_h_inv_k) = signal(init_h_inv_k);
    let (hist_total_wan, set_hist_total_wan) = signal(init_hist_total_wan);
    let (asset_wan, set_asset_wan) = signal(init_asset_wan);
    let (future_mode, set_future_mode) = signal(init_future_mode);
    let (f_inv_k, set_f_inv_k) = signal(init_f_inv_k);
    let (inflation_rate, set_inflation_rate) = signal(init_inflation);
    let (panel_open, set_panel_open) = signal(true);

    // 字串暫存 signals：讓使用者打字中途不被卡死（blur 後才做範圍驗證）
    let (start_age_raw, set_start_age_raw) = signal(init_start_age.to_string());
    let (current_age_raw, set_current_age_raw) = signal(init_current_age.to_string());
    let (target_age_raw, set_target_age_raw) = signal(init_target_age.to_string());
    let (h_inv_k_raw, set_h_inv_k_raw) = signal(init_h_inv_k.to_string());
    let (hist_total_wan_raw, set_hist_total_wan_raw) = signal(init_hist_total_wan.to_string());
    let (asset_wan_raw, set_asset_wan_raw) = signal(init_asset_wan.to_string());
    let (f_inv_k_raw, set_f_inv_k_raw) = signal(init_f_inv_k.to_string());
    let (inflation_rate_raw, set_inflation_rate_raw) = signal(format!("{:.1}", init_inflation));

    // 衍生計算值
    let hist_years = move || current_age.get().saturating_sub(start_age.get());
    let is_hist_years_active = move || hist_years() > 0 && current_age.get() >= start_age.get();

    let total_years = move || {
        let raw_total = target_age.get().saturating_sub(start_age.get());
        let hy = hist_years();
        if current_age.get() < start_age.get() || target_age.get() <= current_age.get() {
            hy.max(1) // 打字中的暫時矛盾：維持最小有效值
        } else {
            raw_total.max(hy).max(1)
        }
    };

    // 歷史每月投入：依輸入模式決定直接取值，或由「總投入成本」平均回推。
    let h_inv = move || match hist_inv_mode.get() {
        HistInvMode::Monthly => h_inv_k.get() * THOUSAND_TO_TWD,
        HistInvMode::Total => {
            let months = (hist_years() * 12) as f64;
            if months > 0.0 {
                (hist_total_wan.get() * WAN_TO_TWD) / months
            } else {
                0.0
            }
        }
    };
    let h_inv_sum = move || match hist_inv_mode.get() {
        // 總投入成本模式下直接採用使用者輸入的總額，避免「先除後乘」的浮點誤差。
        HistInvMode::Total => hist_total_wan.get() * WAN_TO_TWD,
        HistInvMode::Monthly => h_inv() * (hist_years() * 12) as f64,
    };
    let current_asset = move || asset_wan.get() * WAN_TO_TWD;

    let lump_sum = move || current_asset();

    let anchor_roi_pct = move || {
        let hm = hist_years() * 12;
        if hm == 0 || asset_wan.get() == 0.0 || current_age.get() < start_age.get() {
            None
        } else {
            infer_roi_pct(current_asset(), h_inv(), hm)
        }
    };

    let f_inv = move || match future_mode.get() {
        FutureMode::Stop => 0.0,
        FutureMode::Invest => f_inv_k.get() * THOUSAND_TO_TWD,
        FutureMode::Withdraw => -f_inv_k.get() * THOUSAND_TO_TWD,
    };
    // 視窗寬度 signal（響應旋轉 / resize）
    let initial_width: u32 = {
        #[cfg(target_family = "wasm")]
        {
            web_sys::window()
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(1920.0) as u32
        }
        #[cfg(not(target_family = "wasm"))]
        {
            1920
        }
    };
    let (window_width, set_window_width) = signal(initial_width);
    #[cfg(not(target_family = "wasm"))]
    let _ = set_window_width;
    #[cfg(target_family = "wasm")]
    {
        let cb = wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || {
            if let Some(w) = web_sys::window()
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
            {
                set_window_width.set(w as u32);
            }
        });
        if let Some(win) = web_sys::window() {
            let _ = win.add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref());
        }
        cb.forget();
    }

    // 1. 聚合所有參數為 ChartInput（Memo 確保只在依賴變動時重算）
    let active_chart_input_memo = Memo::new(move |_| ChartInput {
        start_age: start_age.get(),
        total_years: total_years(),
        hist_years: hist_years(),
        h_inv: h_inv(),
        anchor_roi_pct: anchor_roi_pct(),
        lump_sum: lump_sum(),
        f_inv: f_inv(),
        inflation_rate: inflation_rate.get(),
        window_width: window_width.get(),
    });

    // 2. 300ms debounce：防止每個字元觸發重算
    let (debounced_chart_input, set_debounced_chart_input) =
        signal(active_chart_input_memo.get_untracked());

    Effect::new(move |_| {
        let new_ci = active_chart_input_memo.get();
        #[cfg(target_family = "wasm")]
        {
            DEBOUNCE_TIMER.with(|id| {
                if let Some(w) = web_sys::window() {
                    let old = id.get();
                    if old >= 0 {
                        w.clear_timeout_with_handle(old);
                    }
                    let cb = wasm_bindgen::closure::Closure::once(move || {
                        set_debounced_chart_input.set(new_ci);
                    });
                    let new_id = w
                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                            cb.as_ref().unchecked_ref(),
                            DEBOUNCE_MS,
                        )
                        .unwrap_or(-1);
                    cb.forget();
                    id.set(new_id);
                }
            });
        }
        #[cfg(not(target_family = "wasm"))]
        {
            set_debounced_chart_input.set(new_ci);
        }
    });

    // 3. 複利計算快取（只依賴 debounced_chart_input，下游多次讀取不重算）
    let trends_memo = Memo::new(move |_| {
        let ci = debounced_chart_input.get();
        calculate_true_pivot_trends(
            ci.h_inv,
            ci.f_inv,
            ci.anchor_roi_pct(),
            ci.inflation_rate,
            ci.hist_years,
            ci.total_years,
            ci.lump_sum,
        )
    });

    // 4. 非同步 Plot resource（從快取讀取 trends，不重複計算）
    let plot_resource = LocalResource::new(move || async move {
        let ci = debounced_chart_input.get();
        let trends = trends_memo.get();
        generate_plot(ci, trends)
    });

    Effect::new(move |_| {
        if let Some(p) = plot_resource.get() {
            #[cfg(target_family = "wasm")]
            {
                let element_id = "financial-graph";
                if let Some(doc) = web_sys::window().and_then(|w| w.document())
                    && doc.get_element_by_id(element_id).is_some()
                {
                    leptos::task::spawn_local(async move {
                        let _ = plotly::bindings::react(element_id, &p).await;
                    });
                }
            }
            #[cfg(not(target_family = "wasm"))]
            {
                let _ = p;
            }
        }
    });

    Effect::new(move |_| {
        let _ = panel_open.get();
        #[cfg(target_family = "wasm")]
        if let Some(window) = web_sys::window() {
            let _ = window.request_animation_frame(&js_sys::Function::new_no_args(
                "setTimeout(function() { if(window.Plotly && document.getElementById('financial-graph')){ Plotly.Plots.resize(document.getElementById('financial-graph')); } }, 50);",
            ));
        }
    });

    // 儲存設定到 localStorage（任一值改變時觸發）
    Effect::new(move |_| {
        ls_set(LS_KEY_START_AGE, &start_age.get().to_string());
        ls_set(LS_KEY_CURRENT_AGE, &current_age.get().to_string());
        ls_set(LS_KEY_TARGET_AGE, &target_age.get().to_string());
        ls_set(
            LS_KEY_HIST_INV_MODE,
            match hist_inv_mode.get() {
                HistInvMode::Monthly => "monthly",
                HistInvMode::Total => "total",
            },
        );
        ls_set(LS_KEY_H_INV_K, &h_inv_k.get().to_string());
        ls_set(LS_KEY_HIST_TOTAL_WAN, &hist_total_wan.get().to_string());
        ls_set(LS_KEY_ASSET_WAN, &asset_wan.get().to_string());
        ls_set(
            LS_KEY_FUTURE_MODE,
            match future_mode.get() {
                FutureMode::Stop => "stop",
                FutureMode::Invest => "invest",
                FutureMode::Withdraw => "withdraw",
            },
        );
        ls_set(LS_KEY_F_INV_K, &f_inv_k.get().to_string());
        ls_set(LS_KEY_INFLATION, &inflation_rate.get().to_string());
    });

    view! {
        <div class="app-container">
            <h2 class="app-title">"人生財務戰略導航：現況資產錨定與未來變革推演模擬器"</h2>

            <div class=move || if panel_open.get() { "controls-panel panel-open" } else { "controls-panel" }>
                <div class="controls-summary"
                    style="display: flex; align-items: center; justify-content: space-between; width: 100%; cursor: pointer; user-select: none;"
                    on:click=move |_| set_panel_open.update(|v| *v = !*v)
                >
                    <span class="summary-title">"⚙️ 模擬參數設定"</span>

                    <div style="display: flex; align-items: center; gap: 12px;">
                        <button
                            type="button"
                            class="clear-storage-btn"
                            style="background-color: #1E293B; color: #94A3B8; border: 1px solid #334155; padding: 4px 10px; border-radius: 4px; font-size: 12px; cursor: pointer; transition: all 0.2s;"
                            on:click=move |ev| {
                                ev.stop_propagation(); // 🛑 重要：防止點擊清除時同時觸發面板收合
                                #[cfg(target_family = "wasm")]
                                if let Some(win) = web_sys::window()
                                    && let Some(ls) = win.local_storage().ok().flatten() {
                                    let _ = ls.clear();
                                    let _ = win.location().reload();
                                }
                            }
                        >
                            "🗑️ 清除記憶紀錄"
                        </button>

                        <span class=move || if panel_open.get() { "panel-status-badge badge-open" } else { "panel-status-badge" }>
                            <span class="badge-text">{move || if panel_open.get() { "收合設定" } else { "修改參數" }}</span>
                            <span class="badge-arrow">"▾"</span>
                        </span>
                    </div>
                </div>
                <Show when=move || panel_open.get()>
                    <div class="controls-body">

                        // Row 1：年齡與歷史投入
                        <div class="controls-grid">

                            <div class="control-group">
                                <label class="control-label">"🗓️ 一：開始投資年齡（歲）"</label>
                                <input type="number" class="number-input"
                                    min="0" max={MAX_AGE.to_string()} step="1" inputmode="numeric"
                                    prop:value=move || start_age_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_start_age_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<usize>() {
                                            set_start_age.set(v);
                                        }
                                    }
                                    on:blur=move |_| {
                                        let v = start_age.get().clamp(0, MAX_AGE);
                                        set_start_age.set(v);
                                        set_start_age_raw.set(v.to_string());
                                        if current_age.get() < v {
                                            set_current_age.set(v);
                                            set_current_age_raw.set(v.to_string());
                                        }
                                        if target_age.get() <= current_age.get() {
                                            let nv = current_age.get() + 1;
                                            set_target_age.set(nv);
                                            set_target_age_raw.set(nv.to_string());
                                        }
                                    }
                                />
                                <div class="input-hint">{move || {
                                    let hy = hist_years();
                                    if hy == 0 { "剛要開始投資".to_string() }
                                    else { format!("已投資 {} 年", hy) }
                                }}</div>
                            </div>

                            <div class="control-group">
                                <label class="control-label">"🎂 二：目前年齡（歲）"</label>
                                <input type="number" class="number-input"
                                    min="0" max={MAX_AGE.to_string()} step="1" inputmode="numeric"
                                    prop:value=move || current_age_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_current_age_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<usize>() {
                                            set_current_age.set(v);
                                        }
                                    }
                                    on:blur=move |_| {
                                        let v = current_age.get().clamp(start_age.get(), MAX_AGE);
                                        set_current_age.set(v);
                                        set_current_age_raw.set(v.to_string());
                                        if target_age.get() <= v {
                                            let nv = v + 1;
                                            set_target_age.set(nv);
                                            set_target_age_raw.set(nv.to_string());
                                        }
                                    }
                                />
                                <div class="input-hint">{move || {
                                    let fy = total_years().saturating_sub(hist_years());
                                    format!("距目標還有 {} 年", fy)
                                }}</div>
                            </div>

                            <div class="control-group">
                                <label class="control-label">"🏁 三：目標年齡（歲）"</label>
                                <input type="number" class="number-input"
                                    min="0" max={(MAX_AGE + 1).to_string()} step="1" inputmode="numeric"
                                    prop:value=move || target_age_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_target_age_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<usize>() {
                                            set_target_age.set(v);
                                        }
                                    }
                                    on:blur=move |_| {
                                        let min_allowed = (current_age.get() + 1).min(MAX_AGE + 1);
                                        let v = target_age.get().clamp(min_allowed, MAX_AGE + 1);
                                        set_target_age.set(v);
                                        set_target_age_raw.set(v.to_string());
                                    }
                                />
                                <div class="input-hint">{move || format!("共模擬 {} 年", total_years())}</div>
                            </div>

                            <Show when=move || is_hist_years_active()>
                                <div class="control-group">
                                    <label class="control-label">{move || match hist_inv_mode.get() {
                                        HistInvMode::Monthly => "💰 四：歷史投入（千元）",
                                        HistInvMode::Total => "💰 四：歷史投入（萬元）",
                                    }}</label>
                                    <div class="input-with-toggle">
                                        <button
                                            type="button"
                                            class="hist-mode-toggle"
                                            title=move || match hist_inv_mode.get() {
                                                HistInvMode::Monthly => "切換為：總成本輸入",
                                                HistInvMode::Total => "切換為：每月投入輸入",
                                            }
                                            on:click=move |_| {
                                                let current_mode = hist_inv_mode.get();
                                                let months = (hist_years() * 12) as f64;

                                                match current_mode {
                                                    HistInvMode::Monthly => {
                                                        // 1. 從「每月」切換到「總額」
                                                        // 算出當前實際總投入（元）
                                                        let current_total_twd = h_inv_k.get() * THOUSAND_TO_TWD * months;
                                                        // 換算為萬元單位
                                                        let new_total_wan = current_total_twd / WAN_TO_TWD;
                                                        // 四捨五入
                                                        let rounded_wan = new_total_wan .round();

                                                        // 同步更新總額相關的 Signals
                                                        set_hist_total_wan.set(rounded_wan);
                                                        set_hist_total_wan_raw.set(rounded_wan.to_string());

                                                        // 執行模式切換
                                                        set_hist_inv_mode.set(HistInvMode::Total);
                                                    }
                                                    HistInvMode::Total => {
                                                        // 2. 從「總額」切換到「每月」
                                                        let new_h_inv_k = if months > 0.0 {
                                                            let current_total_twd = hist_total_wan.get() * WAN_TO_TWD;
                                                            // 算回每月投入並換算成千元單位
                                                            (current_total_twd / months) / THOUSAND_TO_TWD
                                                        } else {
                                                            0.0
                                                        };
                                                        // 四捨五入到整數千元
                                                        let rounded_k = new_h_inv_k.round();

                                                        // 同步更新每月相關的 Signals
                                                        set_h_inv_k.set(rounded_k);
                                                        set_h_inv_k_raw.set(rounded_k.to_string());

                                                        // 執行模式切換
                                                        set_hist_inv_mode.set(HistInvMode::Monthly);
                                                    }
                                                }
                                            }
                                        >{move || match hist_inv_mode.get() {
                                            HistInvMode::Monthly => "📅 每月投入",
                                            HistInvMode::Total => "📦 總成本",
                                        }}</button>
                                        <Show when=move || hist_inv_mode.get() == HistInvMode::Monthly>
                                            <input type="number" class="number-input"
                                                min="0" max="99999" step="1" inputmode="decimal"
                                                prop:value=move || h_inv_k_raw.get()
                                                on:input=move |ev| {
                                                    let val = event_target_value(&ev);
                                                    set_h_inv_k_raw.set(val.clone());
                                                    if let Ok(v) = val.parse::<f64>() {
                                                        set_h_inv_k.set(v.max(0.0));
                                                    }
                                                }
                                                on:blur=move |_| {
                                                    let v = h_inv_k.get().max(0.0);
                                                    set_h_inv_k.set(v);
                                                    set_h_inv_k_raw.set(v.to_string());
                                                }
                                            />
                                        </Show>
                                        <Show when=move || hist_inv_mode.get() == HistInvMode::Total>
                                            <input type="number" class="number-input"
                                                min="0" step="1" inputmode="decimal"
                                                prop:value=move || hist_total_wan_raw.get()
                                                on:input=move |ev| {
                                                    let val = event_target_value(&ev);
                                                    set_hist_total_wan_raw.set(val.clone());
                                                    if let Ok(v) = val.parse::<f64>() {
                                                        set_hist_total_wan.set(v.max(0.0));
                                                    }
                                                }
                                                on:blur=move |_| {
                                                    let v = hist_total_wan.get().max(0.0);
                                                    set_hist_total_wan.set(v);
                                                    set_hist_total_wan_raw.set(v.to_string());
                                                }
                                            />
                                        </Show>
                                    </div>
                                    <div class="input-hint">{move || match hist_inv_mode.get() {
                                        HistInvMode::Monthly => {
                                            if h_inv_k.get() == 0.0 {
                                                "尚未輸入歷史每月投入金額".to_string()
                                            } else {
                                                format!("= {} 共投入 {}", format_twd_financial(h_inv()), format_twd_financial(h_inv_sum()))
                                            }
                                        }
                                        HistInvMode::Total => {
                                            if hist_total_wan.get() == 0.0 {
                                                "尚未輸入歷史總投入成本".to_string()
                                            } else {
                                                format!("= {} 約等同每月投入 {}", format_twd_financial(h_inv_sum()), format_twd_financial(h_inv()))
                                            }
                                        }
                                    }}</div>
                                </div>
                            </Show>
                            <div class="control-group">
                                <label class="control-label">
                                    {move || if hist_years() > 0 {
                                        "🎯 五：現有資產（萬元）"
                                    } else {
                                        "💰 四：起始資金（萬元）"
                                    }}
                                </label>
                                <input type="number" class="number-input"
                                    min="0"
                                    step="1" inputmode="decimal"
                                    prop:value=move || asset_wan_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_asset_wan_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<f64>() {
                                            set_asset_wan.set(v.max(0.0));
                                        }
                                    }
                                    on:blur=move |_| {
                                        let v = asset_wan.get().max(0.0);
                                        set_asset_wan.set(v);
                                        set_asset_wan_raw.set(v.to_string());
                                    }
                                />
                                {move || {
                                    let hy = hist_years();
                                    let av = asset_wan.get();
                                    let asset_twd = av * WAN_TO_TWD;

                                    if hy == 0 {
                                        // 註：輸入框已限制 av ≥ 0（不開放負債起步），故只需處理 0 / 正值兩種情況
                                        let hint = if av == 0.0 {
                                            "0 = 從零開始（不帶資金）".to_string()
                                        } else {
                                            format!("= {}，做為複利起始本金", format_twd_financial(asset_twd))
                                        };
                                        view! { <div class="input-hint">{hint}</div> }.into_any()
                                    } else if av == 0.0 {
                                        view! {
                                            <div class="input-hint">"輸入資產以自動推估歷史報酬率"</div>
                                        }.into_any()
                                    } else {
                                        match anchor_roi_pct() {
                                            None => view! { <div class="input-hint warning">"⚠️ 無法推估，請確認數字"</div> }.into_any(),
                                            Some(r) if !(ROI_WARNING_MIN..=ROI_WARNING_MAX).contains(&r) => view! { <div class="input-hint warning">{format!("🚨 隱含 {:.2}%/年，請確認", r)}</div> }.into_any(),
                                            Some(r) if r < 0.0 => view! { <div class="input-hint warning">{format!("⚠️ 隱含 {:.2}%/年，虧損中", r)}</div> }.into_any(),
                                            Some(r) => view! { <div class="input-hint info">{format!("≈ 年化 {:.2}%", r)}</div> }.into_any(),
                                        }
                                    }
                                }}
                            </div>
                        </div>

                        // Row 2：未來計畫與通膨率
                        <div class="controls-grid controls-grid-future">

                            <div class="control-group control-group-wide">
                                <label class="control-label">
                                    {move || if hist_years() > 0 { "🔵 六：未來計畫金額（千元）" } else { "🔵 五：未來計畫金額（千元）" }}
                                </label>
                                <div class="toggle-group">
                                    <button
                                        class=move || if future_mode.get() == FutureMode::Stop { "toggle-btn active stop" } else { "toggle-btn" }
                                        on:click=move |_| set_future_mode.set(FutureMode::Stop)
                                    >"🛑 停止投入"</button>
                                    <button
                                        class=move || if future_mode.get() == FutureMode::Invest { "toggle-btn active invest" } else { "toggle-btn" }
                                        on:click=move |_| set_future_mode.set(FutureMode::Invest)
                                    >"💰 繼續投入"</button>
                                    <button
                                        class=move || if future_mode.get() == FutureMode::Withdraw { "toggle-btn active withdraw" } else { "toggle-btn" }
                                        on:click=move |_| set_future_mode.set(FutureMode::Withdraw)
                                    >"💸 開始提領"</button>
                                </div>
                                <Show when=move || future_mode.get() != FutureMode::Stop>
                                    <div class="toggle-amount">
                                        <input type="number" class="number-input"
                                            min="0" max="99999" step="1" inputmode="decimal"
                                            prop:value=move || f_inv_k_raw.get()
                                            on:input=move |ev| {
                                                let val = event_target_value(&ev);
                                                set_f_inv_k_raw.set(val.clone());
                                                if let Ok(v) = val.parse::<f64>() {
                                                    set_f_inv_k.set(v.max(0.0));
                                                }
                                            }
                                            on:blur=move |_| {
                                                let v = f_inv_k.get().max(0.0);
                                                set_f_inv_k.set(v);
                                                set_f_inv_k_raw.set(v.to_string());
                                            }
                                        />
                                        <div class="input-hint">{move || {
                                            let amt = format_twd_financial(f_inv_k.get() * THOUSAND_TO_TWD);
                                            let amt_yearly = format_twd_financial(f_inv_k.get() * 12.0 * THOUSAND_TO_TWD);
                                            match future_mode.get() {
                                                FutureMode::Invest   => format!("= 未來每月名目投入 {} 等同每年 {}", amt, amt_yearly),
                                                FutureMode::Withdraw => format!("= 未來每月實質提領 {} 等同每年 {}", amt, amt_yearly),
                                                FutureMode::Stop     => String::new(),
                                            }
                                        }}</div>
                                    </div>
                                </Show>
                            </div>

                            <div class="control-group">
                                <label class="control-label">
                                    {move || if hist_years() > 0 { "📉 七：未來通膨率（%）" } else { "📉 六：未來通膨率（%）" }}
                                </label>
                                <input type="number" class="number-input"
                                    min="0.0" max="10.0" step="0.1" inputmode="decimal"
                                    // 初始化只給一次值，打字時絕對不要讓 Leptos 去改變這個 value 屬性
                                    value=inflation_rate_raw.get_untracked()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_inflation_rate_raw.set(val.clone());

                                        // 放行所有包含小數點的輸入，只要最後能解析成功就偷偷更新後台計算流
                                        if let Ok(v) = val.parse::<f64>() {
                                            set_inflation_rate.set(v.clamp(0.0, 10.0));
                                        }
                                    }
                                    on:blur=move |#[allow(unused)]ev| {
                                        // 只有在滑鼠移開時，才強行把輸入框裡面的字刷新成標準格式（例如 2.5）
                                        let v = inflation_rate.get().clamp(0.0, 10.0);
                                        set_inflation_rate.set(v);
                                        let formatted = format!("{:.1}", v);
                                        set_inflation_rate_raw.set(formatted.clone());

                                        #[cfg(target_family = "wasm")]
                                        {
                                            // 手動去修改 DOM 節點的值，此時使用者已經沒在打字，所以不會卡死
                                            if let Some(target) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) {
                                                target.set_value(&formatted);
                                            }
                                        }
                                    }
                                />
                                <div class="input-hint">{move || {
                                    if inflation_rate.get() <= f64::EPSILON {
                                        "🚫 不考慮通膨影響".to_string()
                                    } else {
                                        format!("預期貨幣購買力每年貶值 {:.1}%", inflation_rate.get())
                                    }
                                }}</div>
                            </div>

                        </div>
                    </div>
                </Show>
            </div>
            <div class="summary-card">
                {move || {
                    let ci = debounced_chart_input.get();
                    let trends = trends_memo.get();
                    let tage = ci.target_age();

                    let (future_desc, end_str, roi_info) = derive_future_summary(
                        &ci, &trends, asset_wan.get(), future_mode.get()
                    );

                    let roi = ci.anchor_roi_pct();

                    match (ci.hist_years, ci.lump_sum == 0.0) {
                        // 情境 A：無歷史、無起始本金
                        (0, true) => {
                            let intro = if ci.start_age == 0 { "👶 幫新生兒從 0 歲白手起家 — " } else { "📊 規劃從 " };
                            view! { <span class="summary-text">
                                {intro} {if ci.start_age > 0 { format!("{} 歲出發 — ", ci.start_age) } else { "".to_string() }}
                                {future_desc} "，以系統基準預估年化回報 " <strong>{format!("{roi:.2}")} "%"</strong> " 在 "
                                <strong>{tage}</strong> " 歲時，" <span inner_html=end_str />
                            </span> }.into_any()
                        },

                        // 情境 B：無歷史、有起始本金（單筆配置）
                        (0, false) => {
                            let intro = if ci.start_age == 0 { "👶 幫小孩從 0 歲配置 — " } else { "💰 規劃從 " };
                            let ls_desc = format!("起始本金 <strong>{}</strong>", format_twd_financial(ci.lump_sum));
                            view! { <span class="summary-text">
                                {intro} {if ci.start_age > 0 { format!("{} 歲配置 — ", ci.start_age) } else { "".to_string() }}
                                <span inner_html=ls_desc /> "，" {future_desc} "，並以基準預估年化回報 " <strong>{format!("{roi:.2}")} "%"</strong> " 在 "
                                <strong>{tage}</strong> " 歲時，" <span inner_html=end_str />
                            </span> }.into_any()
                        },

                        // 情境 C：有歷史投資記錄
                        (_hist, _) => {
                            let strategy_desc = if ci.anchor_roi_pct.is_some() {
                                format!("。延續此回報率並調整戰略為：{future_desc}，期望在 ", )
                            } else {
                                format!("。目前無隱含報酬，調整戰略為：{future_desc}，並以基準預估年化回報 {roi:.2}% 期望在 ")
                            };

                            view! { <span class="summary-text">
                                "自 " {ci.start_age} " 歲起每月投資 " {format_twd_financial(ci.h_inv)}
                                " 共投入 " <strong>{format_twd_financial(ci.h_inv_sum())}</strong>
                                "，至今已投 " {ci.hist_years} " 年" <span inner_html=roi_info />
                                {strategy_desc} <strong>{tage}</strong> " 歲時，" <span inner_html=end_str />
                            </span> }.into_any()
                        }
                    }
                }}
            </div>
            <div class="chart-header">
                <button class="screenshot-btn" title="下載圖表 PNG（1920×1080）"
                    on:click=move |_| {
                        #[cfg(target_family = "wasm")]
                        { let _ = js_sys::eval("Plotly.downloadImage(document.getElementById('financial-graph'),{format:'png',width:1920,height:1080,filename:'financial_simulator'})"); }
                    }
                >"📷 截圖"</button>
            </div>

            <div id="financial-graph" class="graph-container"
                style=move || if panel_open.get() { "height: clamp(520px, 68vh, 720px);" } else { "height: clamp(520px, 82vh, 860px);" }
            ></div>

            <div class="chart-footer-notes">
                <p class="note-item">
                    "💡 " <b>"導航小提示："</b>
                    "名目金額代表未來實際看到的數字；折現（實質金額）則是扣除通膨率後，換算回「現在這一刻」的實質購買力。當您選擇提領時，系統會自動將提領金額隨通膨率調升，以保障您的實質生活水平。資產或終值若出現負數，表示該情境下資金已耗盡並進入負債。"
                </p>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_true_pivot_trends_basic_structure() {
        let h_inv = 10000.0; // 歷史每月投入 1 萬
        let f_inv = 20000.0; // 未來每月改投 2 萬
        let anchor_roi_pct = 10.5; // 精確的 f64 歷史年化報酬率 (非整數)
        let inflation_rate = 2.0;
        let hist_years = 5;
        let total_years = 15;
        let lump_sum = 2000000.0; // 現有資產 200 萬

        let trends = calculate_true_pivot_trends(
            h_inv,
            f_inv,
            anchor_roi_pct,
            inflation_rate,
            hist_years,
            total_years,
            lump_sum,
        );

        // 🎯 驗證結構：因為 10.5% 不是整數，所以除了 0..=20 共 21 條整數線外，會額外外掛 1 條精確主線，總共 22 條！
        assert_eq!(trends.len(), 22);

        // 驗證是否按報酬率由小到大嚴格排序
        for i in 0..trends.len() - 1 {
            assert!(
                trends[i].roi_pct <= trends[i + 1].roi_pct,
                "軌道未按報酬率由小到大排序！位置 {}: {}, 位置 {}: {}",
                i,
                trends[i].roi_pct,
                i + 1,
                trends[i + 1].roi_pct
            );
        }

        // 驗證總時間序列長度 (15年 * 12個月 + 1個起始點 = 181)
        let expected_months = total_years * 12 + 1;

        // 尋找精確主線，驗證其長度
        let anchor_route = trends
            .iter()
            .find(|r| r.is_anchor)
            .expect("必須找到精確主線");
        assert_eq!(anchor_route.data.len(), expected_months);
        assert!((anchor_route.roi_pct - 10.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_historical_period_consistency() {
        let h_inv = 30000.0;
        let f_inv = 0.0;
        let anchor_roi_pct = 8.35; // 精確歷史 ROI
        let inflation_rate = 3.0;
        let hist_years = 10;
        let total_years = 30;
        let lump_sum = 5000000.0; // 現有資產 500 萬

        let trends = calculate_true_pivot_trends(
            h_inv,
            f_inv,
            anchor_roi_pct,
            inflation_rate,
            hist_years,
            total_years,
            lump_sum,
        );

        let hist_months = hist_years * 12;

        // 🎯 金融鐵律 1：在歷史期間（已發生），「名目資產」必定等於「實質資產」
        // 我們直接對包含主線在內的所有軌道進行全面驗證
        for route in trends.iter() {
            for m in 0..=hist_months {
                let (nominal, real) = route.data[m];
                assert!(
                    (nominal - real).abs() < 1e-4,
                    "歷史期間（第 {} 個月）名目與實質應完全相等。ROI: {}, 名目: {}, 實質: {}",
                    m,
                    route.roi_pct,
                    nominal,
                    real
                );
            }
        }

        // 🎯 金融鐵律 2：在歷史結算點（含）以前，所有預測軌跡必須百分之百重合，消滅分叉！
        // 我們以主線 (is_anchor) 作為物理基準錨定點進行比對
        let anchor_route = trends.iter().find(|r| r.is_anchor).expect("找不到主線");
        for m in 0..=hist_months {
            let anchor_val = anchor_route.data[m];
            for route in trends.iter() {
                let current_val = route.data[m];
                assert!(
                    (current_val.0 - anchor_val.0).abs() < 1e-4,
                    "歷史期間所有 ROI 軌跡應完全重合。第 {} 個月, 主線({}): {}, 測試線({}): {}",
                    m,
                    anchor_route.roi_pct,
                    anchor_val.0,
                    route.roi_pct,
                    current_val.0
                );
            }
        }

        // 🎯 金融鐵律 3：歷史期的最後一個月（現在結算點），必須精確等於使用者填寫的 lump_sum，像素級鎖定！
        for route in trends.iter() {
            let current_asset = route.data[hist_months].0;
            assert!(
                (current_asset - lump_sum).abs() < 1e-4,
                "歷史結算點終點數值（{}）必須完美等於使用者宣告的資產（{}）。ROI: {}",
                current_asset,
                lump_sum,
                route.roi_pct
            );
        }
    }

    #[test]
    fn test_future_inflation_law() {
        let h_inv = 10000.0;
        let f_inv = -15000.0; // 模擬每月實質提領 1.5 萬
        let anchor_roi_pct = 6.0; // 剛好是整數的情況
        let inflation_rate = 2.0; // 通膨年化 2%
        let hist_years = 0; // 全新起點，直接進入未來
        let total_years = 20;
        let lump_sum = 1000000.0; // 從 100 萬起始資金直接出發

        let trends = calculate_true_pivot_trends(
            h_inv,
            f_inv,
            anchor_roi_pct,
            inflation_rate,
            hist_years,
            total_years,
            lump_sum,
        );

        let total_months = total_years * 12;
        let inflation_monthly_rate = (1.0 + inflation_rate / 100.0).powf(1.0 / 12.0) - 1.0;

        // 🎯 金融鐵律：在未來期間任何一個時間點，名目金額必定等於 實質金額 * 累計通膨率
        for m in 1..=total_months {
            let cumulative_inflation_factor = (1.0 + inflation_monthly_rate).powf(m as f64);

            for route in trends.iter() {
                let (nominal, real) = route.data[m];
                if real.abs() > 0.001 {
                    let calculated_nominal = real * cumulative_inflation_factor;
                    let diff_ratio = (nominal - calculated_nominal).abs() / nominal.abs();
                    assert!(
                        diff_ratio < 1e-4,
                        "未來區間必須嚴格遵循 名目 = 實質 * 累計通膨 的鐵律。第 {} 個月, ROI {}: 名目 {}, 計算值 {}",
                        m,
                        route.roi_pct,
                        nominal,
                        calculated_nominal
                    );
                }
            }
        }
    }

    #[test]
    fn test_zero_years_edge_case() {
        let lump_sum = 3500000.0; // 設定 350 萬一桶金

        // 🎯 測試極端邊界：若歷史年數為 0，第 0 個月應正常初始化為傳入的 lump_sum
        let trends = calculate_true_pivot_trends(20000.0, 20000.0, 7.0, 2.0, 0, 10, lump_sum);

        for route in trends.iter() {
            let (nominal_start, real_start) = route.data[0];
            assert_eq!(nominal_start, lump_sum);
            assert_eq!(real_start, lump_sum);
        }
    }

    #[test]
    fn test_pivot_point_cohesion_and_divergence() {
        let h_inv = 8000.0;
        let f_inv = -5000.0;
        let anchor_roi_pct = 6.5;
        let inflation_rate = 3.0;
        let hist_years = 3;
        let total_years = 10;
        let lump_sum = 500_000.0;

        let trends = calculate_true_pivot_trends(
            h_inv,
            f_inv,
            anchor_roi_pct,
            inflation_rate,
            hist_years,
            total_years,
            lump_sum,
        );

        let hist_months = hist_years * 12;
        let anchor_route = trends.iter().find(|r| r.is_anchor).expect("找不到主線");

        // 🔍 歷史點檢查：在歷史期間內，所有軌道的數值必須與主線完全重合
        for m in 0..=hist_months {
            let anchor_val = anchor_route.data[m];
            for route in trends.iter() {
                assert_eq!(
                    route.data[m], anchor_val,
                    "在第 {} 個月（歷史期），ROI {}% 應該與主線完全重合",
                    m, route.roi_pct
                );
            }
        }

        // 🔍 未來點檢查：超過歷史期後，高低 ROI 軌道必須在未來終點產生合理的發散分叉
        let final_idx = total_years * 12;

        let route_5pct = trends
            .iter()
            .find(|r| (r.roi_pct - 5.0).abs() < f64::EPSILON)
            .expect("找不到 5% 線");
        let route_15pct = trends
            .iter()
            .find(|r| (r.roi_pct - 15.0).abs() < f64::EPSILON)
            .expect("找不到 15% 線");

        let val_5pct = route_5pct.data[final_idx].0;
        let val_15pct = route_15pct.data[final_idx].0;

        assert!(
            val_15pct > val_5pct,
            "未來終點時，15% ROI 的名目資產（{}）應大於 5% ROI 的資產（{}）",
            val_15pct,
            val_5pct
        );
    }

    /// 擴充測試 5：極端環境測試 ── 0 本金、0 投入、0 通膨下的純數學複利驗證
    #[test]
    fn test_zero_environment_compounding() {
        let h_inv = 0.0;
        let f_inv = 0.0;
        let anchor_roi_pct = 10.0;
        let inflation_rate = 0.0;
        let hist_years = 0;
        let total_years = 1; // 模擬 1 年 (12個月)
        let lump_sum = 100_000.0;

        let trends = calculate_true_pivot_trends(
            h_inv,
            f_inv,
            anchor_roi_pct,
            inflation_rate,
            hist_years,
            total_years,
            lump_sum,
        );

        // 🎯 核心修正：從新版有序 Vec 中精確找出 10% 報酬率的軌道
        let route_10pct = trends
            .iter()
            .find(|r| (r.roi_pct - 10.0).abs() < f64::EPSILON)
            .expect("測試中必須能找到 10% 的報酬率軌道");

        let entries = &route_10pct.data;

        // 驗證第 0 個月
        assert_eq!(entries[0].0, 100_000.0);

        // 驗證第 12 個月（1年後）的名目複利數學精確度：100000 * (1 + 0.1) = 110000.00
        let final_nominal = entries[12].0;
        let expected_approx = 110000.00;
        let delta = (final_nominal - expected_approx).abs();
        assert!(
            delta < 1.0,
            "1年複利後的名目資產 {} 與數學預期 {} 差距過大",
            final_nominal,
            expected_approx
        );

        // 因為通膨為 0，名目資產必須完全等於實質資產
        assert_eq!(
            entries[12].0, entries[12].1,
            "當通膨率為 0 時，名目與實質資產必須完全相等"
        );
    }

    /// 擴充測試 6：新版單一結構體 ChartInput 整合繪圖引擎配置校驗
    #[test]
    fn test_chart_input_and_plot_generation() {
        let ci = ChartInput {
            start_age: 35,
            total_years: 10,
            hist_years: 2,
            h_inv: 12000.0,
            anchor_roi_pct: Some(8.5),
            lump_sum: 2_000_000.0,
            f_inv: -8000.0,
            inflation_rate: 2.0,
            window_width: 1200,
        };

        let trends = calculate_true_pivot_trends(
            ci.h_inv,
            ci.f_inv,
            ci.anchor_roi_pct(),
            ci.inflation_rate,
            ci.hist_years,
            ci.total_years,
            ci.lump_sum,
        );

        // 驗證圖表引擎是否能正常吞下結構體，並產出合法的 JSON 配置
        let plot = generate_plot(ci, trends);
        let json_str = plot.to_json();
        assert!(!json_str.is_empty(), "生成的圖表 JSON 配置字串不應為空");
    }

    // =====================================================================
    // # 9. 新增測試 — infer_roi_pct
    // =====================================================================

    /// 基本正確性：已知本金 + 月投 + 月數，算出來的 ROI 反推回去應接近原始值
    #[test]
    fn test_infer_roi_pct_basic_roundtrip() {
        let h_inv = 10_000.0;
        let expected_annual_roi = 8.0_f64;
        let hist_months = 120_usize; // 10 年

        // 用已知 ROI 正向算出期末資產
        let monthly_rate = (1.0 + expected_annual_roi / 100.0).powf(1.0 / 12.0) - 1.0;
        let mut balance = 0.0;
        for _ in 0..hist_months {
            balance = (balance + h_inv) * (1.0 + monthly_rate);
        }

        // 反推應還原到接近 8.0%
        let inferred =
            infer_roi_pct(balance, h_inv, hist_months).expect("已知有效資料不應回傳 None");
        assert!(
            (inferred - expected_annual_roi).abs() < 0.01,
            "反推值 {:.4}% 應接近 {:.2}%",
            inferred,
            expected_annual_roi
        );
    }

    /// 邊界：hist_months = 0 必須回傳 None
    #[test]
    fn test_infer_roi_pct_zero_months_returns_none() {
        assert!(infer_roi_pct(1_000_000.0, 10_000.0, 0).is_none());
    }

    /// 邊界：h_inv <= 0 必須回傳 None（無法除以零，也沒有意義）
    #[test]
    fn test_infer_roi_pct_zero_h_inv_returns_none() {
        assert!(infer_roi_pct(500_000.0, 0.0, 60).is_none());
        assert!(infer_roi_pct(500_000.0, -1000.0, 60).is_none());
    }

    /// 資產 = 0：代表所有月投都虧光，應推算出接近 -100% 的極端負值
    #[test]
    fn test_infer_roi_pct_zero_asset() {
        let result = infer_roi_pct(0.0, 10_000.0, 12);
        // 資產完全歸零代表每月都蒸發，ROI 必然極負
        assert!(result.is_some());
        assert!(result.unwrap() < -50.0, "零資產應推算出極度負值報酬率");
    }

    /// 資產為負數：帶債情況下應能推算出負報酬率
    #[test]
    fn test_infer_roi_pct_negative_asset() {
        let result = infer_roi_pct(-200_000.0, 10_000.0, 60);
        assert!(result.is_some());
        assert!(
            result.unwrap() < 0.0,
            "資產為負時應推算出負報酬率，實際: {:.2}%",
            result.unwrap()
        );
    }

    /// 資產略高於純本金：對應接近 0% 的低報酬
    #[test]
    fn test_infer_roi_pct_near_zero_roi() {
        let h_inv = 10_000.0;
        let months = 12_usize;
        // 純本金：月初投入，月底算（annuity-due），ROI=0 時最後餘額 = h_inv * months
        let principal = h_inv * months as f64;
        let result = infer_roi_pct(principal, h_inv, months).expect("有效輸入不應回傳 None");
        assert!(
            result.abs() < 1.0,
            "純本金對應的 ROI 應接近 0%，實際: {:.4}%",
            result
        );
    }

    /// 精確度：ROI 反推誤差應小於 0.01%
    #[test]
    fn test_infer_roi_pct_precision() {
        // 測試非整數的精確 ROI（12.75%）
        let h_inv = 50_000.0;
        let target_roi = 12.75_f64;
        let hist_months = 240_usize; // 20 年

        let r = (1.0 + target_roi / 100.0).powf(1.0 / 12.0) - 1.0;
        let mut bal = 0.0;
        for _ in 0..hist_months {
            bal = (bal + h_inv) * (1.0 + r);
        }

        let inferred = infer_roi_pct(bal, h_inv, hist_months).unwrap();
        assert!(
            (inferred - target_roi).abs() < 0.01,
            "應達到 0.01% 精度：目標 {target_roi}%，推算 {inferred:.4}%"
        );
    }

    #[test]
    fn test_chart_input_anchor_roi_fallback() {
        let ci_none = ChartInput {
            start_age: 30,
            total_years: 10,
            hist_years: 0,
            h_inv: 10_000.0,
            anchor_roi_pct: None, // 未設定時應 fallback 到 7.0
            lump_sum: 0.0,
            f_inv: 0.0,
            inflation_rate: 0.0,
            window_width: 1920,
        };
        assert_eq!(ci_none.anchor_roi_pct(), DEFAULT_ANCHOR_ROI_PCT);

        let ci_some = ChartInput {
            anchor_roi_pct: Some(12.5),
            ..ci_none
        };
        assert_eq!(ci_some.anchor_roi_pct(), 12.5);
    }

    #[test]
    fn test_chart_input_h_inv_sum() {
        let ci = ChartInput {
            start_age: 25,
            total_years: 20,
            hist_years: 5, // 5 * 12 = 60 個月
            h_inv: 30_000.0,
            anchor_roi_pct: Some(8.0),
            lump_sum: 0.0,
            f_inv: 0.0,
            inflation_rate: 2.0,
            window_width: 1920,
        };
        let expected = 30_000.0 * 60.0;
        assert!(
            (ci.h_inv_sum() - expected).abs() < f64::EPSILON,
            "h_inv_sum 應等於 h_inv * hist_months"
        );
    }

    #[test]
    fn test_trend_route_exactly_one_anchor() {
        let trends = calculate_true_pivot_trends(20_000.0, 0.0, 7.0, 0.0, 5, 20, 1_000_000.0);
        // 恰好整數（7.0%）：anchor 和整數線重合，總共仍是 22 條（7.0 不在 0..=20 的 key 裡）
        // 只需確保 is_anchor 旗標恰好一條
        let anchor_count = trends.iter().filter(|r| r.is_anchor).count();
        assert_eq!(
            anchor_count, 1,
            "任何情況下應恰好有且僅有 1 條 is_anchor 線"
        );
    }

    #[test]
    fn test_trend_route_integer_anchor_deduplication() {
        // 整數 ROI（如 10.0%）應被 HashMap 去重，不重複計算
        let trends = calculate_true_pivot_trends(10_000.0, 0.0, 10.0, 2.0, 5, 15, 500_000.0);
        // 10.0% 是整數，所以 key=10000 對應同一條，總共仍 21 條而不是 22
        assert_eq!(
            trends.len(),
            21,
            "anchor_roi_pct 為整數時應去重，結果應仍為 21 條"
        );
        let anchor_count = trends.iter().filter(|r| r.is_anchor).count();
        assert_eq!(anchor_count, 1);
    }

    #[test]
    fn test_trend_route_sorted_ascending() {
        let trends =
            calculate_true_pivot_trends(30_000.0, -10_000.0, 8.37, 3.0, 10, 30, 3_000_000.0);
        // 所有 roi_pct 應嚴格遞增排序
        for w in trends.windows(2) {
            assert!(
                w[0].roi_pct <= w[1].roi_pct,
                "排序應遞增：{} <= {} 失敗",
                w[0].roi_pct,
                w[1].roi_pct
            );
        }
    }

    // =====================================================================
    // # 13. 新增測試 — 提領耗盡場景
    // =====================================================================

    /// 大量提領：資產應在模擬期間某個時間點降到 0 以下
    #[test]
    fn test_withdrawal_depletion_scenario() {
        // 起始 100 萬，每月提領 5 萬（實質），報酬 5%
        let lump_sum = 1_000_000.0;
        let f_inv = -50_000.0;
        let hist_years = 0;
        let total_years = 5;

        let trends =
            calculate_true_pivot_trends(0.0, f_inv, 5.0, 0.0, hist_years, total_years, lump_sum);

        let anchor = trends.iter().find(|r| r.is_anchor).unwrap();
        let final_val = anchor.data[total_years * 12].0;

        assert!(
            final_val < 0.0,
            "大量提領下 5 年後資產應耗盡（終值 {}），以確保耗盡邏輯運作正常",
            final_val
        );
    }

    /// 小量提領：充足資產 + 高報酬 → 即使提領，資產仍持續成長
    #[test]
    fn test_small_withdrawal_still_grows() {
        // 起始 1000 萬，每月提領 1 萬，報酬 12%
        let lump_sum = 10_000_000.0;
        let f_inv = -10_000.0;

        let trends = calculate_true_pivot_trends(0.0, f_inv, 12.0, 0.0, 0, 10, lump_sum);

        let anchor = trends.iter().find(|r| r.is_anchor).unwrap();
        let final_val = anchor.data[10 * 12].0;

        assert!(
            final_val > lump_sum,
            "充足資產 + 高報酬下，小量提領後 10 年資產應仍成長（終值 {}）",
            final_val
        );
    }

    // =====================================================================
    // # 14. 新增測試 — format_twd_financial 邊界案例
    // =====================================================================

    /// 測試大於等於兆級的巨額邊界
    #[test]
    fn test_format_twd_trillion_values() {
        assert_eq!(format_twd_financial(1_000_000_000_000.0), "1.0兆");
        assert_eq!(format_twd_financial(3_500_000_000_000.0), "3.5兆");
        assert_eq!(format_twd_financial(-12_400_000_000_000.0), "-12.4兆");
    }

    /// 恰好 100,000,000 元的億級邊界
    #[test]
    fn test_format_twd_exactly_one_hundred_million() {
        assert_eq!(format_twd_financial(100_000_000.0), "1.0億");
        assert_eq!(format_twd_financial(123_450_000_000.0), "1,234.5億");
    }

    /// 恰好 10,000 元的邊界
    #[test]
    fn test_format_twd_exactly_ten_thousand() {
        assert_eq!(format_twd_financial(10_000.0), "1萬");
        assert_eq!(format_twd_financial(15_500.0), "1.6萬"); // 四捨五入示意
    }

    /// 9,999 元應屬於「元」級，不跨越萬
    #[test]
    fn test_format_twd_just_below_ten_thousand() {
        assert_eq!(format_twd_financial(9_999.0), "9,999元");
    }

    /// 負值應正確帶負號，且選擇正確的單位
    #[test]
    fn test_format_twd_negative_values() {
        assert_eq!(format_twd_financial(-50_000.0), "-5萬");
        assert_eq!(format_twd_financial(-100_000_000.0), "-1.0億");
    }

    /// 「總投入成本」模式：由總額 ÷ 已投月數，反推每月投入金額（總成本單位為「萬元」）
    #[test]
    fn test_hist_total_mode_derives_monthly_from_total() {
        let hist_years = 5usize;
        let months = (hist_years * 12) as f64;
        let total_cost_wan = 60.0; // 60萬元（單位：萬元）
        let h_inv = (total_cost_wan * WAN_TO_TWD) / months;
        assert!((h_inv - 10_000.0).abs() < 1e-9); // 600,000 / 60 個月 = 每月 1 萬
    }
}
