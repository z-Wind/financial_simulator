use leptos::prelude::*;
use plotly::common::{Anchor, DashType, Font, HoverInfo, Label, Line, TickMode, Title};
use plotly::configuration::DisplayModeBar;
use plotly::layout::{
    Annotation, Axis, AxisType, DragMode, HoverMode, ItemClick, Layout, Legend, Margin, Shape,
    ShapeLayer, ShapeLine, ShapeType,
};
use plotly::{Configuration, Plot, Scatter};
use std::collections::HashMap;

// =====================================================================
// # 1. 智慧型金融格式化與等寬對齊模組
// =====================================================================

/// 輔助函式：將數字加上千分位逗號（等同 Python f"{val:,.1f}"）
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

pub fn format_twd_financial(val: f64) -> String {
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

pub fn make_clean_text_row(name_str: &str, val_str: &str) -> String {
    format!(
        "<span style='font-family:Consolas,monospace;'>{: <8} │ {: >9}</span>",
        name_str, val_str
    )
}

// =====================================================================
// # 2. 真·雙階段獨立複利演算法（時序連續版）
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
// # 3. 動態標籤產生模組
// =====================================================================
pub fn get_annotations(
    trends: &HashMap<usize, Vec<f64>>,
    hist_years: usize,
    anchor_roi: usize,
    _total_years: usize,
) -> Vec<Annotation> {
    let mut ann_list = Vec::new();
    let hist_idx = hist_years * 12;

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
            60.0,
        )
    } else {
        (4.0, "📍 全新出發點 (第 0 年)".to_string(), false, 0.0, 0.0)
    };

    ann_list.push(
        Annotation::new()
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
            .border_pad(5.0),
    );

    ann_list
}

// =====================================================================
// # 4. 全自動響應式圖表引擎
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
        colors.push(format!("rgba({}, {}, 255, 0.8)", 50 + i * 8, 80 + i * 5));
    }

    let mut hover_labels_text = Vec::new();
    for (m, _) in x_numeric_timeline.iter().enumerate() {
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
        let is_p = [5, 10, 15, 20].contains(&r) || r == anchor_roi;
        let total_idx = total_years * 12;
        let amt_future = trends[&r][total_idx]; // 取得這條線在第 40/90 年的終值

        // 💡 彭博風格圖例字串：利用等寬字型排版，讓 ROI 與終值數字在 Legend 列表裡整齊排列
        let legend_name = if r == anchor_roi {
            format!(
                "<span style='font-family:Consolas,monospace; color:#F43F5E; font-weight:bold;'>ROI {:2}% 主線 │ {}</span>",
                r,
                format_twd_financial(amt_future)
            )
        } else {
            format!(
                "<span style='font-family:Consolas,monospace;'>ROI {:2}% 未來 │ {}</span>",
                r,
                format_twd_financial(amt_future)
            )
        };

        let mut trace = Scatter::new(x_numeric_timeline.clone(), trends[&r].clone())
            .name(legend_name)
            .hover_info(HoverInfo::Skip);

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
        plot.add_trace(trace);
    }

    // 未來純本金線的圖例
    let amt_principal_future = trends[&0][total_years * 12];
    let principal_name = format!(
        "<span style='font-family:Consolas,monospace;'>ROI  0% 本金 │ {}</span>",
        format_twd_financial(amt_principal_future)
    );

    let principal_trace = Scatter::new(x_numeric_timeline.clone(), trends[&0].clone())
        .name(principal_name)
        .line(Line::new().color("#A0AEC0").width(2.5).dash(DashType::Dash))
        .text_array(hover_labels_text)
        .hover_template("%{text}<extra></extra>")
        .show_legend(true);
    plot.add_trace(principal_trace);

    let future_plan_text = if f_inv > 0.0 {
        format!("每月改投{}", format_twd_financial(f_inv))
    } else if f_inv < 0.0 {
        format!("每月提領{}", format_twd_financial(f_inv.abs()))
    } else {
        "不再投入(利滾利)".to_string()
    };

    let strategy_subtitle = format!(
        "<br><span style='font-size: 13px; color: #2DD4BF; font-weight: normal; letter-spacing: 0.5px; line-height: 1.6;'>\
        📊 配置戰略 ── 已投入：{} 年 ({}/月) │ 未來：{} 年 ({})</span>",
        hist_years,
        format_twd_financial(h_inv),
        total_years - hist_years,
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

    let legend = Legend::new()
        .y_anchor(Anchor::Bottom)
        .y(0.1)
        .x_anchor(Anchor::Right)
        .x(0.99)
        .background_color("rgba(30, 41, 59, 0.8)")
        .border_color("#475569")
        .border_width(1)
        .font(Font::new().color("#F1F5F9"))
        .item_click(ItemClick::False)
        .item_double_click(ItemClick::False);

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
    // 開啟 responsive，讓圖表能隨容器/視窗尺寸變化（例如手機旋轉、桌機拉伸視窗）自動重新繪製大小
    plot.set_configuration(
        Configuration::new()
            .responsive(true)
            .display_mode_bar(DisplayModeBar::False),
    );
    plot
}

// =====================================================================
// # 5. Leptos 網頁 UI 組件與主入口
// =====================================================================
#[component]
fn App() -> impl IntoView {
    let (total_years, set_total_years) = signal(40_usize);
    let (hist_years, set_hist_years) = signal(15_usize);
    let (h_inv, set_h_inv) = signal(30000.0_f64);
    let (anchor_roi, set_anchor_roi) = signal(10_usize);
    let (f_inv, set_f_inv) = signal(0.0_f64);
    let (panel_open, set_panel_open) = signal(true);

    // 核心安全修正：利用 Resource 統一追蹤依賴項，防止 Effect 非同步競態造成的畫面劇烈閃爍
    let plot_resource = LocalResource::new(move || async move {
        // 直接在非同步閉包內讀取所有 Signal，Leptos 會全自動建立響應式追蹤鏈
        generate_plot(
            total_years.get(),
            hist_years.get(),
            h_inv.get(),
            anchor_roi.get(),
            f_inv.get(),
        )
    });

    // 監聽並將新數據繪製到 DOM 樹上
    Effect::new(move |_| {
        if let Some(p) = plot_resource.get() {
            #[cfg(target_family = "wasm")]
            {
                let element_id = "financial-graph";
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    if doc.get_element_by_id(element_id).is_some() {
                        // 💡 關鍵修正：在外面直接把 p 解構或直接轉移給內部的 async 區塊
                        leptos::task::spawn_local(async move {
                            let _ = plotly::bindings::react(element_id, &p).await;
                        });
                    }
                }
            }

            // 💡 確保在非 WASM 環境下（如編譯測試階段）消除未使用的警告
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

    view! {
        <div class="app-container">
            <h2 class="app-title">"人生財務戰略導航：現況資產錨定與未來變革推演模擬器"</h2>

            // 💡 可收合的控制面板
            <div class=move || if panel_open.get() { "controls-panel panel-open" } else { "controls-panel" }>
                <button class="controls-summary" on:click=move |_| set_panel_open.update(|v| *v = !*v)>
                    <span class="summary-title">"⚙️ 模擬參數設定"</span>
                    <span class=move || if panel_open.get() { "panel-status-badge badge-open" } else { "panel-status-badge" }>
                        <span class="badge-text">
                            {move || if panel_open.get() { "收合設定" } else { "修改參數" }}
                        </span>
                        <span class="badge-arrow">"▾"</span>
                    </span>
                </button>

                <Show when=move || panel_open.get()>
                    <div class="controls-body">
                        <div class="controls-grid">

                            // 🧿 步驟零：設定總模擬年數 (包含展開的 20-90 年清單)
                            <div class="control-group">
                                <label class="control-label">"🧿 零：設定總模擬年數"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() {
                                            set_total_years.set(val);
                                            if hist_years.get() > val { set_hist_years.set(val); }
                                        }
                                    }>
                                        {move || [20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 80, 85, 90].iter().map(|&y| {
                                            view! { <option value=y selected=move || total_years.get() == y>{format!("🔮 總共模擬 {} 年", y)}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 📆 步驟一：設定已投資年數
                            <div class="control-group">
                                <label class="control-label">"📆 一：設定已投資年數"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() {
                                            set_hist_years.set(val.min(total_years.get()));
                                        }
                                    }>
                                        {move || (0..=total_years.get()).map(|y| {
                                            let label = if y == 0 { "🆕 剛要開始 (0 年)".to_string() } else { format!("⏳ 已投入 {} 年", y) };
                                            view! { <option value=y selected=move || hist_years.get() == y>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 🟢 步驟二：設定過去每月投入金額
                            <div class="control-group">
                                <label class="control-label">"🟢 二：設定過去每月投入金額"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<f64>() { set_h_inv.set(val); }
                                    }>
                                        {move || (1..=20).map(|i| {
                                            let h = (i * 5000) as f64;
                                            view! { <option value=h selected=move || h_inv.get() == h>{format!("💰 每月投入：{}", format_twd_financial(h))}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 🎯 步驟三：目前資產錨定過去報酬
                            <div class="control-group">
                                <label class="control-label accent">"🎯 三：目前資產錨定過去報酬"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<usize>() { set_anchor_roi.set(val); }
                                    }>
                                        {move || (0..=20).map(|r| {
                                            let label = if r == 0 { "⚖️ 報酬率：0%".to_string() } else { format!("📈 年化報酬率：{}%", r) };
                                            view! { <option value=r selected=move || anchor_roi.get() == r>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                            // 🔵 步驟四：模擬未來每月改投金額
                            <div class="control-group">
                                <label class="control-label">"🔵 四：模擬未來每月改投金額"</label>
                                <div class="select-wrapper">
                                    <select class="control-select" on:change=move |ev| {
                                        if let Ok(val) = event_target_value(&ev).parse::<f64>() { set_f_inv.set(val); }
                                    }>
                                        {move || (-20..=20).map(|i| {
                                            let f = (i * 5000) as f64;
                                            let label = if i == 0 { "🛑 未來不再投入".to_string() }
                                                else if i > 0 { format!("💰 每月改投：{}", format_twd_financial(f)) }
                                                else { format!("💸 每月提領：{}", format_twd_financial(f.abs())) };
                                            view! { <option value=f selected=move || f_inv.get() == f>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                </div>
                            </div>

                        </div>
                    </div>
                </Show>
            </div>

            // 📊 圖表渲染容器 (最低高度會影響 hover)
            <div
                id="financial-graph"
                class="graph-container"
                style=move || {
                    if panel_open.get() {
                        "height: clamp(520px, 68vh, 720px);"
                    } else {
                        "height: clamp(520px, 82vh, 860px);"
                    }
                }
            ></div>
        </div>
    }
}

// =====================================================================
// # 6. Wasm 應用程式主進入點 (Client-Side Rendering)
// =====================================================================
fn main() {
    // 啟動 Leptos 的用戶端日誌追蹤（方便在瀏覽器 F12 主控台除錯）
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();

    // 將 App 元件掛載至網頁 HTML 的 <body> 中
    leptos::prelude::mount_to_body(App);
}
