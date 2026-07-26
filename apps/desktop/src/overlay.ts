import { onOverlayState, onSnapshot } from "./snapshot";

// Preflight가 먼저, 토큰이 나중.
import "./styles/index.css";
import "./styles/tokens.css";

// React를 쓰지 않는다. 게임 위에 상시 떠 있는 창이라 가벼울수록 좋고,
// 내용도 숫자 몇 개뿐이라 React가 주는 게 없다.

const nf = new Intl.NumberFormat("ko-KR");
const $ = (id: string) => document.getElementById(id)!;

onSnapshot((s) => {
  $("count").textContent = s.paused ? "일시정지" : nf.format(s.today);
  $("total").textContent = nf.format(s.total);
  $("best").textContent = nf.format(s.best_day);
  $("session").textContent = nf.format(s.session);
  $("cb").textContent = `${nf.format(s.max_cb_us)} µs`;
  $("count").classList.toggle("text-overlay-accent", s.paused);
});

onOverlayState((o) => {
  document.body.dataset.variant = o.variant;
});
