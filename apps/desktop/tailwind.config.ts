import type { Config } from "tailwindcss";

/**
 * anti-card 디자인 가이드를 따른다.
 *
 * 색은 **전부 CSS 변수의 의미 역할명을 가리킨다.** 여기에 색상 리터럴을 쓰면
 * 유저 테마 선택(v2)이 토큰 블록 교체로 끝나지 않는다 — PLAN.md §2.
 * 실제 값은 src/styles/tokens.css에 있다.
 */
export default {
  content: [
    "./index.html",
    "./overlay.html",
    "./src/**/*.{ts,tsx}",
    // anti-card는 Tailwind 클래스를 문자열로 들고 있는 컴포넌트 라이브러리다.
    // 이 글롭이 없으면 라이브러리가 쓰는 유틸리티가 통째로 생성되지 않아
    // 컴포넌트가 스타일 없이 나온다.
    "./node_modules/@freeive/anti-card/dist/**/*.{js,mjs,cjs}",
  ],
  theme: {
    extend: {
      fontFamily: {
        sans: ["var(--font-sans)"],
        mono: ["var(--font-mono)"],
      },
      colors: {
        bg: "var(--bg)",
        "bg-subtle": "var(--bg-subtle)",
        surface: "var(--surface)",
        border: "var(--border)",
        "border-strong": "var(--border-strong)",
        text: "var(--text)",
        "text-muted": "var(--text-muted)",
        "text-subtle": "var(--text-subtle)",
        accent: "var(--accent)",
        "accent-bg": "var(--accent-bg)",
        "accent-border": "var(--accent-border)",
        danger: "var(--danger)",
        // 오버레이는 배경을 고를 수 없어 테마를 따라가지 않는다 — tokens.css 참고.
        "overlay-surface": "var(--overlay-surface)",
        "overlay-border": "var(--overlay-border)",
        "overlay-text": "var(--overlay-text)",
        "overlay-text-muted": "var(--overlay-text-muted)",
        "overlay-accent": "var(--overlay-accent)",
      },
      borderRadius: {
        sm: "var(--radius-sm)",
        DEFAULT: "var(--radius)",
        lg: "var(--radius-lg)",
      },
      transitionDuration: {
        instant: "75ms",
        fast: "150ms",
        slow: "300ms",
        slower: "500ms",
      },
      transitionTimingFunction: {
        enter: "cubic-bezier(0, 0, 0.2, 1)",
        exit: "cubic-bezier(0.4, 0, 1, 1)",
      },
    },
  },
  darkMode: ["variant", [":root[data-theme='dark'] &", "@media (prefers-color-scheme: dark) { :root:not([data-theme='light']) & }"]],
  plugins: [],
} satisfies Config;
