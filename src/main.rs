use leptos::prelude::*;
use plotly::common::{Anchor, DashType, Font, HoverInfo, Label, Line, TickMode, Title};
use plotly::layout::{
    Annotation, Axis, AxisType, HoverMode, Layout, Legend, Margin, Shape, ShapeLayer, ShapeLine,
    ShapeType,
};
use plotly::{Configuration, Plot, Scatter};
use std::collections::HashMap;

// =====================================================================
// # 1. 智慧型金融格式化與等寬對齊模組
// =====================================================================

/// 輔助函式：將數字加上千分位逗號（等同 Python f"{val:,.1f}"）
fn format_with_commas(val: f64, precision: usize) -> String {
    let s = format!("{:.1$}", val.abs(), precision);
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

pub fn format_twd_financial(val: f64) -> String {
    let abs_val = val.abs();
    if abs_val >= 100_000_000.0 {
        format!("{}億", format_with_commas(val / 100_000_000.0, 1))
    } else if abs_val >= 10_000.0 {
        if (val % 10_000.0).abs() < f64::EPSILON {
            format!("{}萬", format_with_commas(val / 10_000.0, 0))
        } else {
            format!("{}萬", format_with_commas(val / 10_000.0, 1))
        }
    } else {
        format_with_commas(val, 0)
    }
}

pub fn make_clean_text_row(name_str: &str, val_str: &str) -> String {
    format!(
        "<span style='font-family:Consolas,monospace;'>{: <8} │ {: >9}</span>",
        name_str, val_str
    )
}
// =====================================================================
// # 2. 真·雙階段獨立複利演算法
// =====================================================================
pub fn calculate_true_pivot_trends(
    h_inv: f64,
    f_inv: f64,
    anchor_roi: usize,
    hist_years: usize,
    total_years: usize,
) -> HashMap<usize, Vec<f64>> {
    let hist_months = hist_years * 12;
    let total_months = total_years * 12;
    let future_months = total_months - hist_months;

    let anchor_monthly_rate = (1.0 + (anchor_roi as f64) / 100.0).powf(1.0 / 12.0) - 1.0;
    let mut hist_route = vec![0.0];
    let mut curr_balance = 0.0;

    for _ in 1..=hist_months {
        curr_balance = (curr_balance + h_inv) * (1.0 + anchor_monthly_rate);
        hist_route.push(curr_balance);
    }

    let new_initial_asset = *hist_route.last().unwrap_or(&0.0);
    let mut trends = HashMap::new();

    for r in 0..=20 {
        let monthly_rate = (1.0 + (r as f64) / 100.0).powf(1.0 / 12.0) - 1.0;
        let mut full_route = hist_route.clone();
        let mut curr_f = new_initial_asset;

        for _ in 1..=future_months {
            curr_f = (curr_f + f_inv) * (1.0 + monthly_rate);
            full_route.push(curr_f);
        }
        trends.insert(r, full_route);
    }

    trends
}

// =====================================================================
// # 3. 動態標籤產生模組 (🚀 終極重構：單獨剝離紅色關鍵主線終值標籤)
// =====================================================================
pub fn get_annotations(
    trends: &HashMap<usize, Vec<f64>>,
    hist_years: usize,
    anchor_roi: usize,
    total_years: usize,
) -> Vec<Annotation> {
    let mut ann_list = Vec::new();
    let hist_idx = hist_years * 12;
    let total_idx = total_years * 12;

    // 📍 歷史起點錨定初值
    let amt_now = trends[&anchor_roi][hist_idx];
    let (y_val_log, text_str, show_arrow, ax, ay) = if hist_years > 0 {
        (
            amt_now.max(10000.0).log10(),
            format!(
                "📍 初值錨定 ({}%): {}",
                anchor_roi,
                format_twd_financial(amt_now)
            ),
            true,
            0.0,
            60.0, // 💡 ax=0, ay=60 垂直向下沉
        )
    } else {
        (4.17, "📍 全新出發點 (第 0 年)".to_string(), false, 0.0, 0.0)
    };

    let ann_start = Annotation::new()
        .x(hist_years as f64)
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
        .border_pad(5.0);
    ann_list.push(ann_start);

    // 🚀 專屬紅色主線未來終值看板
    let amt_red_future = trends[&anchor_roi][total_idx];
    let ann_red = Annotation::new()
        .x(total_years as f64)
        .y(amt_red_future.max(10000.0).log10())
        .text(format!(
            "🎯 錨定主線({}%)未來終值: {}",
            anchor_roi,
            format_twd_financial(amt_red_future)
        ))
        .show_arrow(true)
        .arrow_head(2)
        .arrow_color("#F43F5E")
        .arrow_size(1.0)
        .arrow_width(2.0)
        .ax(-160.0)
        .ay(-80.0)
        .font(
            Font::new()
                .size(11)
                .color("#F43F5E")
                .family("Consolas, monospace"),
        )
        .background_color("rgba(15, 23, 42, 0.95)")
        .border_color("#F43F5E")
        .border_width(1.5)
        .border_pad(5.0);
    ann_list.push(ann_red);

    // 右側常駐標籤群
    for &r in &[0, 5, 10, 15, 20] {
        let amt_future = trends[&r][total_idx];
        let y_off = match r {
            20 => 14.0,
            15 => 7.0,
            10 => 0.0,
            5 => -7.0,
            _ => -14.0,
        };
        let ann_right = Annotation::new()
            .x(total_years as f64)
            .y(amt_future.max(10000.0).log10())
            .text(format!(
                "{}%未來終值: {}",
                r,
                format_twd_financial(amt_future)
            ))
            .show_arrow(false)
            .x_shift(65.0)
            .y_shift(y_off)
            .font(
                Font::new()
                    .size(9)
                    .color("#2DD4BF")
                    .family("Consolas, monospace"),
            )
            .background_color("rgba(15, 23, 42, 0.9)")
            .border_color("#475569")
            .border_width(1.0)
            .border_pad(3.0);
        ann_list.push(ann_right);
    }

    ann_list
}

// =====================================================================
// # 5. 全自動響應式圖表引擎 (21條全線路與完整 Hover 終極復刻版)
// =====================================================================
pub fn generate_plot(
    total_years: usize,
    hist_years: usize,
    h_inv: f64,
    anchor_roi: usize,
    f_inv: f64,
) -> Plot {
    let trends = calculate_true_pivot_trends(h_inv, f_inv, anchor_roi, hist_years, total_years);
    let total_months = total_years * 12;
    let hist_months = hist_years * 12;
    let x_numeric_timeline: Vec<f64> = (0..=total_months).map(|m| m as f64 / 12.0).collect();

    // 🎨 預算 21 條線的色彩矩陣 (對齊 Python colors 陣列)
    let mut colors = Vec::new();
    for i in 0..=20 {
        colors.push(format!("rgba({}, {}, 255, 0.8)", 100 + i * i * 3, 80 + i));
    }

    // ✨【關鍵核心復刻 1】：Hover 看板老老實實跑滿 0% 到 20% 全部 21 條線的幾何數據！
    let mut hover_labels_text = Vec::new();

    for (m, _time_val) in x_numeric_timeline.iter().enumerate() {
        let y = m / 12;
        let mo = m % 12;
        let time_header = if mo > 0 {
            format!("<b>第 {} 年 {} 個月</b>", y, mo)
        } else {
            format!("<b>第 {} 年整</b>", y)
        };

        let mut lines = vec![time_header, "────────────────────────".to_string()];

        if m <= hist_months {
            // 🟢 歷史區：只印出步驟三設定的指定報酬率（anchor_roi），消滅其餘重複數字！
            let val_str = format_twd_financial(trends[&anchor_roi][m]);
            lines.push(make_clean_text_row(
                &format!("ROI {:2}%", anchor_roi),
                &val_str,
            ));
        } else {
            // 🔵 未來區：跨過分水嶺，由大到小 (20 到 0) 完整塞入 21 條線的財富數字！
            for r in (0..=20).rev() {
                let val_str = format_twd_financial(trends[&r][m]);
                lines.push(make_clean_text_row(&format!("ROI {:2}%", r), &val_str));
            }
        }
        hover_labels_text.push(lines.join("<br>"));
    }

    let mut plot = Plot::new();

    // ✨【關鍵核心復刻 2】：背景默默繪製「全部 21 條線」，一條都不漏！
    for r in (0..=20).rev() {
        // 判定哪些線條需要顯式出現在右側圖例中 (5, 10, 15, 20 或 錨定主線)
        let is_p = [5, 10, 15, 20].contains(&r) || r == anchor_roi;

        let mut trace = Scatter::new(x_numeric_timeline.clone(), trends[&r].clone())
            .name(format!("未來 ROI {}%", r))
            .hover_info(HoverInfo::Skip); // 跳過單線提示，避免畫面混亂

        // 線條色彩與寬度設定
        let color = if r == anchor_roi {
            "#F43F5E".to_string()
        } else {
            colors[r].clone()
        };
        let width = if r == anchor_roi {
            3.5 // 錨定主線超加粗
        } else if is_p {
            2.0 // 核心觀測線加粗
        } else {
            0.8 // 其餘 16 條背景幾何線維持極細
        };

        trace = trace.line(Line::new().color(color).width(width));
        trace = trace.show_legend(is_p); // 💡 只有符合條件的核心線才顯示在圖例
        plot.add_trace(trace);
    }

    // 將 21 條全數據 Hover 看板數組注入「未來純本金 (0%)」線條中作為頂層觸發網格
    let principal_trace = Scatter::new(x_numeric_timeline.clone(), trends[&0].clone())
        .name("未來純本金 (0%)")
        .line(Line::new().color("#A0AEC0").width(2.5).dash(DashType::Dash))
        .text_array(hover_labels_text)
        .hover_template("%{text}<extra></extra>")
        .show_legend(true);
    plot.add_trace(principal_trace);

    // 戰略副標題組裝
    let future_plan_text = if f_inv > 0.0 {
        format!("每月改投{}", format_twd_financial(f_inv))
    } else if f_inv < 0.0 {
        format!("每月提領{}", format_twd_financial(f_inv.abs()))
    } else {
        "不再投入(利滾利)".to_string()
    };
    let remaining_years = total_years - hist_years;
    let strategy_subtitle = format!(
        "<br><span style='font-size: 13px; color: #2DD4BF; font-weight: normal; letter-spacing: 0.5px; line-height: 1.6;'>\
        📊 配置戰略 ── 已投入：{} 年 ({}/月) │ 未來：{} 年 ({})</span>",
        hist_years,
        format_twd_financial(h_inv),
        remaining_years,
        future_plan_text
    );

    let anns = get_annotations(&trends, hist_years, anchor_roi, total_years);

    // X 軸刻度文字設定
    let (x_ticks, x_tick_text) = if hist_years > 0 {
        (
            vec![0.0, hist_years as f64, total_years as f64],
            vec![
                "🎬 第 0 年 (歷史起點)".to_string(),
                format!("📍 第 {} 年 (現在結算點)", hist_years),
                format!("🏁 第 {} 年 (未來終點)", total_years),
            ],
        )
    } else {
        (
            vec![0.0, total_years as f64],
            vec![
                "🎯 第 0 年 (現在起點)".to_string(),
                format!("🏁 第 {} 年 (未來終點)", total_years),
            ],
        )
    };

    // 縱向分水嶺格線設定 (Shapes)
    let mut shapes = Vec::new();
    let x_positions = if hist_years > 0 {
        vec![
            hist_years as f64 / 2.0,
            hist_years as f64,
            hist_years as f64 + (total_years - hist_years) as f64 / 2.0,
            total_years as f64,
        ]
    } else {
        vec![0.0, total_years as f64 / 2.0, total_years as f64]
    };

    for x_pos in x_positions {
        let is_now = x_pos == hist_years as f64 && hist_years > 0;
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

    // 調用你親自修復通過的真·無錯誤高規格 Layout
    let x_axis = Axis::new()
        .title(
            Title::new()
                .text("時間推移軸 (Timeline)")
                .font(Font::new().color("#F8FAFC").size(14)),
        )
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
        // ✨【關鍵核心修正 2】：完美復刻 Python 的 domain=[0.05, 1.0]，給底部保留完美呼吸空間
        .domain(&[0.05, 1.0])
        .auto_range(true);

    let legend = Legend::new()
        .y_anchor(Anchor::Top)
        .y(0.99)
        .x_anchor(Anchor::Left)
        .x(0.01)
        .background_color("rgba(30, 41, 59, 0.8)")
        .border_color("#475569")
        .border_width(1)
        .font(Font::new().color("#F1F5F9"));

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
        .title(title)
        .paper_background_color("#0F172A")
        .plot_background_color("#0F172A")
        .margin(Margin::new().left(60).right(40).top(48).bottom(55))
        .hover_mode(HoverMode::X)
        .hover_label(hover_label)
        .legend(legend)
        .x_axis(x_axis)
        .y_axis(y_axis)
        .shapes(shapes)
        .annotations(anns);

    plot.set_layout(layout);
    // 開啟 responsive，讓圖表能隨容器/視窗尺寸變化（例如手機旋轉、桌機拉伸視窗）自動重新繪製大小
    plot.set_configuration(Configuration::new().responsive(true));
    plot
}

// =====================================================================
// # 6. Leptos 網頁 UI 組件與主入口 (終極修正版)
// =====================================================================
#[component]
fn App() -> impl IntoView {
    // 響應式狀態核心
    let (total_years, set_total_years) = signal(40_usize);
    let (hist_years, set_hist_years) = signal(15_usize);
    let (h_inv, set_h_inv) = signal(30000.0_f64);
    let (anchor_roi, set_anchor_roi) = signal(10_usize);
    let (f_inv, set_f_inv) = signal(0.0_f64);

    // 控制面板展開/收合狀態（預設展開）
    let (panel_open, set_panel_open) = signal(true);

    // 完美復刻 Callback 防呆機制
    Effect::new(move |_| {
        if hist_years.get() > total_years.get() {
            set_hist_years.set(total_years.get());
        }
    });

    let plot_view = move || {
        let p = generate_plot(
            total_years.get(),
            hist_years.get(),
            h_inv.get(),
            anchor_roi.get(),
            f_inv.get(),
        );
        let element_id = "financial-graph";
        #[cfg(target_family = "wasm")]
        leptos::task::spawn_local(async move {
            let _ = plotly::bindings::react(element_id, &p).await;
        });

        // 確保 Clippy 同步放行
        #[cfg(not(target_family = "wasm"))]
        {
            let _ = (element_id, p); // 避免 unused variable 警告
        }
    };

    Effect::new(move |_| {
        plot_view();
    });

    // 監聽面板收合狀態，當 panel_open 改變時，主動通知 Plotly 重新計算 RWD 高度
    Effect::new(move |_| {
        // 讀取訊號使其與此 Effect 綁定
        let _ = panel_open.get();

        #[cfg(target_family = "wasm")]
        {
            // 利用網頁標準的 window.request_animation_frame
            // 這是瀏覽器效能最好的核心 API，能完美同步 CSS 的 :has 伸縮動畫
            if let Some(window) = web_sys::window() {
                let _ = window.request_animation_frame(
                    &js_sys::Function::new_no_args(
                        "if(window.Plotly){ Plotly.Plots.resize(document.getElementById('financial-graph')); }"
                    )
                );
            }
        }
    });

    view! {
        <div class="app-container">
            <h2 class="app-title">"人生財務戰略導航：現況資產錨定與未來變革推演模擬器"</h2>

            // 💡 可收合的控制面板
            // 用 Leptos signal 管理展開/收合，避免 <details> 的 open 屬性被 Leptos reconcile 重置的問題
            <div class=move || if panel_open.get() { "controls-panel panel-open" } else { "controls-panel" }>
                // 標題列兼收合按鈕
                <button
                    class="controls-summary"
                    on:click=move |_| set_panel_open.update(|v| *v = !*v)
                >
                    <span class="summary-title">"⚙️ 模擬參數設定"</span>
                    <span class=move || if panel_open.get() { "panel-status-badge badge-open" } else { "panel-status-badge" }>
                        <span class="badge-text">
                            {move || if panel_open.get() { "收合設定" } else { "修改參數" }}
                        </span>
                        <span class="badge-arrow">"▾"</span>
                    </span>
                </button>

                // 收合時整個 body 被移出 DOM，展開時才插回來
                <Show when=move || panel_open.get()>
                    <div class="controls-body">
                        // 💡 響應式控制列：使用 CSS grid (auto-fit)，桌機排 5 欄，窄螢幕自動換行甚至單欄堆疊
                        <div class="controls-grid">

                            // 🧿 步驟零：設定總模擬年數
                            // ✨ 用 move || 包住，才能在 total_years 改變時重新計算 is_selected
                            <div class="control-group">
                                <label class="control-label">"🧿 步驟零：設定總模擬年數"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() { set_total_years.set(val); }
                                    }>
                                        {move || [30, 35, 40, 45, 50, 55, 60].iter().map(|&y| {
                                            let is_selected = total_years.get() == y;
                                            view! { <option value=y selected=is_selected>{format!("🔮 總共模擬 {} 年", y)}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 📆 步驟一：設定已投資年數（已有 move ||，不需再改）
                            <div class="control-group">
                                <label class="control-label">"📆 步驟一：設定已投資年數"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() { set_hist_years.set(val); }
                                    }>
                                        {move || (0..=total_years.get()).map(|y| {
                                            let label = if y == 0 { "🆕 剛要開始 (0 年)".to_string() } else { format!("⏳ 已投入 {} 年", y) };
                                            let is_selected = hist_years.get() == y;
                                            view! { <option value=y selected=is_selected>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 🟢 步驟二：設定過去每月投入金額
                            <div class="control-group">
                                <label class="control-label">"🟢 步驟二：設定過去每月投入金額"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<f64>() { set_h_inv.set(val); }
                                    }>
                                        {move || (1..=20).map(|i| {
                                            let h = (i * 5000) as f64;
                                            let label = format!("💰 每月投入：{}", format_twd_financial(h));
                                            let is_selected = h_inv.get() == h;
                                            view! { <option value=h selected=is_selected>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 🎯 步驟三：對照目前資產，錨定過去報酬率
                            <div class="control-group">
                                <label class="control-label accent">"🎯 步驟三：對照目前資產，錨定過去報酬率"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() {
                                            set_anchor_roi.set(val);
                                        }
                                    }>
                                        {move || (0..=20).map(|r| {
                                            let label = if r == 0 {
                                                "⚖️ 報酬率：0%".to_string()
                                            } else {
                                                format!("📈 年化報酬率：{}%", r)
                                            };
                                            let is_selected = anchor_roi.get() == r;
                                            view! { <option value=r selected=is_selected>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 🔵 步驟四：模擬未來每月改投金額
                            <div class="control-group">
                                <label class="control-label">"🔵 步驟四：模擬未來每月改投金額"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<f64>() { set_f_inv.set(val); }
                                    }>
                                        {move || (-20..=20).map(|i| {
                                            let f = (i * 5000) as f64;
                                            let label = if i == 0 {
                                                "🛑 未來不再投入".to_string()
                                            } else if i > 0 {
                                                format!("💰 每月改投：{}", format_twd_financial(f))
                                            } else {
                                                format!("💸 每月提領：{}", format_twd_financial(f.abs()))
                                            };
                                            let is_selected = f_inv.get() == f;
                                            view! { <option value=f selected=is_selected>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                        </div>
                    </div>
                </Show>
            </div>

            // 圖表渲染容器：高度改用 CSS clamp()，桌機/平板/手機平滑縮放；
            // 控制面板收合時（.panel-open 消失）:has() 讓圖表自動長高
            <div id="financial-graph" class="graph-container"></div>
        </div>
    }
}

fn main() {
    leptos::mount::mount_to_body(|| view! { <App /> });
}
