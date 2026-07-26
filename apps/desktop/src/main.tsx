import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

// Preflight가 먼저, 토큰이 나중. 우리 .t-* 유틸리티가 Preflight를 이겨야 한다.
import "./styles/index.css";
import "./styles/tokens.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
