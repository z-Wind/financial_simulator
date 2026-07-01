# 📈 人生財務戰略導航模擬器 (Financial Simulator)
[![Get Data and Deploy](https://github.com/z-Wind/financial_simulator/actions/workflows/deploy.yml/badge.svg)](https://github.com/z-Wind/financial_simulator/actions/workflows/deploy.yml)

👉 **[點此進入線上模擬器](https://z-Wind.github.io/financial_simulator/)**

一個基於 WebAssembly (WASM) 技術開發的高效能網頁端財務規劃與模擬工具。本專案採用 Rust 語言的 **Leptos** 框架進行用戶端渲染 (CSR)，並整合 **Plotly.js** 提供流暢、互動性高的財務趨勢圖表，幫助使用者視覺化未來的人生財務戰略。

## 🚀 特色功能

- **高效能 WASM 驅動**：利用 Rust 與 WebAssembly，帶來極速的計算響應與流暢網頁體驗。
- **動態圖表視覺化**：整合 Plotly 繪圖庫，直觀呈現資產增長、退休準備金與現金流變化。
- **響應式介面設計**：支援跨裝置瀏覽，隨時隨地進行財務健康檢查與策略調整。
- **隱私安全**：純客戶端 (CSR) 計算，所有財務數據均保留在您的瀏覽器中，絕不上傳伺服器。

## 🛠️ 開發技術棧

- **前端框架**：[Leptos 0.8 (CSR Mode)](https://leptos.dev) - 全功能、響應式的 Rust 前端框架。
- **圖表渲染**：[Plotly For Rust](https://github.com/plotly/plotly.rs) & Plotly.js (via CDN)。
- **數據處理**：Serde & Serde_json (高效的序列化與反序列化工具)。
- **打包工具**：[Trunk](https://trunk-rs.github.io/trunk/) - WASM 網頁應用程式打包利器。

## 💻 本地開發指南

要在本地環境運行此專案，請確保您已安裝 Rust 工具鏈。

### 1. 安裝必要工具

首先，安裝 WebAssembly 目標編譯群組：
```bash
rustup target add wasm32-unknown-unknown
```

接著，安裝編譯與打包工具 Trunk：
```bash
cargo install --locked trunk
```

### 2. 啟動本地開發伺服器

在專案根目錄下執行以下指令：
```bash
trunk serve
```
啟動後，開啟瀏覽器造訪 `http://127.0.0.1:8080` 即可預覽並進行開發。

## 📦 部署與發布

專案已配置 **GitHub Actions** 自動化工作流。每當程式碼推送到 `main` 分支時，系統會自動透過 Trunk 編譯成 WASM，並將產出部署至 **GitHub Pages**。

## 📄 授權條款

本專案採用 [MIT License](LICENSE) 授權條款。
