import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 창이 둘이라 엔트리도 둘이다.
//   index.html   → 대시보드 (React)
//   overlay.html → 오버레이 (React 없이 순수 TS)
//
// 오버레이는 게임 위에 상시 떠 있는 창이라 가벼울수록 좋고, 내용도 숫자
// 몇 개뿐이라 React가 주는 게 없다. 같은 Tailwind·토큰 파이프라인은 그대로 탄다.
export default defineConfig({
  plugins: [react()],
  // Tauri가 고정 포트를 기대한다. 포트가 밀리면 devUrl과 어긋나 빈 창이 뜬다.
  server: { port: 5173, strictPort: true },
  build: {
    rollupOptions: {
      // root 기준 상대 경로. node:path와 __dirname을 끌어들이면 설정 파일 하나
      // 때문에 @types/node가 필요해진다.
      input: {
        dashboard: "index.html",
        overlay: "overlay.html",
      },
    },
  },
});
