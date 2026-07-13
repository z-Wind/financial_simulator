use leptos::prelude::*;
use plotly::common::{Anchor, DashType, Font, HoverInfo, Label, Line, TickMode, Title};
use plotly::configuration::DisplayModeBar;
use plotly::layout::{
    Annotation, Axis, AxisType, DragMode, HoverMode, ItemClick, Layout, Legend, Margin, Shape,
    ShapeLayer, ShapeLine, ShapeType,
};
use plotly::{Configuration, Plot, Scatter};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
#[cfg(target_family = "wasm")]
use wasm_bindgen::JsCast;

// =====================================================================
// # 0. 全域狀態型別定義
// =====================================================================

#[derive(Clone, Copy, PartialEq)]
enum FutureMode {
    Stop,
    Invest,
    Withdraw,
}

#[derive(Clone, PartialEq)]
struct ChartInput {
    start_age: usize,
    total_years: usize,
    hist_years: usize,
    h_inv: f64,
    anchor_roi_pct: Option<f64>,
    lump_sum: f64,
    f_inv: f64,
    inflation_rate: usize,
    window_width: u32,
}

impl ChartInput {
    fn anchor_roi_pct(&self) -> f64 {
        self.anchor_roi_pct.unwrap_or(7.0)
    }

    fn h_inv_sum(&self) -> f64 {
        self.h_inv * (self.hist_years * 12) as f64
    }
}

thread_local! {
    static DEBOUNCE_TIMER: Cell<i32> = const { Cell::new(-1) };
}

// =====================================================================
// # 1. localStorage 記憶持久化工具（僅在 WASM 環境有效）
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
// # 2. 核心金融數學引擎（二分搜尋 ROI 與終值計算）
// =====================================================================

/// 由「目前資產 / 歷史月投 / 已投月數」以二分搜尋反推隱含年化報酬率（%）。
/// 搜尋範圍 -99% ~ 50%，迭代 100 次，精確到約 0.01%。
///
/// - `current_asset` 允許為負（虧損或帶債起步）
/// - 回傳 `None` 代表無法計算（`hist_months == 0` 或 `h_inv <= 0`）
fn infer_roi_pct(current_asset: f64, h_inv: f64, hist_months: usize) -> Option<f64> {
    if hist_months == 0 || h_inv <= 0.0 {
        return None;
    }
    let mut lo = -99.0_f64;
    let mut hi = 50.0_f64;
    for _ in 0..100 {
        let mid = (lo + hi) / 2.0;
        let r = (1.0 + mid / 100.0).powf(1.0 / 12.0) - 1.0;
        let mut bal = 0.0;
        for _ in 0..hist_months {
            bal = (bal + h_inv) * (1.0 + r);
        }
        if bal < current_asset {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    let final_roi = (lo + hi) / 2.0;
    // 消除 IEEE 754 殘留的 -0.00% 符號
    if final_roi.abs() < 0.005 {
        Some(0.0)
    } else {
        Some(final_roi)
    }
}

/// 將 ROI 數值格式化為固定欄寬的對齊標籤字串。
///
/// - 整數 ROI（例如 5、10、20）：`"ROI  5%"` / `"ROI 20%"`（右對齊到 4 位）
/// - 主線 ROI（例如 8.5）：`"ROI 8.50%"`（固定顯示 4 個字 但進位 2 位小數）
fn fmt_roi_label(roi_pct: f64, is_major: bool) -> String {
    if is_major {
        format!("ROI {:.4}%", format!("{:.2}", roi_pct))
    } else {
        format!("ROI {:4}%", roi_pct as usize)
    }
}

// =====================================================================
// # 3. 智慧型金融格式化與等寬對齊模組
// =====================================================================

fn format_with_commas(val: f64, precision: usize) -> String {
    let factor = 10.0_f64.powi(precision as i32);
    let rounded = (val.abs() * factor).round() / factor;

    let s = format!("{:.1$}", rounded, precision);
    let parts: Vec<&str> = s.split('.').collect();
    let num_part = parts[0];

    let mut result = String::new();
    for (count, c) in num_part.chars().rev().enumerate() {
        if count > 0 && count % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    let mut formatted = result.chars().rev().collect::<String>();
    if val < 0.0 {
        formatted.insert(0, '-');
    }
    if parts.len() > 1 {
        formatted.push('.');
        formatted.push_str(parts[1]);
    }
    formatted
}

fn format_twd_financial(val: f64) -> String {
    let abs_val = val.abs();

    // 🎯 只有當物理數值「真正突破或等於一億（100,000,000）」時，才放行至億的分支
    if abs_val >= 100_000_000.0 {
        format!("{}億", format_with_commas(val / 100_000_000.0, 1))
    }
    // 🎯 只要不到一億，哪怕是 99,999,999 元，也老老實實用「萬」呈現，保留完整細節
    else if abs_val >= 10_000.0 {
        let val_in_wan = val / 10_000.0;
        let abs_wan = val_in_wan.abs();

        // 檢查是不是刚好整除（例如 500.0 萬則顯示 500 萬，500.5 萬則顯示 500.5 萬）
        let is_round = (abs_wan - abs_wan.round()).abs() < 0.01;
        if is_round {
            format!("{}萬", format_with_commas(val_in_wan, 0))
        } else {
            format!("{}萬", format_with_commas(val_in_wan, 1))
        }
    }
    // 🎯 低於一萬，直接顯示千分位整數
    else {
        format!("{}元", format_with_commas(val, 0))
    }
}

fn make_clean_text_row(
    name_str: &str,
    val_str: &str,
    real_val_str: &str,
    highlight: bool,
    is_inflation: bool,
    is_narrow: bool,
) -> String {
    let style = if highlight {
        "style='font-family:Consolas,monospace; color:#F43F5E; font-weight:bold;'"
    } else {
        "style='font-family:Consolas,monospace;'"
    };

    let pad_w = 10;
    if is_narrow {
        format!("<span {style}>{name_str:<pad_w$} {real_val_str:>pad_w$}(折現)</span>")
    } else {
        if is_inflation {
            format!(
                "<span {style}>{name_str:<pad_w$} │ {val_str:>pad_w$} │ 折現：{real_val_str:>pad_w$}</span>"
            )
        } else {
            format!("<span {style}>{name_str:<pad_w$} │ {val_str:>pad_w$}</span>")
        }
    }
}

// =====================================================================
// # 4. 真·雙階段獨立複利演算法（時序連續防禦版）
// =====================================================================
/// 代表單一條資產成長軌道
#[derive(Debug, Clone, PartialEq)]
struct TrendRoute {
    /// 該條軌道所使用的精確年化報酬率 (例如 0.0, 5.4, 20.0)
    roi_pct: f64,
    /// 是否為使用者指定的主線錨定點
    is_anchor: bool,
    /// 軌道上每個月的名目與實質資產價值: Vec<(名目資產, 實質資產)>
    data: Vec<(f64, f64)>,
}

fn calculate_true_pivot_trends(
    h_inv: f64,            // 歷史每月投入（元）
    f_inv: f64,            // 未來每月投入/提領（元）
    anchor_roi_pct: f64,   // 外部傳入的精確浮點數年化 ROI
    inflation_rate: usize, // 未來通膨率
    hist_years: usize,     // 歷史年期
    total_years: usize,    // 總模擬年期
    lump_sum: f64,         // 現有資產結算點（元）
) -> Vec<TrendRoute> {
    let hist_months = hist_years * 12;
    let total_months = total_years * 12;
    let future_months = total_months.saturating_sub(hist_months);

    // 換算月化複合利率
    let anchor_monthly_rate = (1.0 + (anchor_roi_pct / 100.0)).powf(1.0 / 12.0) - 1.0;
    let inflation_monthly_rate = (1.0 + (inflation_rate as f64) / 100.0).powf(1.0 / 12.0) - 1.0;

    // ─── 1. 真實歷史區間動態模擬 ───
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

        // 🎯 物理鎖定：歷史最後一格強制等於現有資產
        if let Some(last_node) = hist_route.last_mut() {
            *last_node = (lump_sum, lump_sum);
        }
    }

    let initial_asset = hist_route.last().map(|&(_, real)| real).unwrap_or(0.0);

    // ─── 2. 收集所有需要計算的年化報酬率 ───
    // 使用 HashMap 除去重複值，避免當 anchor_roi_pct 剛好是整數時重複計算
    let mut target_rois: HashMap<i32, f64> = (0..=20)
        .map(|r| (r * 1000, r as f64)) // 放大 1000 倍作為 Key 規避浮點數 Hash 問題
        .collect();

    // 插入精確的主線 ROI (放大萬倍取整數作為唯一識別 Key，防止微幅抖動)
    let anchor_key = (anchor_roi_pct * 1000.0).round() as i32;
    target_rois.insert(anchor_key, anchor_roi_pct);

    let mut routes = Vec::with_capacity(target_rois.len());

    // ─── 3. 軌道發散點火模擬 ───
    for (&_key, &roi) in target_rois.iter() {
        let is_anchor = (roi - anchor_roi_pct).abs() < f64::EPSILON;
        let monthly_rate = (1.0 + (roi / 100.0)).powf(1.0 / 12.0) - 1.0;

        // 預先配置完整空間，避免 cloned 之後的 push 再次引發 Reallocation
        let mut full_route = Vec::with_capacity(total_months + 1);
        full_route.extend_from_slice(&hist_route);

        let mut curr_nominal = initial_asset;
        let mut future_inflation_factor = 1.0;

        for _ in 1..=future_months {
            future_inflation_factor *= 1.0 + inflation_monthly_rate;

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

    // ─── 4. 嚴謹排序 ───
    // 依據年化報酬率由小到大 (0% -> ... -> 20%) 排好，供後端繪圖直接按順序疊加
    routes.sort_by(|a, b| {
        a.roi_pct
            .partial_cmp(&b.roi_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    routes
}

// =====================================================================
// # 5. 動態標籤產生模組（優化：X軸定位改用真實歲數坐標）
// =====================================================================
fn get_annotations(
    trends: &[TrendRoute],
    start_age: usize,
    hist_years: usize,
    anchor_roi_pct: Option<f64>,
    lump_sum: f64,
) -> Vec<Annotation> {
    let mut ann_list = Vec::new();
    let hist_idx = hist_years * 12;

    let amt_now = if let Some(anchor_route) = trends.iter().find(|r| r.is_anchor) {
        // 安全防護：確保索引不越界
        if hist_idx < anchor_route.data.len() {
            anchor_route.data[hist_idx].0
        } else {
            lump_sum
        }
    } else {
        lump_sum
    };

    // 🎯 核心防禦：將 X 軸標籤定位點從「相對年期」平移為「真實年齡」
    // Y 軸因為是對數軸 (Log Scale)，其定位數值必須做安全防護，避免負數或零引發 log10() 出錯
    let (x_pos, y_val_log, text_str, show_arrow, ax, ay) = if hist_years > 0 {
        let label_text = if let Some(actual_roi) = anchor_roi_pct {
            format!(
                "📍 現況錨定 ({:.2}%): {}",
                actual_roi,
                format_twd_financial(amt_now)
            )
        } else {
            // 🎯 若為 None，直接拔除百分比括號，老老實實回歸資產數字，視覺上極度嚴謹
            format!("📍 現況結算: {}", format_twd_financial(amt_now))
        };

        (
            (start_age + hist_years) as f64,
            amt_now.max(10000.0).log10(), // 強制限低防禦對數軸爆炸
            label_text,
            true,
            0.0,
            60.0,
        )
    } else if lump_sum.abs() > 0.0 {
        (
            start_age as f64,
            lump_sum.abs().max(10000.0).log10(),
            if lump_sum >= 0.0 {
                format!("💰 起始資金: {}", format_twd_financial(lump_sum))
            } else {
                format!("⚠️ 起始負債: {}", format_twd_financial(lump_sum.abs()))
            },
            true,
            0.0,
            60.0,
        )
    } else {
        return ann_list;
    };

    ann_list.push(
        Annotation::new()
            .x(x_pos)
            .y(y_val_log)
            .text(&text_str)
            .show_arrow(show_arrow)
            .arrow_head(2)
            .arrow_color("#F43F5E")
            .arrow_size(1.0)
            .arrow_width(2.0)
            .ax(ax)
            .ay(ay)
            .font(
                Font::new()
                    .size(11)
                    .color("#F43F5E")
                    .family("Consolas, monospace"),
            )
            .background_color("rgba(15, 23, 42, 0.95)")
            .border_color("#F43F5E")
            .border_width(1.5)
            .border_pad(5.0),
    );

    ann_list
}

// =====================================================================
// # 6. 全自動響應式圖表引擎（前半段：數據流與線條配置）
// =====================================================================
fn generate_plot(ci: ChartInput, sorted_trends: Vec<TrendRoute>) -> Plot {
    let is_narrow = ci.window_width < 640;
    let is_inflation = ci.inflation_rate > 0;
    let total_months = ci.total_years * 12;
    let hist_months = ci.hist_years * 12;

    // 建立一組共用的 X 軸數值時間軸（真實歲數）
    let x_numeric_timeline: Vec<f64> = (0..=total_months)
        .map(|m| ci.start_age as f64 + (m as f64 / 12.0))
        .collect();

    let shared_x = Rc::new(x_numeric_timeline);

    // 預先動態生成 0% 到 20% 基礎色階
    let mut colors = Vec::with_capacity(21);
    for i in 0..=20 {
        colors.push(format!("rgba({}, {}, 255, 0.8)", 50 + i * 8, 80 + i * 5));
    }

    // 1. 組裝唯一的主線 Hover 提示文字
    let mut hover_labels_text = Vec::with_capacity(shared_x.len());
    for m in 0..=total_months {
        let elapsed_years = m / 12;
        let mo = m % 12;
        let current_calc_age = ci.start_age + elapsed_years;

        let time_header = if mo > 0 {
            format!(
                "<b>🎯 實際年齡：{} 歲 {} 個月</b> (第 {} 年)",
                current_calc_age, mo, elapsed_years
            )
        } else {
            format!(
                "<b>🎯 實際年齡：{} 歲整</b> (第 {} 年)",
                current_calc_age, elapsed_years
            )
        };

        let mut lines = vec![time_header, "────────────────────────".to_string()];

        if m <= hist_months {
            if let Some(anchor_route) = sorted_trends.iter().find(|r| r.is_anchor) {
                let amt = anchor_route.data[m];
                let label = if let Some(roi_val) = ci.anchor_roi_pct {
                    fmt_roi_label(roi_val, true)
                } else {
                    "ROI ----%".to_string()
                };
                lines.push(make_clean_text_row(
                    &label,
                    &format_twd_financial(amt.0),
                    &format_twd_financial(amt.1),
                    false,
                    is_inflation,
                    is_narrow,
                ));
            }
        } else {
            for route in sorted_trends.iter().rev() {
                let amt = route.data[m];
                let is_integer = (route.roi_pct - route.roi_pct.round()).abs() < f64::EPSILON;

                if route.is_anchor || is_integer {
                    let label = if route.is_anchor {
                        fmt_roi_label(ci.anchor_roi_pct(), true)
                    } else {
                        fmt_roi_label(route.roi_pct, false)
                    };
                    lines.push(make_clean_text_row(
                        &label,
                        &format_twd_financial(amt.0),
                        &format_twd_financial(amt.1),
                        route.is_anchor,
                        is_inflation,
                        is_narrow,
                    ));
                }
            }
        }
        hover_labels_text.push(lines.join("<br>"));
    }

    let mut hover_labels_opt = Some(hover_labels_text);
    let mut plot = Plot::new();

    // 2. 依序繪製所有跡線 (Traces)
    for route in sorted_trends.iter().rev() {
        let roi_floor = route.roi_pct.floor() as usize;
        let is_integer = (route.roi_pct - route.roi_pct.round()).abs() < f64::EPSILON;
        let is_p =
            (is_integer && [5, 10, 15, 20].contains(&(route.roi_pct as usize))) || route.is_anchor;
        let amt_future = route.data[total_months];

        let label = if is_narrow {
            if route.is_anchor {
                fmt_roi_label(ci.anchor_roi_pct(), true)
            } else {
                fmt_roi_label(route.roi_pct, false)
            }
        } else if route.is_anchor {
            format!("{} 主線", fmt_roi_label(ci.anchor_roi_pct(), true))
        } else {
            format!("{} 未來", fmt_roi_label(route.roi_pct, false))
        };

        let legend_name = make_clean_text_row(
            &label,
            &format_twd_financial(amt_future.0),
            &format_twd_financial(amt_future.1),
            route.is_anchor,
            is_inflation,
            is_narrow,
        );

        let y_data: Vec<f64> = route.data.iter().map(|x| x.0).collect();
        let mut trace = Scatter::new((*shared_x).clone(), y_data).name(legend_name);

        let color = if route.is_anchor {
            "#F43F5E".to_string()
        } else {
            colors
                .get(roi_floor)
                .cloned()
                .unwrap_or_else(|| "rgba(100,100,255,0.5)".to_string())
        };

        let width = if route.is_anchor {
            3.5 // 主線加粗，成為焦點
        } else if is_p {
            2.0 // 重點提示線
        } else {
            0.8 // 背景細線
        };

        trace = trace
            .line(Line::new().color(color).width(width))
            .show_legend(is_p);

        if route.is_anchor {
            if let Some(labels) = hover_labels_opt.take() {
                trace = trace
                    .text_array(labels)
                    .hover_template("%{text}<extra></extra>");
            }
        } else {
            trace = trace.hover_info(HoverInfo::Skip);
        }
        plot.add_trace(trace);
    }

    // 3. 繪製 0% 本金對照虛線
    if let Some(base_route) = sorted_trends
        .iter()
        .find(|r| !r.is_anchor && (r.roi_pct).abs() < f64::EPSILON)
    {
        let amt_principal_future = base_route.data[total_months];
        let principal_name = make_clean_text_row(
            if is_narrow {
                "ROI    0%"
            } else {
                "ROI    0% 本金"
            },
            &format_twd_financial(amt_principal_future.0),
            &format_twd_financial(amt_principal_future.1),
            false,
            is_inflation,
            is_narrow,
        );

        let y_base: Vec<f64> = base_route.data.iter().map(|x| x.0).collect();
        let principal_trace = Scatter::new((*shared_x).clone(), y_base)
            .name(principal_name)
            .line(Line::new().color("#A0AEC0").width(2.5).dash(DashType::Dash))
            .show_legend(true)
            .hover_info(HoverInfo::Skip);

        plot.add_trace(principal_trace);
    }

    let future_plan_text = if ci.f_inv > 0.0 {
        format!("每月改投名目 {}", format_twd_financial(ci.f_inv))
    } else if ci.f_inv < 0.0 {
        format!("每月提領實質 {}", format_twd_financial(ci.f_inv.abs()))
    } else {
        "不再投入(利滾利)".to_string()
    };

    let strategy_subtitle = if is_narrow {
        format!(
            "<br><span style='font-size: 11px; color: #2DD4BF;'>起始 {}歲 | 現況 {}歲 | {}</span>",
            ci.start_age,
            ci.start_age + ci.hist_years,
            future_plan_text
        )
    } else {
        let history_investment_text = if ci.hist_years > 0 {
            format!(" 已投入 {}", format_twd_financial(ci.h_inv_sum()))
        } else {
            String::new()
        };

        format!(
            "<br><span style='font-size: 13px; color: #2DD4BF; letter-spacing: 0.5px;'>📊 戰略配置 ── 起始 {}歲 ({}/月{}) | 現況 {}歲 | 目標 {}歲 [{}] | 折現通膨 {}%/年</span>",
            ci.start_age,
            format_twd_financial(ci.h_inv),
            history_investment_text,
            ci.start_age + ci.hist_years,
            ci.start_age + ci.total_years,
            future_plan_text,
            ci.inflation_rate
        )
    };

    let anns = get_annotations(
        &sorted_trends,
        ci.start_age,
        ci.hist_years,
        ci.anchor_roi_pct,
        ci.lump_sum,
    );

    // 4. 座標防禦刻度
    let f_years = ci.total_years.saturating_sub(ci.hist_years);
    let (x_ticks, x_tick_text) = if ci.hist_years > 0 && ci.total_years >= ci.hist_years {
        (
            vec![
                ci.start_age as f64,
                (ci.start_age + ci.hist_years) as f64,
                (ci.start_age + ci.total_years) as f64,
            ],
            vec![
                format!("🎬 {} 歲", ci.start_age),
                format!("📍 {} 歲 (結算)", ci.start_age + ci.hist_years),
                format!("🏁 {} 歲 (終點)", ci.start_age + ci.total_years),
            ],
        )
    } else {
        (
            vec![ci.start_age as f64, (ci.start_age + ci.total_years) as f64],
            vec![
                format!("🎯 {} 歲", ci.start_age),
                format!("🏁 {} 歲 (終點)", ci.start_age + ci.total_years),
            ],
        )
    };

    // 5. 縱向分水嶺定位線
    let mut shapes = Vec::new();
    let x_positions = if ci.hist_years > 0 && ci.total_years >= ci.hist_years {
        vec![
            ci.start_age as f64 + (ci.hist_years as f64 / 2.0),
            (ci.start_age + ci.hist_years) as f64,
            (ci.start_age + ci.hist_years) as f64 + (f_years as f64 / 2.0),
            (ci.start_age + ci.total_years) as f64,
        ]
    } else {
        vec![
            ci.start_age as f64,
            ci.start_age as f64 + (ci.total_years as f64 / 2.0),
            (ci.start_age + ci.total_years) as f64,
        ]
    };

    for x_pos in x_positions {
        let is_now = ci.hist_years > 0
            && (x_pos - (ci.start_age + ci.hist_years) as f64).abs() < f64::EPSILON;
        let line_color = if is_now {
            "rgba(244,63,94,0.8)".to_string()
        } else {
            "rgba(255,255,255,0.12)".to_string()
        };
        let line_width = if is_now { 2.5 } else { 1.5 };
        let dash_type = if is_now {
            DashType::Solid
        } else {
            DashType::Dash
        };

        shapes.push(
            Shape::new()
                .shape_type(ShapeType::Line)
                .x0(x_pos)
                .x1(x_pos)
                .y0(0.0)
                .y1(1.0)
                .y_ref("paper")
                .line(
                    ShapeLine::new()
                        .color(line_color)
                        .width(line_width)
                        .dash(dash_type),
                )
                .layer(ShapeLayer::Below),
        );
    }

    // 1. 物理視野下限死鎖在 1 萬元，徹底杜絕負數或趨零暴跌把對數軸向下無限扯塌！
    let visual_floor: f64 = 10_000.0;
    let y_min_log = visual_floor.log10(); // 順利通過編譯，精確得到 4.0

    // 2. 掃描所有軌道中的最高點資產，決定動態頂部視野
    let mut max_val = 100_000.0;
    for route in sorted_trends.iter() {
        for &(nominal, _) in route.data.iter() {
            if nominal > max_val {
                max_val = nominal;
            }
        }
    }

    // 3. 🎯 同步修正：對 1.5 倍的放大常數進行型別與 log10 安全處理
    let y_max_log = if max_val > visual_floor {
        (max_val * 1.5_f64).log10()
    } else {
        5.0 // 最大資產不到 10 萬時，預設給予 10 萬（5.0）的對數上限空間
    };

    // 4. 備妥所有可能橫跨的台灣標準純中文理財對數刻度關卡
    let all_potential_ticks: Vec<(f64, &str)> = vec![
        (10_000.0, "1萬"),
        (100_000.0, "10萬"),
        (1_000_000.0, "100萬"),
        (10_000_000.0, "1,000萬"),
        (100_000_000.0, "1億"),
        (1_000_000_000.0, "10億"),
        (10_000_000_000.0, "100億"),
        (100_000_000_000.0, "1000億"),
    ];

    // 5. 過濾出「真正落在我們設定的動態物理視野範圍內」的中文刻度
    let mut dynamic_y_vals = Vec::new();
    let mut dynamic_y_text = Vec::new();

    for (val, text) in all_potential_ticks {
        let val_log = val.log10();
        // 刻度打點起點嚴格卡在 1 萬（4.0），終點則不能超過當前的最大视野上限
        if val_log >= y_min_log && val_log <= y_max_log + 0.3 {
            dynamic_y_vals.push(val);
            dynamic_y_text.push(text.to_string());
        }
    }

    // ─── 6. 座標軸與圖例配置 ───
    let x_axis = Axis::new()
        .type_(AxisType::Linear)
        .tick_mode(TickMode::Array)
        .tick_values(x_ticks)
        .tick_text(x_tick_text)
        .grid_color("#1E293B")
        .zero_line_color("#334155")
        .tick_font(Font::new().color("#F1F5F9").size(11))
        .tick_length(15)
        .tick_color("rgba(0,0,0,0)");

    let y_axis = Axis::new()
        .title(
            Title::new()
                .text("資產總額價值 ( TWD )<br>&nbsp;")
                .font(Font::new().color("#F1F5F9").size(13)),
        )
        .type_(AxisType::Log)
        .auto_range(false)
        .range(vec![y_min_log, y_max_log])
        .tick_mode(TickMode::Array)
        .tick_values(dynamic_y_vals)
        .tick_text(dynamic_y_text)
        .grid_color("#1E293B")
        .zero_line_color("#334155")
        .tick_font(Font::new().color("#CBD5E1").size(11))
        .domain(&[0.05, 1.0]);

    let legend = if is_narrow {
        Legend::new()
            .y_anchor(Anchor::Top)
            .y(0.99)
            .x_anchor(Anchor::Left)
            .x(0.01)
            .background_color("rgba(30, 41, 59, 0.88)")
            .border_color("#475569")
            .border_width(1)
            .font(Font::new().color("#F1F5F9").size(8))
            .item_click(ItemClick::False)
            .item_double_click(ItemClick::False)
    } else {
        Legend::new()
            .y_anchor(Anchor::Bottom)
            .y(0.1)
            .x_anchor(Anchor::Right)
            .x(0.99)
            .background_color("rgba(30, 41, 59, 0.8)")
            .border_color("#475569")
            .border_width(1)
            .font(Font::new().color("#F1F5F9"))
            .item_click(ItemClick::False)
            .item_double_click(ItemClick::False)
    };

    let hover_label = Label::new()
        .background_color("rgba(15, 23, 42, 0.96)")
        .border_color("#475569")
        .font(
            Font::new()
                .size(12)
                .color("white")
                .family("Consolas, monospace"),
        );

    let title = Title::new()
        .text(format!("<b>人生財務戰航模擬器</b>{}", strategy_subtitle))
        .x(0.5)
        .y(0.96)
        .font(
            Font::new()
                .size(15)
                .color("#F8FAFC")
                .family("Microsoft JhengHei"),
        );

    let layout = Layout::new()
        .drag_mode(DragMode::False)
        .title(title)
        .paper_background_color("#0F172A")
        .plot_background_color("#0F172A")
        .margin(Margin::new().left(65).right(110).top(60).bottom(100))
        .hover_mode(HoverMode::X)
        .hover_label(hover_label)
        .legend(legend)
        .x_axis(x_axis)
        .y_axis(y_axis)
        .shapes(shapes)
        .annotations(anns);

    plot.set_layout(layout);
    plot.set_configuration(
        Configuration::new()
            .responsive(true)
            .display_mode_bar(DisplayModeBar::False),
    );

    plot
}

// ─── 輔助函數：計算終值並生成狀態描述 ──────────────────────────────────
fn derive_future_summary(
    ci: &ChartInput,
    sorted_trends: &[TrendRoute],
    asset_wan_val: f64,
    future_mode_val: FutureMode,
) -> (String, String, String) {
    // 精確定位主線軌道
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
                    "🚨 警告：依此提領速度與模擬回報，資產預計將在 <strong style='color:#F43F5E;'>{} 歲 {} 個月</strong> 時提早耗盡歸零！",
                    exact_age, b_months
                )
            } else {
                format!(
                    "🚨 警告：依此提領速度與模擬回報，資產預計將在 <strong style='color:#F43F5E;'>{} 歲整</strong> 時提早耗盡歸零！",
                    exact_age
                )
            };
        }

        bankruptcy_text
    } else {
        let real_str = if ci.inflation_rate > 0 {
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

    let av = asset_wan_val * 10_000.0;
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
// # 7. Leptos 網頁 UI 組件與主入口
// =====================================================================
#[component]
fn App() -> impl IntoView {
    // ─── 從 localStorage 讀取上次設定（擴大年齡與數值防禦） ───────────────────
    let init_start_age = ls_usize("fs_start_age", 25).clamp(0, 150);
    let init_current_age = ls_usize("fs_current_age", 38).clamp(init_start_age, 150);
    let init_target_age = ls_usize("fs_target_age", 65).clamp(init_current_age + 1, 150);
    let init_h_inv_k = ls_f64("fs_h_inv_k", 30.0).max(0.0);
    let init_asset_wan = ls_f64("fs_asset_wan", 0.0).max(0.0);
    let init_f_inv_k = ls_f64("fs_f_inv_k", 0.0).max(0.0);
    let init_inflation = ls_usize("fs_inflation", 2).min(6);
    let init_future_mode = match ls_str("fs_future_mode", "stop").as_str() {
        "invest" => FutureMode::Invest,
        "withdraw" => FutureMode::Withdraw,
        _ => FutureMode::Stop,
    };

    // ─── 核心計算型 Signals ────────────────────────────────────────────
    let (start_age, set_start_age) = signal(init_start_age);
    let (current_age, set_current_age) = signal(init_current_age);
    let (target_age, set_target_age) = signal(init_target_age);
    let (h_inv_k, set_h_inv_k) = signal(init_h_inv_k);
    let (asset_wan, set_asset_wan) = signal(init_asset_wan);
    let (future_mode, set_future_mode) = signal(init_future_mode);
    let (f_inv_k, set_f_inv_k) = signal(init_f_inv_k);
    let (inflation_rate, set_inflation_rate) = signal(init_inflation);
    let (panel_open, set_panel_open) = signal(true);

    // 🎯 核心 UX 優化：使用非卡死字串型暫存 Signals，讓使用者能流暢倒退修改
    let (start_age_raw, set_start_age_raw) = signal(init_start_age.to_string());
    let (current_age_raw, set_current_age_raw) = signal(init_current_age.to_string());
    let (target_age_raw, set_target_age_raw) = signal(init_target_age.to_string());
    let (h_inv_k_raw, set_h_inv_k_raw) = signal(init_h_inv_k.to_string());
    let (asset_wan_raw, set_asset_wan_raw) = signal(init_asset_wan.to_string());
    let (f_inv_k_raw, set_f_inv_k_raw) = signal(init_f_inv_k.to_string());

    // ─── 🎯 衍生計算值（升級為最高級別的防禦性時序約束） ─────────────────────

    // 使用 saturating_sub 確保無論如何相減，最低就是 0，絕不溢出
    let hist_years = move || current_age.get().saturating_sub(start_age.get());
    let is_hist_years_active = move || hist_years() > 0 && current_age.get() >= start_age.get();

    // 確保總年期必然大於或等於歷史年期，且不論使用者怎麼填，最低安全值就是 1 年
    let total_years = move || {
        let raw_total = target_age.get().saturating_sub(start_age.get());
        let hy = hist_years();
        if current_age.get() < start_age.get() || target_age.get() <= current_age.get() {
            // 如果年齡結構發生嚴重的短暫打字矛盾，強迫總年期等於歷史年期（即未來期為 0）
            hy.max(1)
        } else {
            raw_total.max(hy).max(1)
        }
    };

    let h_inv = move || h_inv_k.get() * 1000.0;
    let h_inv_sum = move || h_inv() * (hist_years() * 12) as f64;
    let current_asset = move || asset_wan.get() * 10_000.0;

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
        FutureMode::Invest => f_inv_k.get() * 1000.0,
        FutureMode::Withdraw => -f_inv_k.get() * 1000.0,
    };
    // ─── 視窗寬度 signal（響應旋轉與縮放）──────────────────────────
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

    // ─── 1. 基礎參數聚合 Memo ───
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

    // ─── 2. Debounced 接收端信號（控制高頻輸入） ───
    let (debounced_chart_input, set_debounced_chart_input) =
        signal(active_chart_input_memo.get_untracked());

    // 精準的 300ms 節流防禦 Effect
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
                            300,
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

    // ─── 3. ✨ 核心性能亮點：全系統唯一的複利數據計算源 ✨ ───
    // 這個 Memo 只監聽 debounced_chart_input，不論下游重複讀取多少次，都只會運算一次
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

    // ─── 4. Plot 異步 Resource（修改為直接從共享的 trends_memo 提煉，不再重複計算） ───
    let plot_resource = LocalResource::new(move || async move {
        let ci = debounced_chart_input.get();
        let trends = trends_memo.get(); // 👈 直接從緩存獲取，速度極快

        // 呼叫更新後的 generate_plot (我們將在下一段重構 generate_plot)
        generate_plot(ci, trends)
    });

    // ─── Effects ────────────────────────────────────────────────────
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

    // ─── 儲存設定到 localStorage ────────────────────────────────────
    Effect::new(move |_| {
        ls_set("fs_start_age", &start_age.get().to_string());
        ls_set("fs_current_age", &current_age.get().to_string());
        ls_set("fs_target_age", &target_age.get().to_string());
        ls_set("fs_h_inv_k", &h_inv_k.get().to_string());
        ls_set("fs_asset_wan", &asset_wan.get().to_string());
        ls_set("fs_f_inv_k", &f_inv_k.get().to_string());
        ls_set("fs_inflation", &inflation_rate.get().to_string());
        ls_set(
            "fs_future_mode",
            match future_mode.get() {
                FutureMode::Stop => "stop",
                FutureMode::Invest => "invest",
                FutureMode::Withdraw => "withdraw",
            },
        );
    });

    view! {
        <div class="app-container">
            <h2 class="app-title">"人生財務戰略導航：現況資產錨定與未來變革推演模擬器"</h2>

            // ── 控制面板外框 ───────────────────────────────────────────
            <div class=move || if panel_open.get() { "controls-panel panel-open" } else { "controls-panel" }>
                <button class="controls-summary" on:click=move |_| set_panel_open.update(|v| *v = !*v)>
                    <span class="summary-title">"⚙️ 模擬參數設定"</span>
                    <span class=move || if panel_open.get() { "panel-status-badge badge-open" } else { "panel-status-badge" }>
                        <span class="badge-text">{move || if panel_open.get() { "收合設定" } else { "修改參數" }}</span>
                        <span class="badge-arrow">"▾"</span>
                    </span>
                </button>
                <Show when=move || panel_open.get()>
                    <div class="controls-body">

                        // ── Row 1：基本年齡資料資料與歷史投入 ──
                        <div class="controls-grid">

                            // 1. 開始投資年齡
                            <div class="control-group">
                                <label class="control-label">"🗓️ 一：開始投資年齡（歲）"</label>
                                <input type="number" class="number-input"
                                    min="0" max="150" step="1" inputmode="numeric"
                                    prop:value=move || start_age_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_start_age_raw.set(val.clone()); // 打字時只更新字串暫存，絕不卡手
                                        if let Ok(v) = val.parse::<usize>() {
                                            set_start_age.set(v); // 背景靜態更新數值
                                        }
                                    }
                                    on:blur=move |_| {
                                        // 焦點移開時才執行全套防禦性約束與強制洗回
                                        let v = start_age.get().clamp(0, 150);
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

                            // 2. 目前年齡
                            <div class="control-group">
                                <label class="control-label">"🎂 二：目前年齡（歲）"</label>
                                <input type="number" class="number-input"
                                    min="0" max="150" step="1" inputmode="numeric"
                                    prop:value=move || current_age_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_current_age_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<usize>() {
                                            set_current_age.set(v);
                                        }
                                    }
                                    on:blur=move |_| {
                                        let v = current_age.get().clamp(start_age.get(), 150);
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

                            // 3. 目標年齡
                            <div class="control-group">
                                <label class="control-label">"🏁 三：目標年齡（歲）"</label>
                                <input type="number" class="number-input"
                                    min="0" max="150" step="1" inputmode="numeric"
                                    prop:value=move || target_age_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_target_age_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<usize>() {
                                            set_target_age.set(v);
                                        }
                                    }
                                    on:blur=move |_| {
                                        let v = target_age.get().clamp(current_age.get() + 1, 150);
                                        set_target_age.set(v);
                                        set_target_age_raw.set(v.to_string());
                                    }
                                />
                                <div class="input-hint">{move || format!("共模擬 {} 年", total_years())}</div>
                            </div>

                            // 4. 歷史每月投入
                            <Show when=move || is_hist_years_active()>
                                <div class="control-group">
                                    <label class="control-label">"💰 四：歷史每月投入（千元）"</label>
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
                                    <div class="input-hint">{move || format!("= {} 共投入 {}", format_twd_financial(h_inv()), format_twd_financial(h_inv_sum()))}</div>
                                </div>
                            </Show>
                            // 5. 現有資產 / 起始資金（動態標籤，不允許負數）
                            <div class="control-group">
                                <label class="control-label">
                                    {move || if hist_years() > 0 {
                                        "🎯 五：現有資產（萬元）"
                                    } else {
                                        "💰 四：起始資金（萬元）"
                                    }}
                                </label>
                                <input type="number" class="number-input"
                                    min="0" // 透過瀏覽器原生屬性暗示不接受負數
                                    step="1" inputmode="decimal"
                                    prop:value=move || asset_wan_raw.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        set_asset_wan_raw.set(val.clone());
                                        if let Ok(v) = val.parse::<f64>() {
                                            // 打字時如果輸入負數，背景悄悄將計算核心截斷為 0.0，維持圖表不破版
                                            set_asset_wan.set(v.max(0.0));
                                        }
                                    }
                                    on:blur=move |_| {
                                        // 當使用者滑鼠移開欄位時，強制把小於 0 的不合法輸入文字清洗成 "0"
                                        let v = asset_wan.get().max(0.0);
                                        set_asset_wan.set(v);
                                        set_asset_wan_raw.set(v.to_string());
                                    }
                                />
                                {move || {
                                    let hy = hist_years();
                                    let av = asset_wan.get();
                                    let asset_twd = av * 10_000.0;

                                    if hy == 0 {
                                        let hint = if av == 0.0 {
                                            "0 = 從零開始（不帶資金）".to_string()
                                        } else if av > 0.0 {
                                            format!("= {}，做為複利起始本金", format_twd_financial(asset_twd))
                                        } else {
                                            format!("= {}，帶負債起步", format_twd_financial(asset_twd.abs()))
                                        };
                                        view! { <div class="input-hint">{hint}</div> }.into_any()
                                    } else if av == 0.0 {
                                        view! {
                                            <div class="input-hint">"輸入資產以自動推估歷史報酬率"</div>
                                        }.into_any()
                                    } else {
                                        match anchor_roi_pct() {
                                            None => view! { <div class="input-hint warning">"⚠️ 無法推估，請確認數字"</div> }.into_any(),
                                            Some(r) if !(-25.0..=25.0).contains(&r) => view! { <div class="input-hint warning">{format!("🚨 隱含 {:.2}%/年，請確認", r)}</div> }.into_any(),
                                            Some(r) if r < 0.0 => view! { <div class="input-hint warning">{format!("⚠️ 隱含 {:.2}%/年，虧損中", r)}</div> }.into_any(),
                                            Some(r) => view! { <div class="input-hint info">{format!("≈ 年化 {:.2}%", r)}</div> }.into_any(),
                                        }
                                    }
                                }}
                            </div>
                        </div>

                        // ── Row 2：未來計畫 + 通膨率 ──
                        <div class="controls-grid controls-grid-future">

                            // 6. 未來計畫（🎯 已精確修正：補足「千元」語意標籤與提示）
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
                                            let amt = format_twd_financial(f_inv_k.get() * 1000.0);
                                            match future_mode.get() {
                                                FutureMode::Invest   => format!("= 未來每月名目投入 {}", amt),
                                                FutureMode::Withdraw => format!("= 未來每月實質提領 {}", amt),
                                                FutureMode::Stop     => String::new(),
                                            }
                                        }}</div>
                                    </div>
                                </Show>
                            </div>

                            // 7. 通膨率
                            <div class="control-group">
                                <label class="control-label">
                                    {move || if hist_years() > 0 { "📉 七：未來通膨率" } else { "📉 六：未來通膨率" }}
                                </label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() { set_inflation_rate.set(val); }
                                    }>
                                        {move || (0..=10).map(|r| {
                                            let label = if r == 0 { "🚫 不考慮通膨 (0%)".to_string() } else { format!("📉 通膨率：{}%/年", r) };
                                            view! { <option value=r selected=move || inflation_rate.get() == r>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                        </div>
                    </div>
                </Show>
            </div>
            // ── 情境摘要卡片（重構後：邏輯清晰、極易讀） ───────────────────
            <div class="summary-card">
                {move || {
                    let ci = debounced_chart_input.get();
                    let trends = trends_memo.get();
                    let sage = start_age.get();
                    let tage = target_age.get();

                    let (future_desc, end_str, roi_info) = derive_future_summary(
                        &ci, &trends, asset_wan.get(), future_mode.get()
                    );

                    let roi = ci.anchor_roi_pct();

                    match (ci.hist_years, ci.lump_sum == 0.0) {
                        // 情境 A：白手起家（無歷史、無起始本金 ── 隱含報酬率必為 None）
                        (0, true) => {
                            let intro = if sage == 0 { "👶 幫新生兒從 0 歲白手起家 — " } else { "📊 規劃從 " };
                            view! { <span class="summary-text">
                                {intro} {if sage > 0 { format!("{} 歲出發 — ", sage) } else { "".to_string() }}
                                {future_desc} "，以系統基準預估年化回報 " <strong>{format!("{roi:.2}")} "%"</strong> " 在 "
                                <strong>{tage}</strong> " 歲時，" <span inner_html=end_str />
                            </span> }.into_any()
                        },

                        // 情境 B：單純單筆配置（無歷史、有起始本金 ── 隱含報酬率必為 None）
                        (0, false) => {
                            let intro = if sage == 0 { "👶 幫小孩從 0 歲配置 — " } else { "💰 規劃從 " };
                            let ls_desc = format!("起始本金 <strong>{}</strong>", format_twd_financial(ci.lump_sum));
                            view! { <span class="summary-text">
                                {intro} {if sage > 0 { format!("{} 歲配置 — ", sage) } else { "".to_string() }}
                                <span inner_html=ls_desc /> "，" {future_desc} "，並以基準預估年化回報 " <strong>{format!("{roi:.2}")} "%"</strong> " 在 "
                                <strong>{tage}</strong> " 歲時，" <span inner_html=end_str />
                            </span> }.into_any()
                        },

                        // 情境 C：已有過去投資歷史（區分真實算出與 None 的歷史情境）
                        (_hist, _) => {
                            let strategy_desc = if ci.anchor_roi_pct.is_some() {
                                format!("。延續此回報率並調整戰略為：{future_desc}，期望在 ", )
                            } else {
                                format!("。目前無隱含報酬，調整戰略為：{future_desc}，並以基準預估年化回報 {roi:.2}% 期望在 ")
                            };

                            view! { <span class="summary-text">
                                "自 " {sage} " 歲起每月投資 " {format_twd_financial(ci.h_inv)}
                                " 共投入 " <strong>{format_twd_financial(ci.h_inv_sum())}</strong>
                                "，至今已投 " {ci.hist_years} " 年" <span inner_html=roi_info />
                                {strategy_desc} <strong>{tage}</strong> " 歲時，" <span inner_html=end_str />
                            </span> }.into_any()
                        }
                    }
                }}
            </div>
            // ── 截圖按鈕 ─────────────────────────────────────────────
            <div class="chart-header">
                <button class="screenshot-btn" title="下載圖表 PNG（1920×1080）"
                    on:click=move |_| {
                        #[cfg(target_family = "wasm")]
                        { let _ = js_sys::eval("Plotly.downloadImage(document.getElementById('financial-graph'),{format:'png',width:1920,height:1080,filename:'financial_simulator'})"); }
                    }
                >"📷 截圖"</button>
            </div>

            // ── 圖表 ─────────────────────────────────────────────────
            <div id="financial-graph" class="graph-container"
                style=move || if panel_open.get() { "height: clamp(520px, 68vh, 720px);" } else { "height: clamp(520px, 82vh, 860px);" }
            ></div>

            // ── 底部說明 ──────────────────────────────────────────────
            <div class="chart-footer-notes">
                <p class="note-item">
                    "💡 " <b>"導航小提示："</b>
                    "名目金額代表未來實際看到的數字；折現（實質金額）則是扣除通膨率後，換算回「現在這一刻」的實質購買力。當您選擇提領時，系統會自動將提領金額隨通膨率調升，以保障您的實質生活水平。資產或終值若出現負數，表示該情境下資金已耗盡並進入負債。"
                </p>
            </div>
        </div>
    }
}

// =====================================================================
// # 8. Wasm 應用程式主進入點 (Client-Side Rendering)
// =====================================================================
fn main() {
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    leptos::prelude::mount_to_body(App);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_with_commas_basic() {
        // 測試純千分位逗號與精準度
        assert_eq!(format_with_commas(1234.56, 1), "1,234.6");
        assert_eq!(format_with_commas(1000000.0, 0), "1,000,000");
        assert_eq!(format_with_commas(-500.55, 1), "-500.6");
        assert_eq!(format_with_commas(0.0, 2), "0.00");
    }

    #[test]
    fn test_twd_financial_under_ten_thousand() {
        // 🎯 測試低於一萬的狀況：直接顯示千分位整數，不帶「萬」或「億」
        assert_eq!(format_twd_financial(0.0), "0元");
        assert_eq!(format_twd_financial(150.0), "150元");
        assert_eq!(format_twd_financial(9999.0), "9,999元");
        assert_eq!(format_twd_financial(-8500.0), "-8,500元");
    }

    #[test]
    fn test_twd_financial_wan_level() {
        // 🎯 測試萬級距 (1萬 ~ 9999萬) 且包含整除與不整除的細緻邏輯
        assert_eq!(format_twd_financial(10000.0), "1萬");
        assert_eq!(format_twd_financial(500000.0), "50萬");

        // 測試整除微調 (例如 500.0 萬顯示 500 萬)
        assert_eq!(format_twd_financial(5000000.0), "500萬");

        // 測試小數點過渡 (例如 500.5 萬顯示 500.5 萬)
        assert_eq!(format_twd_financial(5005000.0), "500.5萬");

        // 測試極限邊界：只要不到一億，哪怕 9999.9 萬也老實呈現萬
        assert_eq!(format_twd_financial(99999000.0), "9,999.9萬");
        assert_eq!(format_twd_financial(-250000.0), "-25萬");
    }

    #[test]
    fn test_twd_financial_yi_level() {
        // 🎯 測試億級距邊界條件 (≥ 100,000,000)
        assert_eq!(format_twd_financial(100000000.0), "1.0億");
        assert_eq!(format_twd_financial(150000000.0), "1.5億");
        assert_eq!(format_twd_financial(10005000000.0), "100.1億");
        assert_eq!(format_twd_financial(-1200000000.0), "-12.0億");
    }

    #[test]
    fn test_text_row_alignment() {
        // 🎯 測試等寬對齊與 HTML 標籤注入的字串長度與結構
        let normal_row = make_clean_text_row("ROI  5%", "500萬", "500萬", false, false, false);
        assert!(normal_row.contains("style='font-family:Consolas,monospace;'"));
        assert!(normal_row.contains("ROI  5%"));
        // 由於沒有折現落差，不應該出現「折現：」字樣
        assert!(!normal_row.contains("折現："));

        let discount_row = make_clean_text_row("ROI 10%", "1,000萬", "800萬", false, true, false);
        assert!(discount_row.contains("折現："));

        let highlight_row = make_clean_text_row("ROI 10%", "1億", "1億", true, true, false);
        assert!(highlight_row.contains("color:#F43F5E; font-weight:bold;"));

        let short_row = make_clean_text_row("ROI  5%", "350萬", "350萬", true, true, true);
        assert!(short_row.contains("color:#F43F5E; font-weight:bold;"));
        assert!(short_row.contains("350萬"));
    }

    // =====================================================================
    // # 8. 測試模組 - 第二部分：複利演算核心與時序連續性整合測試
    // =====================================================================

    #[test]
    fn test_calculate_true_pivot_trends_basic_structure() {
        let h_inv = 10000.0; // 歷史每月投入 1 萬
        let f_inv = 20000.0; // 未來每月改投 2 萬
        let anchor_roi_pct = 10.5; // 精確的 f64 歷史年化報酬率 (非整數)
        let inflation_rate = 2;
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
        let inflation_rate = 3;
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
        let inflation_rate = 2; // 通膨年化 2%
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
        let inflation_monthly_rate = (1.0 + (inflation_rate as f64) / 100.0).powf(1.0 / 12.0) - 1.0;

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
        let trends = calculate_true_pivot_trends(20000.0, 20000.0, 7.0, 2, 0, 10, lump_sum);

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
        let inflation_rate = 3;
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
        let inflation_rate = 0;
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
            inflation_rate: 2,
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

    // =====================================================================
    // # 10. 新增測試 — fmt_roi_label
    // =====================================================================

    #[test]
    fn test_fmt_roi_label_integer() {
        // 整數 ROI 格式
        assert_eq!(fmt_roi_label(5.0, false), "ROI    5%");
        assert_eq!(fmt_roi_label(10.0, false), "ROI   10%");
        assert_eq!(fmt_roi_label(20.0, false), "ROI   20%");
    }

    #[test]
    fn test_fmt_roi_label_float_consistency() {
        // 先鎖定兩位，再強迫格式化補滿固定寬度
        assert_eq!(fmt_roi_label(8.5, true), "ROI 8.50%");
        assert_eq!(fmt_roi_label(12.5, true), "ROI 12.5%"); // 關鍵：不應截斷
        assert_eq!(fmt_roi_label(0.0, true), "ROI 0.00%");
        assert_eq!(fmt_roi_label(-5.5, true), "ROI -5.5%");
    }

    // =====================================================================
    // # 11. 新增測試 — ChartInput 方法
    // =====================================================================

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
            inflation_rate: 0,
            window_width: 1920,
        };
        assert_eq!(ci_none.anchor_roi_pct(), 7.0);

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
            inflation_rate: 2,
            window_width: 1920,
        };
        let expected = 30_000.0 * 60.0;
        assert!(
            (ci.h_inv_sum() - expected).abs() < f64::EPSILON,
            "h_inv_sum 應等於 h_inv * hist_months"
        );
    }

    // =====================================================================
    // # 12. 新增測試 — TrendRoute 結構與排序
    // =====================================================================

    #[test]
    fn test_trend_route_exactly_one_anchor() {
        let trends = calculate_true_pivot_trends(20_000.0, 0.0, 7.0, 0, 5, 20, 1_000_000.0);
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
        let trends = calculate_true_pivot_trends(10_000.0, 0.0, 10.0, 2, 5, 15, 500_000.0);
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
        let trends = calculate_true_pivot_trends(30_000.0, -10_000.0, 8.37, 3, 10, 30, 3_000_000.0);
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
            calculate_true_pivot_trends(0.0, f_inv, 5.0, 0, hist_years, total_years, lump_sum);

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

        let trends = calculate_true_pivot_trends(0.0, f_inv, 12.0, 0, 0, 10, lump_sum);

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

    /// 恰好 10,000 元的邊界
    #[test]
    fn test_format_twd_exactly_ten_thousand() {
        assert_eq!(format_twd_financial(10_000.0), "1萬");
    }

    /// 恰好 100,000,000 元的億級邊界
    #[test]
    fn test_format_twd_exactly_one_hundred_million() {
        assert_eq!(format_twd_financial(100_000_000.0), "1.0億");
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
}
