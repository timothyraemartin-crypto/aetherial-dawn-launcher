// Ambient motion over the background art: twinkling stars, drifting mist and
// falling snow on one canvas, plus a slow camera drift and aurora glow in CSS.
// Runs at about 30 fps and pauses while the launcher is minimised or in the
// background. It stays off when Windows asks for reduced motion or the player
// turns it off in Settings, and switches itself off if the PC can't keep up.
(() => {
  const canvas = document.getElementById('ambient');
  const ctx = canvas.getContext('2d');
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const KEY = 'ad.animatedBackground';
  let on = true;
  try { on = localStorage.getItem(KEY) !== 'off'; } catch {}

  let W = 0, H = 0, dpr = 1, stars = [], mist = [], snow = [], raf = 0, last = 0, t = 0;
  let focused = true, slow = false, probe = { n: 0, since: 0 };
  const rnd = (a, b) => a + Math.random() * (b - a);

  function build() {
    const r = canvas.getBoundingClientRect();
    dpr = 1;
    W = r.width; H = r.height;
    canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    stars = Array.from({ length: Math.round(W * H / 9000) }, () => ({ x: rnd(0, W), y: rnd(0, H * 0.38), r: rnd(0.4, 1.3), p: rnd(0, 6.28), s: rnd(0.6, 2) }));
    mist = Array.from({ length: 7 }, () => ({ x: rnd(-W * 0.3, W), y: rnd(H * 0.42, H * 0.72), rx: rnd(W * 0.18, W * 0.34), ry: rnd(H * 0.04, H * 0.08), v: rnd(4, 12), a: rnd(0.05, 0.1) }));
    snow = Array.from({ length: Math.round(W * H / 11000) }, () => flake(true));
  }
  function flake(anywhere) {
    const z = Math.random();                    // 0 far .. 1 near
    return { x: rnd(0, W), y: anywhere ? rnd(0, H) : rnd(-20, -4), z, r: 0.5 + z * 1.8, vy: 12 + z * 38, sway: rnd(0, 6.28), a: 0.25 + z * 0.55 };
  }

  function frame(now) {
    raf = requestAnimationFrame(frame);
    if (now - last < 33) return;
    const gap = now - last;
    const dt = Math.min(0.1, gap / 1000); last = now; t += dt;
    // Watch the first seconds: if frames arrive slower than ~15 fps, stop.
    if (probe.n < 90) {
      if (!probe.since) probe.since = now;
      if (++probe.n === 90 && (now - probe.since) / 89 > 66) { slow = true; start(); return; }
    }
    ctx.clearRect(0, 0, W, H);

    for (const s of stars) {
      const a = 0.25 + 0.55 * (0.5 + 0.5 * Math.sin(s.p + t * s.s));
      ctx.fillStyle = `rgba(225,238,255,${a * (1 - s.y / (H * 0.45))})`;
      ctx.beginPath(); ctx.arc(s.x, s.y, s.r, 0, 6.29); ctx.fill();
    }
    for (const m of mist) {
      m.x += m.v * dt;
      if (m.x - m.rx > W) { m.x = -m.rx; m.y = rnd(H * 0.42, H * 0.72); }
      ctx.save(); ctx.translate(m.x, m.y); ctx.scale(1, m.ry / m.rx);
      const g = ctx.createRadialGradient(0, 0, 0, 0, 0, m.rx);
      g.addColorStop(0, `rgba(200,215,230,${m.a})`); g.addColorStop(1, 'rgba(200,215,230,0)');
      ctx.fillStyle = g; ctx.fillRect(-m.rx, -m.rx, m.rx * 2, m.rx * 2); ctx.restore();
    }
    for (let i = 0; i < snow.length; i++) {
      const f = snow[i];
      f.y += f.vy * dt; f.x += (Math.sin(t * 0.8 + f.sway) * 8 - 6) * dt * (0.4 + f.z);
      if (f.y > H + 4 || f.x < -4) snow[i] = flake(false);
      ctx.fillStyle = `rgba(235,242,250,${f.a})`;
      ctx.beginPath(); ctx.arc(f.x, f.y, f.r, 0, 6.29); ctx.fill();
    }
  }

  function start() {
    cancelAnimationFrame(raf);
    const allowed = on && !reduce && !slow;
    const run = allowed && !document.hidden && focused;
    document.documentElement.classList.toggle('animated', allowed);
    document.documentElement.classList.toggle('paused', !run);
    canvas.hidden = !allowed;
    if (run) { if (probe.n < 90) probe = { n: 0, since: 0 }; last = performance.now(); raf = requestAnimationFrame(frame); }
  }

  new ResizeObserver(() => { build(); start(); }).observe(canvas);
  document.addEventListener('visibilitychange', start);
  window.addEventListener('focus', () => { focused = true; start(); });
  window.addEventListener('blur', () => { focused = false; start(); });

  window.Ambient = {
    get enabled() { return on; },
    set(v) { on = v; slow = false; probe = { n: 0, since: 0 }; try { localStorage.setItem(KEY, v ? 'on' : 'off'); } catch {} start(); },
  };
})();
