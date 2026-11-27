import { isTauri } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./index.css";
import { sessionApi } from "./lib/api";
import { AppProvider } from "./lib/store";

// 页面重新加载后，上一次页面打开的终端已经没有界面了。
if (isTauri()) sessionApi.closeAll().catch(() => {});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppProvider>
      <App />
    </AppProvider>
  </StrictMode>,
);
