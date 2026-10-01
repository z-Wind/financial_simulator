use plotly::common::{Anchor, DashType, Font, HoverInfo, Label, Line, TickMode, Title};
use plotly::configuration::DisplayModeBar;
use plotly::layout::{
    Annotation, Axis, AxisType, DragMode, HoverMode, ItemClick, Layout, Legend, Margin, Shape,
    ShapeLayer, ShapeLine, ShapeType,
};
use plotly::{Configuration, Plot, Scatter};
use std::rc::Rc;

use crate::app::{
    CHART_Y_DEFAULT_LOG_MAX, CHART_Y_DEFAULT_MAX, CHART_Y_HEADROOM_MULTIPLIER,
    CHART_Y_VISUAL_FLOOR, ChartInput, NARROW_WIDTH_BREAKPOINT, TrendRoute,
};

pub(crate) fn format_with_commas(val: f64, precision: usize) -> String {
    let factor = 10.0_f64.powi(precision as i32);
    let rounded = (val.abs() * factor).round() / factor;

    let s = format!("{:.1$}", rounded, precision);
    let parts: Vec<&str> = s.split('.').collect();
    let num_part = parts[0];

    // 根據實際數值，動態計算是否需要留負號的空間
    let sign_space = if val < 0.0 { 1 } else { 0 };

    // 根據 precision，動態計算小數點與小數位所佔用的空間
    let decimal_space = if precision > 0 { 1 + precision } else { 0 };

    let mut result =
        String::with_capacity(num_part.len() + (num_part.len() / 3) + sign_space + decimal_space);

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

/// 格式化新台幣（TWD）財務大額數字，自動轉換單位：元、萬、億、兆
pub(crate) fn format_twd_financial(val: f64) -> String {
    let abs_val = val.abs();

    // 定義大額單位配置：(門檻值, 單位名稱, 是否強制顯示1位小數)
    // 依數值由大到小排列
    let units = [
        (1e12, "兆", true), // 兆
        (1e8, "億", true),  // 億
    ];

    // 1. 處理億級與兆級以上的巨額數字
    for &(threshold, unit, force_decimal) in &units {
        if abs_val >= threshold {
            let unit_val = val / threshold;
            // 根據配置決定是否強制保留 1 位小數（如 1.0億、1.5兆）
            let decimals = if force_decimal { 1 } else { 0 };
            return format!("{}{}", format_with_commas(unit_val, decimals), unit);
        }
    }

    // 2. 處理萬級數字（1萬 <= val < 1億）
    if abs_val >= 10_000.0 {
        let val_in_wan = val / 10_000.0;
        let abs_wan = val_in_wan.abs();

        // 判斷是否為整萬（誤差小於 0.01）
        let is_round = (abs_wan - abs_wan.round()).abs() < 0.01;
        let decimals = if is_round { 0 } else { 1 };

        return format!("{}萬", format_with_commas(val_in_wan, decimals));
    }

    // 3. 處理小於 1 萬的數字（直接顯示千分位整數）
    format!("{}元", format_with_commas(val, 0))
}

/// 格式化 ROI 為固定欄寬標籤，用於 hover tooltip 與圖例對齊。
///
/// 兩種模式的數字欄位都固定寬度 6（右對齊），確保 `"ROI "` + 6 碼 + `"%"` = 10 字元，
/// 與 `make_clean_text_row` 的 `pad_w = 10` 完全吻合，天然對齊、不需額外補空白：
/// - `is_major = false`（整數線，範圍 0..=20）：`{:>6}` → `"ROI      5%"` / `"ROI     20%"`
/// - `is_major = true`（主線，可能為任意浮點且含負值）：`{:>6.2}` 四捨五入到小數點後二位
///   → `"ROI   8.50%"` / `"ROI  12.50%"` / `"ROI  -5.50%"` / `"ROI -25.50%"`
///
/// 用 Rust 內建的數值寬度格式化（四捨五入 + 右對齊補空白），涵蓋 -99.00% ~ 50.00% 的
/// 完整範圍都不會超出 5 個字元，不會再有這個問題。
pub(crate) fn fmt_roi_label(roi_pct: f64, is_major: bool) -> String {
    if is_major {
        format!("ROI {:>6.2}%", roi_pct)
    } else {
        format!("ROI {:>6}%", roi_pct as usize)
    }
}

pub(crate) fn make_clean_text_row(
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

/// 產生動態圖表標註 (Annotation)
pub(crate) fn get_annotations(
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

    // X 軸用真實年齡；Y 軸對數軸需設下限（min 1萬）防止 log(0)
    let (x_pos, y_val_log, text_str, show_arrow, ax, ay) = if hist_years > 0 {
        let label_text = if let Some(actual_roi) = anchor_roi_pct {
            format!(
                "📍 現況錨定 ({:.2}%): {}",
                actual_roi,
                format_twd_financial(amt_now)
            )
        } else {
            format!("📍 現況結算: {}", format_twd_financial(amt_now))
        };

        (
            (start_age + hist_years) as f64,
            amt_now.max(CHART_Y_VISUAL_FLOOR).log10(), // 強制限低防禦對數軸爆炸
            label_text,
            true,
            0.0,
            60.0,
        )
    } else if lump_sum > 0.0 {
        // 註：UI 層已保證 lump_sum ≥ 0（負債起步會讓 ROI 反推與後續模擬失真，
        // 故不開放輸入負值），此處不需再處理負數分支。
        (
            start_age as f64,
            lump_sum.max(CHART_Y_VISUAL_FLOOR).log10(),
            format!("💰 起始資金: {}", format_twd_financial(lump_sum)),
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

/// 圖表渲染核心引擎
pub(crate) fn generate_plot(ci: ChartInput, sorted_trends: Vec<TrendRoute>) -> Plot {
    let is_narrow = ci.window_width < NARROW_WIDTH_BREAKPOINT;
    let is_inflation = ci.inflation_rate > 0.0;
    let total_months = ci.total_years * 12;
    let hist_months = ci.hist_years * 12;

    // X 軸以真實年齡為單位，所有 trace 共用同一份資料（Rc 避免複製）
    let x_numeric_timeline: Vec<f64> = (0..=total_months)
        .map(|m| ci.start_age as f64 + (m as f64 / 12.0))
        .collect();

    let shared_x = Rc::new(x_numeric_timeline);

    let colors: Vec<String> = (0..=20)
        .map(|i| format!("rgba({}, {}, 255, 0.8)", 50 + i * 8, 80 + i * 5))
        .collect();

    // 預配置精確容量，消滅記憶體劇烈波動與碎片化
    let mut hover_labels_text = Vec::with_capacity(total_months + 1);

    let anchor_route_opt = sorted_trends.iter().find(|r| r.is_anchor);
    let anchor_label = match ci.anchor_roi_pct {
        Some(roi_val) => fmt_roi_label(roi_val, true),
        _ => "ROI   ----%".to_string(),
    };

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
            if let Some(anchor_route) = anchor_route_opt {
                let amt = anchor_route.data[m];
                lines.push(make_clean_text_row(
                    &anchor_label,
                    &format_twd_financial(amt.0),
                    &format_twd_financial(amt.1),
                    false,
                    is_inflation,
                    is_narrow,
                ));
            }
        } else {
            for route in sorted_trends.iter().rev() {
                let is_integer = (route.roi_pct - route.roi_pct.round()).abs() < f64::EPSILON;

                if route.is_anchor || is_integer {
                    let label = if route.is_anchor {
                        fmt_roi_label(ci.anchor_roi_pct(), true)
                    } else {
                        fmt_roi_label(route.roi_pct, false)
                    };
                    let amt = route.data[m];
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

    // 依序繪製跡線
    for route in sorted_trends.iter().rev() {
        let roi_floor = route.roi_pct.floor() as usize;
        let is_integer = (route.roi_pct - route.roi_pct.round()).abs() < f64::EPSILON;
        let is_p =
            (is_integer && [5, 10, 15, 20].contains(&(route.roi_pct as usize))) || route.is_anchor;
        let amt_future = route.data[total_months];

        let label = if is_narrow {
            fmt_roi_label(route.roi_pct, !route.is_anchor)
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
            3.5
        } else if is_p {
            2.0
        } else {
            0.8
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

    // 0% 本金虛線及佈局配置
    if let Some(base_route) = sorted_trends
        .iter()
        .find(|r| !r.is_anchor && r.roi_pct.abs() < f64::EPSILON)
    {
        let amt_principal_future = base_route.data[total_months];
        let principal_name = make_clean_text_row(
            if is_narrow {
                "ROI      0%"
            } else {
                "ROI      0% 本金"
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
            "<br><span style='font-size: 13px; color: #2DD4BF; letter-spacing: 0.5px;'>📊 戰略配置 ── 起始 {}歲 ({}/月{}) | 現況 {}歲 | 目標 {}歲 [{}] | 折現通膨 {:.1}%/年</span>",
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

    // 座標防禦刻度
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

    // 縱向分水嶺定位線
    let mut shapes = Vec::new();
    let f_years = ci.total_years.saturating_sub(ci.hist_years);
    let x_positions: Vec<f64> = if ci.hist_years > 0 && ci.total_years >= ci.hist_years {
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

    // Y 軸動態上限：掃描所有軌道最大值
    let mut max_val = CHART_Y_DEFAULT_MAX;
    for route in sorted_trends.iter() {
        for &(nominal, _) in route.data.iter() {
            if nominal > max_val {
                max_val = nominal;
            }
        }
    }
    // Y 軸：對數軸下限固定 1 萬，避免近零值無限下拉
    let y_min_log = CHART_Y_VISUAL_FLOOR.log10();
    let y_max_log = if max_val > CHART_Y_VISUAL_FLOOR {
        (max_val * CHART_Y_HEADROOM_MULTIPLIER).log10()
    } else {
        CHART_Y_DEFAULT_LOG_MAX // 最大資產不到 10 萬時，預設給予 10 萬（5.0）的對數上限空間
    };

    // Y 軸刻度候選（中文單位，動態篩選落在可視範圍內的）
    let all_potential_ticks: Vec<(f64, &str)> = vec![
        (1e4, "1萬"),
        (1e5, "10萬"),
        (1e6, "100萬"),
        (1e7, "1,000萬"),
        (1e8, "1億"),
        (1e9, "10億"),
        (1e10, "100億"),
        (1e11, "1,000億"),
        (1e12, "1兆"),
        (1e13, "10兆"),
        (1e14, "100兆"),
        (1e15, "1,000兆"),
    ];

    let mut dynamic_y_vals = Vec::new();
    let mut dynamic_y_text = Vec::new();

    for (val, text) in all_potential_ticks {
        let val_log = val.log10();
        if val_log >= y_min_log && val_log <= y_max_log + 0.3 {
            dynamic_y_vals.push(val);
            dynamic_y_text.push(text.to_string());
        }
    }

    // 座標軸、圖例
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

    #[test]
    fn test_fmt_roi_label_integer() {
        // 整數 ROI 格式（數字欄固定寬度 5）
        assert_eq!(fmt_roi_label(5.0, false), "ROI      5%");
        assert_eq!(fmt_roi_label(10.0, false), "ROI     10%");
        assert_eq!(fmt_roi_label(20.0, false), "ROI     20%");
    }

    #[test]
    fn test_fmt_roi_label_float_consistency() {
        // 四捨五入到小數點後二位，右對齊補到固定寬度 6
        assert_eq!(fmt_roi_label(8.5, true), "ROI   8.50%");
        assert_eq!(fmt_roi_label(12.5, true), "ROI  12.50%");
        assert_eq!(fmt_roi_label(0.0, true), "ROI   0.00%");
        assert_eq!(fmt_roi_label(-5.5, true), "ROI  -5.50%");
    }

    // 改用數值寬度格式化，-99.90% ~ 50.00% 全範圍都應正確顯示。
    #[test]
    fn test_fmt_roi_label_double_digit_negative_regression() {
        assert_eq!(fmt_roi_label(50.0, true), "ROI  50.00%");
        assert_eq!(fmt_roi_label(-25.5, true), "ROI -25.50%");
        assert_eq!(fmt_roi_label(-10.0, true), "ROI -10.00%");
        assert_eq!(fmt_roi_label(-99.9, true), "ROI -99.90%");
    }
}
