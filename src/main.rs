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
    anchor_roi_pct: f64,
    lump_sum: f64,
    f_inv: f64,
    inflation_rate: usize,
    window_width: u32,
}

thread_local! {
    static DEBOUNCE_TIMER: Cell<i32> = const { Cell::new(-1) };
}

// =====================================================================
// # 1. localStorage 記憶持久化工具
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
    Some((lo + hi) / 2.0)
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
        format_with_commas(val, 0)
    }
}

fn make_clean_text_row(
    name_str: &str,
    val_str: &str,
    real_val_str: &str,
    highlight: bool,
) -> String {
    let style = if highlight {
        "style='font-family:Consolas,monospace; color:#F43F5E; font-weight:bold;'"
    } else {
        "style='font-family:Consolas,monospace;'"
    };

    if val_str == real_val_str {
        format!("<span {style}>{name_str:<9} │ {val_str:>9}</span>")
    } else {
        format!("<span {style}>{name_str:<9} │ {val_str:>9} │ 折現：{real_val_str:>9}</span>")
    }
}

fn make_short_clean_text_row(name_str: &str, val_str: &str, highlight: bool) -> String {
    let style = if highlight {
        "style='font-family:Consolas,monospace; color:#F43F5E; font-weight:bold;'"
    } else {
        "style='font-family:Consolas,monospace;'"
    };

    format!("<span {style}>{name_str:<9} {val_str:>9}(折現)</span>")
}

// =====================================================================
// # 4. 真·雙階段獨立複利演算法（時序連續防禦版）
// =====================================================================
fn calculate_true_pivot_trends(
    h_inv: f64,            // 歷史每月投入（元）
    f_inv: f64,            // 未來每月投入/提領（元）
    anchor_roi_pct: f64,   // 直接傳入外面算好的「精確浮點數年化ROI」
    inflation_rate: usize, // 未來通膨率
    hist_years: usize,     // 歷史年期
    total_years: usize,    // 總模擬年期
    lump_sum: f64,         // 現有資產（元）
) -> HashMap<usize, Vec<(f64, f64)>> {
    let hist_months = hist_years * 12;
    let total_months = total_years * 12;
    let future_months = total_months.saturating_sub(hist_months);

    // 使用外面傳入的精確歷史年化報酬率，換算為月化複合利率
    let anchor_monthly_rate = (1.0 + (anchor_roi_pct / 100.0)).powf(1.0 / 12.0) - 1.0;
    let inflation_monthly_rate = (1.0 + (inflation_rate as f64) / 100.0).powf(1.0 / 12.0) - 1.0;
    let mut trends = HashMap::new();

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

        // 🎯 物理鎖定：不論浮點數再怎麼微幅抖動，歷史最後一格（現在結算點）強制等於使用者輸入的 200 萬！
        if let Some(last_node) = hist_route.last_mut() {
            *last_node = (lump_sum, lump_sum);
        }
    }

    // ─── 2. 未來 21 條軌道發散點火 (0% ~ 20%) ───
    // 未來期的出發點，百分之百死死鎖定在歷史終點（也就是使用者填寫的 200 萬）上！
    let initial_asset = hist_route.last().map(|&(_, real)| real).unwrap_or(0.0);

    for r in 0..=20 {
        let monthly_rate = (1.0 + (r as f64) / 100.0).powf(1.0 / 12.0) - 1.0;
        let mut full_route = hist_route.clone();
        let mut curr_nominal = initial_asset;
        let mut future_inflation_factor = 1.0;

        for _ in 1..=future_months {
            future_inflation_factor *= 1.0 + inflation_monthly_rate;

            // 名目現金流動態判定：投入不變、提領隨通膨調整
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
        trends.insert(r, full_route);
    }

    trends
}

// =====================================================================
// # 5. 動態標籤產生模組（優化：X軸定位改用真實歲數坐標）
// =====================================================================
fn get_annotations(
    trends: &HashMap<usize, Vec<(f64, f64)>>,
    start_age: usize,
    hist_years: usize,
    anchor_roi: usize,
    anchor_roi_pct: f64,
    lump_sum: f64,
) -> Vec<Annotation> {
    let mut ann_list = Vec::new();
    let hist_idx = hist_years * 12;
    let amt_now = trends[&anchor_roi][hist_idx].0;

    // 🎯 核心修正：將 X 軸標籤定位點從「相對年期」平移為「真實年齡」
    let (x_pos, y_val_log, text_str, show_arrow, ax, ay) = if hist_years > 0 {
        (
            (start_age + hist_years) as f64,
            amt_now.abs().max(10000.0).log10(),
            format!(
                "📍 現況錨定 ({:.2}%): {}",
                anchor_roi_pct,
                format_twd_financial(amt_now)
            ),
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
fn generate_plot(ci: ChartInput) -> Plot {
    // 🎯 核心重構：直接解構 ChartInput 結構體，完美收合參數，消滅 Clippy 參數過多警告
    let ChartInput {
        start_age,
        total_years,
        hist_years,
        h_inv,
        anchor_roi_pct,
        lump_sum,
        f_inv,
        inflation_rate,
        window_width,
    } = ci;

    let is_narrow = window_width < 640;
    let anchor_roi = anchor_roi_pct.round().clamp(0.0, 20.0) as usize;

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
    let hist_months = hist_years * 12;

    // 將 X 軸數值由「相對年期數」直接換算成「真實歲數」
    let x_numeric_timeline: Vec<f64> = (0..=total_months)
        .map(|m| start_age as f64 + (m as f64 / 12.0))
        .collect();

    let mut colors = Vec::new();
    for i in 0..=20 {
        colors.push(format!("rgba({}, {}, 255, 0.8)", 50 + i * 8, 80 + i * 5));
    }

    let mut hover_labels_text = Vec::with_capacity(x_numeric_timeline.len());

    #[allow(clippy::needless_range_loop)]
    for m in 0..=total_months {
        let elapsed_years = m / 12;
        let mo = m % 12;
        let current_calc_age = start_age + elapsed_years;

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
            let amt = trends[&anchor_roi][m];
            lines.push(make_clean_text_row(
                &format!("ROI {:.4}%", anchor_roi_pct.to_string()),
                &format_twd_financial(amt.0),
                &format_twd_financial(amt.1),
                false,
            ));
        } else {
            for r in (0..=20).rev() {
                let is_anchor = r == anchor_roi;
                let amt = trends[&r][m];
                let label = if is_anchor {
                    format!("ROI {:.4}%", anchor_roi_pct.to_string())
                } else {
                    format!("ROI {:4}%", r)
                };
                lines.push(make_clean_text_row(
                    &label,
                    &format_twd_financial(amt.0),
                    &format_twd_financial(amt.1),
                    is_anchor,
                ));
            }
        }
        hover_labels_text.push(lines.join("<br>"));
    }

    let mut hover_labels_opt = Some(hover_labels_text);
    let mut plot = Plot::new();

    for r in (0..=20).rev() {
        let is_p = [5, 10, 15, 20].contains(&r) || r == anchor_roi;
        let total_idx = total_years * 12;
        let amt_future = trends[&r][total_idx];

        let legend_name = if is_narrow {
            if r == anchor_roi {
                make_short_clean_text_row(
                    &format!("ROI {:.4}%", anchor_roi_pct.to_string()),
                    &format_twd_financial(amt_future.1),
                    true,
                )
            } else {
                make_short_clean_text_row(
                    &format!("ROI {:4}%", r),
                    &format_twd_financial(amt_future.1),
                    false,
                )
            }
        } else if r == anchor_roi {
            make_clean_text_row(
                &format!("ROI {:.4}% 主線", format!("{anchor_roi_pct:.2}")),
                &format_twd_financial(amt_future.0),
                &format_twd_financial(amt_future.1),
                true,
            )
        } else {
            make_clean_text_row(
                &format!("ROI {:4}% 未來", r),
                &format_twd_financial(amt_future.0),
                &format_twd_financial(amt_future.1),
                false,
            )
        };

        let mut trace = Scatter::new(
            x_numeric_timeline.clone(),
            trends[&r].iter().map(|x| x.0).collect(),
        )
        .name(legend_name);

        let color = if r == anchor_roi {
            "#F43F5E".to_string()
        } else {
            colors[r].clone()
        };
        let width = if r == anchor_roi {
            3.5
        } else if is_p {
            2.0
        } else {
            0.8
        };

        trace = trace
            .line(Line::new().color(color).width(width))
            .show_legend(is_p);

        if r == anchor_roi {
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
    if anchor_roi != 0 {
        let amt_principal_future = trends[&0][total_years * 12];
        let principal_name = if is_narrow {
            make_short_clean_text_row(
                "ROI    0%",
                &format_twd_financial(amt_principal_future.1),
                false,
            )
        } else {
            make_clean_text_row(
                "ROI    0% 本金",
                &format_twd_financial(amt_principal_future.0),
                &format_twd_financial(amt_principal_future.1),
                false,
            )
        };

        let principal_trace = Scatter::new(
            x_numeric_timeline.clone(),
            trends[&0].iter().map(|x| x.0).collect(),
        )
        .name(principal_name)
        .line(Line::new().color("#A0AEC0").width(2.5).dash(DashType::Dash))
        .show_legend(true)
        .hover_info(HoverInfo::Skip);

        plot.add_trace(principal_trace);
    }

    let future_plan_text = if f_inv > 0.0 {
        format!("每月改投名目 {}", format_twd_financial(f_inv))
    } else if f_inv < 0.0 {
        format!("每月提領實質 {}", format_twd_financial(f_inv.abs()))
    } else {
        "不再投入(利滾利)".to_string()
    };

    let strategy_subtitle = if is_narrow {
        format!(
            "<br><span style='font-size: 11px; color: #2DD4BF; font-weight: normal;'>\
            起始 {}歲 | 現況 {}歲 | {}</span>",
            start_age,
            start_age + hist_years,
            future_plan_text,
        )
    } else {
        format!(
            "<br><span style='font-size: 13px; color: #2DD4BF; font-weight: normal; \
            letter-spacing: 0.5px; line-height: 1.6;'>\
            📊 戰略配置 ── 起始 {}歲 ({} /月) | 現況 {}歲 | 目標 {}歲 [{}] | 折現通膨 {}%/年</span>",
            start_age,
            format_twd_financial(h_inv),
            start_age + hist_years,
            start_age + total_years,
            future_plan_text,
            inflation_rate,
        )
    };

    let anns = get_annotations(
        &trends,
        start_age,
        hist_years,
        anchor_roi,
        anchor_roi_pct,
        lump_sum,
    );

    // 🎯 核心防禦：重新定義安全的刻度防禦機制
    let f_years = total_years.saturating_sub(hist_years);
    let (x_ticks, x_tick_text) = if hist_years > 0 && total_years >= hist_years {
        (
            vec![
                start_age as f64,
                (start_age + hist_years) as f64,
                (start_age + total_years) as f64,
            ],
            vec![
                format!("🎬 {} 歲 (起點)", start_age),
                format!("📍 {} 歲 (現在結算)", start_age + hist_years),
                format!("🏁 {} 歲 (未來終點)", start_age + total_years),
            ],
        )
    } else {
        (
            vec![start_age as f64, (start_age + total_years) as f64],
            vec![
                format!("🎯 {} 歲 (現在起點)", start_age),
                format!("🏁 {} 歲 (未來終點)", start_age + total_years),
            ],
        )
    };

    // 🎯 核心防禦：縱向分水嶺定位線也一併加上安全防護，避免除以零或時序顛倒
    let mut shapes = Vec::new();
    let x_positions = if hist_years > 0 && total_years >= hist_years {
        vec![
            start_age as f64 + (hist_years as f64 / 2.0),
            (start_age + hist_years) as f64,
            (start_age + hist_years) as f64 + (f_years as f64 / 2.0),
            (start_age + total_years) as f64,
        ]
    } else {
        vec![
            start_age as f64,
            start_age as f64 + (total_years as f64 / 2.0),
            (start_age + total_years) as f64,
        ]
    };

    for x_pos in x_positions {
        let is_now = hist_years > 0 && x_pos == (start_age + hist_years) as f64;
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

        let shape = Shape::new()
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
            .layer(ShapeLayer::Below);
        shapes.push(shape);
    }

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
        .grid_color("#1E293B")
        .zero_line_color("#334155")
        .tick_font(Font::new().color("#CBD5E1").size(11))
        .tick_values(vec![
            10000.0,
            100000.0,
            1000000.0,
            10000000.0,
            100000000.0,
            1000000000.0,
            10000000000.0,
        ])
        .tick_text(vec![
            "1萬", "10萬", "100萬", "1,000萬", "1億", "10億", "100億",
        ])
        .domain(&[0.05, 1.0])
        .auto_range(true);

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
        .text(format!(
            "<b>人生財務戰略導航模擬器</b>{}",
            strategy_subtitle
        ))
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
    asset_wan_val: f64,
    future_mode_val: FutureMode,
) -> (String, String, String) {
    // 1. 呼叫與圖表完全同源的複利演算法
    let anchor_roi_idx = ci.anchor_roi_pct.round().clamp(0.0, 20.0) as usize;
    let trends = calculate_true_pivot_trends(
        ci.h_inv,
        ci.f_inv,
        ci.anchor_roi_pct,
        ci.inflation_rate,
        ci.hist_years,
        ci.total_years,
        ci.lump_sum,
    );

    // 2. 取得最後一個月的（名目終值, 實質終值）
    let total_months = ci.total_years * 12;
    let (nom, real) = trends
        .get(&anchor_roi_idx)
        .and_then(|route| route.get(total_months))
        .copied()
        .unwrap_or((0.0, 0.0));

    // 3. 完美利用：`future_mode_val` 生成未來投入模式描述
    let future_desc = match future_mode_val {
        FutureMode::Stop => "未來不再投入".to_string(),
        FutureMode::Invest => format!("未來每月投入 {}", format_twd_financial(ci.f_inv.abs())),
        FutureMode::Withdraw => format!("未來每月提領 {}", format_twd_financial(ci.f_inv.abs())),
    };

    // 4. 生成結尾資產累積描述
    let end_str = if nom <= 0.0 {
        format!("⚠️ 資產恐將耗盡（名目終值 {}）", format_twd_financial(nom))
    } else {
        let real_str = if ci.inflation_rate > 0 {
            format!("，實質購買力約 {}", format_twd_financial(real))
        } else {
            String::new()
        };
        format!("資產累積名目約 {}{}", format_twd_financial(nom), real_str)
    };

    // 5. 完美利用：`asset_wan_val` 與 `ci.anchor_roi_pct` 計算隱含年化資訊
    let av = asset_wan_val * 10_000.0;
    let roi_info = if av != 0.0 {
        format!(
            "，現有資產 {} (≈隱含年化 {:.2}%)",
            format_twd_financial(av),
            ci.anchor_roi_pct
        )
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
    let current_asset = move || asset_wan.get() * 10_000.0;

    let lump_sum = move || current_asset();

    let roi_pct = move || {
        let hm = hist_years() * 12;
        if hm == 0 || asset_wan.get() == 0.0 || current_age.get() < start_age.get() {
            None
        } else {
            infer_roi_pct(current_asset(), h_inv(), hm)
        }
    };
    let anchor_roi_pct = move || roi_pct().unwrap_or(7.0);
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

    // ─── 修正：將單純的閉包改為強大的 Memo，完美解決非追蹤上下文存取警告 ───
    // Memo 會自動在 Leptos 內部的追蹤上下文執行第一次初始化，不會引發任何警告
    let active_chart_input_memo = Memo::new(move |_| ChartInput {
        start_age: start_age.get(),
        total_years: total_years(),
        hist_years: hist_years(),
        h_inv: h_inv(),
        anchor_roi_pct: anchor_roi_pct().clamp(-99.0, 50.0),
        lump_sum: lump_sum(),
        f_inv: f_inv(),
        inflation_rate: inflation_rate.get(),
        window_width: window_width.get(),
    });

    // ─── Debounced ChartInput 接收端信號 ──────────────────────────────
    // 初始值採用 untracked（或直接複製 Memo 的當前數值）來防禦警告
    let (chart_input, set_chart_input) = signal(active_chart_input_memo.get_untracked());

    // ─── 精準的 300ms 節流防禦 Effect ─────────────────────────────────
    // 當 Memo 的上游參數改變時，這裡會被觸發，並透過計時器延遲更新 chart_input
    Effect::new(move |_| {
        let new_ci = active_chart_input_memo.get(); // 這裡在 Effect 內部，是完全安全的追蹤環境
        #[cfg(target_family = "wasm")]
        {
            DEBOUNCE_TIMER.with(|id| {
                if let Some(w) = web_sys::window() {
                    let old = id.get();
                    if old >= 0 {
                        w.clear_timeout_with_handle(old);
                    }
                    let cb = wasm_bindgen::closure::Closure::once(move || {
                        set_chart_input.set(new_ci);
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
            set_chart_input.set(new_ci);
        }
    });

    // ─── Plot resource（維持不變，依然安全偵聽 chart_input） ───────────────
    let plot_resource = LocalResource::new(move || async move {
        let ci = chart_input.get();
        generate_plot(ci)
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

            // ── 情境摘要卡片（重構後：邏輯清晰、極易讀） ───────────────────
            <div class="summary-card">
                {move || {
                    let ci = chart_input.get();
                    let sage = start_age.get();
                    let tage = target_age.get();

                    // 呼叫純計算邏輯
                    let (future_desc, end_str, roi_info) = derive_future_summary(
                        &ci, asset_wan.get(), future_mode.get()
                    );

                    let roi = ci.anchor_roi_pct;

                    match (ci.hist_years, ci.lump_sum == 0.0) {
                        // 情境 A：白手起家（無歷史、無起始本金）
                        (0, true) => {
                            let intro = if sage == 0 { "👶 幫新生兒從 0 歲白手起家 — " } else { "📊 規劃從 " };
                            view! { <span class="summary-text">
                                {intro} {if sage > 0 { format!("{} 歲出發 — ", sage) } else { "".to_string() }}
                                {future_desc} "，預計年化報酬率 " {format!("{roi:.2}")} "% 在 "
                                <strong>{tage}</strong> " 歲時" {end_str}
                            </span> }.into_any()
                        },

                        // 情境 B：單純單筆配置（無歷史、有起始本金）
                        (0, false) => {
                            let intro = if sage == 0 { "👶 幫小孩從 0 歲配置 — " } else { "💰 規劃從 " };
                            let ls_desc = format!("起始本金 {}", format_twd_financial(ci.lump_sum));
                            view! { <span class="summary-text">
                                {intro} {if sage > 0 { format!("{} 歲配置 — ", sage) } else { "".to_string() }}
                                {ls_desc} "，" {future_desc} "，預計年化報酬率 " {format!("{roi:.2}")} "% 在 "
                                <strong>{tage}</strong> " 歲時" {end_str}
                            </span> }.into_any()
                        },

                        // 情境 C：已有過去投資歷史
                        (_hist, _) => {
                            view! { <span class="summary-text">
                                "自 " <strong>{sage}</strong> " 歲起每月投資 " <strong>{format_twd_financial(ci.h_inv)}</strong>
                                "，至今已投 " <strong>{ci.hist_years}</strong> " 年" {roi_info}
                                "。調整戰略為：" {future_desc} "，期望在 "
                                <strong>{tage}</strong> " 歲時" {end_str}
                            </span> }.into_any()
                        }
                    }
                }}
            </div>

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
                                    <div class="input-hint">{move || format!("= {}", format_twd_financial(h_inv()))}</div>
                                </div>
                            </Show>
                            // 5. 現有資產 / 起始資金（動態標籤，不允許負數）
                            <div class="control-group">
                                <label class=move || {
                                    if hist_years() > 0 { "control-label accent" } else { "control-label" }
                                }>
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
                                        match roi_pct() {
                                            None => view! { <div class="input-hint warning">"⚠️ 無法推估，請確認數字"</div> }.into_any(),
                                            Some(r) if r < -10.0 => view! { <div class="input-hint warning">{format!("⚠️ 隱含 {:.2}%/年，嚴重虧損，請確認", r)}</div> }.into_any(),
                                            Some(r) if r < 0.0 => view! { <div class="input-hint warning">{format!("⚠️ 隱含 {:.2}%/年，目前虧損中", r)}</div> }.into_any(),
                                            Some(r) if r > 25.0 => view! { <div class="input-hint warning">{format!("⚠️ 隱含 {:.2}%/年，請再確認是否正確", r)}</div> }.into_any(),
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
                                        {move || (0..=6).map(|r| {
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

// =====================================================================
// # 7. 測試模組 - 第一部分：智慧型金融格式化單元測試
// =====================================================================
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
        assert_eq!(format_twd_financial(0.0), "0");
        assert_eq!(format_twd_financial(150.0), "150");
        assert_eq!(format_twd_financial(9999.0), "9,999");
        assert_eq!(format_twd_financial(-8500.0), "-8,500");
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
        let normal_row = make_clean_text_row("ROI  5%", "500萬", "500萬", false);
        assert!(normal_row.contains("style='font-family:Consolas,monospace;'"));
        assert!(normal_row.contains("ROI  5%"));
        // 由於沒有折現落差，不應該出現「折現：」字樣
        assert!(!normal_row.contains("折現："));

        let discount_row = make_clean_text_row("ROI 10%", "1,000萬", "800萬", false);
        assert!(discount_row.contains("折現："));

        let highlight_row = make_clean_text_row("ROI 10%", "1億", "1億", true);
        assert!(highlight_row.contains("color:#F43F5E; font-weight:bold;"));
    }

    #[test]
    fn test_short_text_row() {
        let short_row = make_short_clean_text_row("ROI  5%", "350萬", true);
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
        let anchor_roi_pct = 10.5; // 🎯 修正 #3：傳入精確的 f64 歷史年化報酬率
        let inflation_rate = 2;
        let hist_years = 5;
        let total_years = 15;
        let lump_sum = 2000000.0; // 現有資產 200 萬

        // 🎯 核心修正：100% 完美對齊您的 7 參數函數簽章與型別順序
        let trends = calculate_true_pivot_trends(
            h_inv,
            f_inv,
            anchor_roi_pct,
            inflation_rate,
            hist_years,
            total_years,
            lump_sum,
        );

        // 驗證雜湊表是否完整生成 0% 到 20% 的 21 條預測線
        assert_eq!(trends.len(), 21);
        for r in 0..=20 {
            assert!(trends.contains_key(&r));
        }

        // 驗證總時間序列長度 (15年 * 12個月 + 1個起始點 = 181)
        let expected_months = total_years * 12 + 1;
        let anchor_idx = anchor_roi_pct.round() as usize;
        assert_eq!(trends[&anchor_idx].len(), expected_months);
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
        let anchor_idx = anchor_roi_pct.round() as usize;

        // 🎯 金融鐵律 1：在歷史期間（已發生），「名目資產」必定等於「實質資產」
        #[allow(clippy::needless_range_loop)]
        for m in 0..=hist_months {
            let (nominal, real) = trends[&anchor_idx][m];
            assert!(
                (nominal - real).abs() < 1e-4,
                "歷史期間（第 {} 個月）名目與實質應完全相等。名目: {}, 實質: {}",
                m,
                nominal,
                real
            );
        }

        // 🎯 金融鐵律 2：在歷史結算點（含）以前，所有 21 條預測線線路軌跡必須百分之百重合，消滅分叉！
        #[allow(clippy::needless_range_loop)]
        for m in 0..=hist_months {
            let anchor_val = trends[&anchor_idx][m];
            for r in 0..=20 {
                let current_val = trends[&r][m];
                assert!(
                    (current_val.0 - anchor_val.0).abs() < 1e-4,
                    "歷史期間所有 ROI 軌跡應完全重合（第 {} 個月不應出現分叉）。ROI {}: {}, ROI {}: {}",
                    m,
                    anchor_idx,
                    anchor_val.0,
                    r,
                    current_val.0
                );
            }
        }

        // 🎯 金融鐵律 3：歷史期的最後一個月（現在結算點），必須精確等於使用者填寫的 lump_sum，像素級鎖定！
        for r in 0..=20 {
            let current_current_asset = trends[&r][hist_months].0;
            assert!(
                (current_current_asset - lump_sum).abs() < 1e-4,
                "歷史結算點終點數值（{}）必須完美等於使用者宣告的資產（{}）",
                current_current_asset,
                lump_sum
            );
        }
    }

    #[test]
    fn test_future_inflation_law() {
        let h_inv = 10000.0;
        let f_inv = -15000.0; // 模擬每月實質提領 1.5 萬
        let anchor_roi_pct = 6.0;
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
        #[allow(clippy::needless_range_loop)]
        for m in 1..=total_months {
            let cumulative_inflation_factor = (1.0 + inflation_monthly_rate).powf(m as f64);

            for r in 0..=20 {
                let (nominal, real) = trends[&r][m];
                if real.abs() > 0.001 {
                    let calculated_nominal = real * cumulative_inflation_factor;
                    let diff_ratio = (nominal - calculated_nominal).abs() / nominal.abs();
                    assert!(
                        diff_ratio < 1e-4,
                        "未來區間必須嚴格遵循 名目 = 實質 * 累計通膨 的鐵律。第 {} 個月, ROI {}: 名目 {}, 計算值 {}",
                        m,
                        r,
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

        // 🎯 測試極端邊界：若歷史年數為 0，歷史與未來衔接點（即第0個月）應正常初始化為傳入的 lump_sum 起始資金
        let trends = calculate_true_pivot_trends(20000.0, 20000.0, 7.0, 2, 0, 10, lump_sum);

        for r in 0..=20 {
            let (nominal_start, real_start) = trends[&r][0];
            assert_eq!(nominal_start, lump_sum);
            assert_eq!(real_start, lump_sum);
        }
    }

    #[test]
    fn test_row_formatting_utilities() {
        // 驗證寬螢幕與窄螢幕文字產生器是否如預期運作，防止渲染文字格式倒退
        let row_long = make_clean_text_row("ROI 10%", "$1,000", "$1,200", true);
        assert!(row_long.contains("ROI 10%"));
        assert!(row_long.contains("$1,000"));

        let row_short = make_short_clean_text_row("ROI 5%", "$500", false);
        assert!(row_short.contains("ROI 5%"));
        assert!(row_short.contains("$500"));
    }

    /// 擴充測試 3：轉折點銜接與發散測試（修復了原本的雜湊表與元組語法錯誤）
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
        let anchor_idx = anchor_roi_pct.round() as usize;

        // 🔍 歷史點檢查：在歷史期間內，所有 21 條軌道的數值必須與主線完全重合
        #[allow(clippy::needless_range_loop)]
        for m in 0..=hist_months {
            let anchor_val = trends[&anchor_idx][m];
            for r in 0..=20 {
                assert_eq!(
                    trends[&r][m], // 修正：正確的雜湊表二維點選語法
                    anchor_val,
                    "在第 {} 個月（歷史期），ROI {}% 應該與主線完全重合",
                    m,
                    r
                );
            }
        }

        // 🔍 未來點檢查：超過歷史期後，高低 ROI 軌道必須在未來終點產生合理的發散分叉
        let final_idx = total_years * 12;
        let val_5pct = trends[&5][final_idx].0; // 修正：使用 .0 存取元組的第一個元素（名目資產）
        let val_15pct = trends[&15][final_idx].0;
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

        let entries = &trends[&10];

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
            anchor_roi_pct: 8.5,
            lump_sum: 2_000_000.0,
            f_inv: -8000.0,
            inflation_rate: 2,
            window_width: 1200,
        };

        // 驗證圖表引擎是否能正常吞下結構體，並產出合法的 JSON 配置
        let plot = generate_plot(ci);
        let json_str = plot.to_json();
        assert!(!json_str.is_empty(), "生成的圖表 JSON 配置字串不應為空");
    }
}
