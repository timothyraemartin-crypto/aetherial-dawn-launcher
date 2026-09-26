// Ambient motion over the background art. Two canvases:
//  - #lights sits inside .scene with the backdrop, so it drifts with the image.
//    It carries the details pinned to the art: flickering castle windows,
//    flowing waterfalls with spray, glints on the lake, a breathing glow on the
//    rune stone, the emblem star and the moons.
//  - #ambient floats above: twinkling stars, a rare shooting star, drifting
//    mist and falling snow.
// The CSS adds a slow camera drift and aurora glow. Everything runs at about
// 30 fps and pauses while the launcher is minimised or in the background. It
// stays off when Windows asks for reduced motion or the player turns it off in
// Settings, and switches itself off if the PC can't keep up.
(() => {
  const canvas = document.getElementById('ambient');
  const ctx = canvas.getContext('2d');
  const lcan = document.getElementById('lights');
  const lctx = lcan.getContext('2d');
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const KEY = 'ad.animatedBackground';
  let on = true;
  try { on = localStorage.getItem(KEY) !== 'off'; } catch {}

  let W = 0, H = 0, stars = [], mist = [], snow = [], raf = 0, last = 0, t = 0, shoot = null, nextShoot = 8;
  let focused = true, slow = false, probe = { n: 0, since: 0 };
  const rnd = (a, b) => a + Math.random() * (b - a);

  // ---- points on art/background.jpg (1920x1081), found by scanning the art ----
  const IW = 1920, IH = 1081;
  const WINDOWS = [[1681,509,22],[1607,516,117],[1578,522,110],[1683,529,28],[1537,529,5],[1701,538,7],[1647,560,26],
    [1725,560,13],[1757,568,5],[1800,585,44],[1825,585,8],[1281,632,2],[1502,645,9],[1579,649,43],[1457,647,16],
    [1422,650,23],[1552,649,4],[1435,652,25],[1379,653,12],[1367,654,15],[1243,658,8],[1621,760,13],[1642,761,46],[1612,774,24]];
  const FALLS = [[1345,690,80,122],[1698,690,42,82],[1150,688,18,52],[1565,726,10,26]];   // x, y, w, h
  const LAKE = [1020,838,620,62], REFLECT = [1362,850,48,105];
  const GLOWS = [                                                                         // x, y, radius, rgb, period
    { x: 173, y: 588, r: 95, c: '150,205,255', p: 5.5, lo: 0.1, hi: 0.4 },               // rune stone sigil
    { x: 250, y: 705, r: 34, c: '255,200,120', p: 4.2, lo: 0.15, hi: 0.45 },             // lantern by the stone
    { x: 960, y: 128, r: 80, c: '220,235,255', p: 3.6, lo: 0.15, hi: 0.5, flare: 1 },     // emblem star
    { x: 1291, y: 153, r: 170, c: '170,200,255', p: 9, lo: 0.05, hi: 0.14 },             // big moon
    { x: 469, y: 156, r: 75, c: '170,200,255', p: 7, lo: 0.05, hi: 0.14 },               // small moon
  ];
  let map = { s: 1, ox: 0, oy: 0 }, lamps = [], streaks = [], glints = [];
  const X = x => map.ox + x * map.s, Y = y => map.oy + y * map.s;

  // Soft round glow, drawn once and stamped with drawImage (much cheaper than a gradient per light).
  function sprite(rgb) {
    const c = document.createElement('canvas'); c.width = c.height = 64;
    const g = c.getContext('2d'), grad = g.createRadialGradient(32, 32, 0, 32, 32, 32);
    grad.addColorStop(0, `rgba(${rgb},1)`); grad.addColorStop(0.25, `rgba(${rgb},.45)`); grad.addColorStop(1, `rgba(${rgb},0)`);
    g.fillStyle = grad; g.fillRect(0, 0, 64, 64); return c;
  }
  const warm = sprite('255,176,92'), core = sprite('255,226,170'), spray = sprite('215,230,245');
  const tint = {};
  const glowSprite = rgb => tint[rgb] || (tint[rgb] = sprite(rgb));
  function stamp(g, img, x, y, r, a) { if (a <= 0.003) return; g.globalAlpha = Math.min(1, a); g.drawImage(img, x - r, y - r, r * 2, r * 2); }

  function build() {
    W = canvas.clientWidth; H = canvas.clientHeight;
    canvas.width = Math.round(W); canvas.height = Math.round(H);
    lcan.width = Math.round(W); lcan.height = Math.round(H);
    // Same maths as the backdrop's object-fit: cover; object-position: 30% 0.
    const s = Math.max(W / IW, H / IH);
    map = { s, ox: (W - IW * s) * 0.3, oy: 0 };

    stars = Array.from({ length: Math.round(W * H / 9000) }, () => ({ x: rnd(0, W), y: rnd(0, H * 0.38), r: rnd(0.4, 1.3), p: rnd(0, 6.28), s: rnd(0.6, 2) }));
    mist = Array.from({ length: 7 }, () => ({ x: rnd(-W * 0.3, W), y: rnd(H * 0.42, H * 0.72), rx: rnd(W * 0.18, W * 0.34), ry: rnd(H * 0.04, H * 0.08), v: rnd(4, 12), a: rnd(0.05, 0.1) }));
    snow = Array.from({ length: Math.round(W * H / 11000) }, () => flake(true));

    // Each window gets its own candle rhythm, and now and then gutters out for a moment.
    lamps = WINDOWS.map(([x, y, n]) => ({ x, y, r: 7 + Math.sqrt(n) * 1.8, a1: rnd(1.2, 2.6), a2: rnd(6, 12), p1: rnd(0, 6.28), p2: rnd(0, 6.28), dip: 0, next: rnd(3, 30) }));
    streaks = [];
    for (const [fx, fy, fw, fh] of FALLS) for (let i = 0; i < Math.max(4, Math.round(fw * fh / 170)); i++) streaks.push(streak(fx, fy, fw, fh, true));
    glints = Array.from({ length: 34 }, () => glint(true));
  }
  function flake(anywhere) {
    const z = Math.random();                    // 0 far .. 1 near
    return { x: rnd(0, W), y: anywhere ? rnd(0, H) : rnd(-20, -4), z, r: 0.5 + z * 1.8, vy: 12 + z * 38, sway: rnd(0, 6.28), a: 0.25 + z * 0.55 };
  }
  function streak(fx, fy, fw, fh, anywhere) {
    return { fx, fy, fw, fh, x: fx + rnd(0, fw), k: anywhere ? Math.random() : rnd(-0.3, 0), len: rnd(10, 30), v: rnd(0.5, 0.95), a: rnd(0.22, 0.5) };
  }
  function glint(anywhere) {
    const col = Math.random() < 0.35, [gx, gy, gw, gh] = col ? REFLECT : LAKE;
    const life = rnd(0.6, 1.8);
    return { x: gx + rnd(0, gw), y: gy + rnd(0, gh), len: rnd(3, col ? 12 : 8), life, age: anywhere ? rnd(0, life) : 0, a: rnd(0.25, col ? 0.7 : 0.5) };
  }

  function drawLights(dt) {
    const g = lctx, s = map.s;
    g.clearRect(0, 0, W, H);
    g.globalCompositeOperation = 'lighter';

    for (const L of lamps) {
      if ((L.next -= dt) <= 0) { L.dip = 1; L.next = rnd(6, 32); }
      L.dip = Math.max(0, L.dip - dt * 1.6);
      const f = (0.72 + 0.18 * Math.sin(t * L.a1 + L.p1) + 0.1 * Math.sin(t * L.a2 + L.p2)) * (1 - 0.7 * Math.sin(Math.PI * L.dip));
      const x = X(L.x), y = Y(L.y);
      stamp(g, warm, x, y, L.r * s * 2.2, 0.28 * f);
      stamp(g, core, x, y, L.r * s * 0.7, 0.55 * f);
    }

    g.lineCap = 'round';
    for (let i = 0; i < streaks.length; i++) {
      const q = streaks[i];
      q.k += q.v * dt;
      if (q.k > 1.05) { streaks[i] = streak(q.fx, q.fy, q.fw, q.fh, false); continue; }
      if (q.k < 0) continue;
      const y = q.fy + q.k * q.fh, a = q.a * Math.sin(Math.PI * Math.min(1, q.k));
      g.globalAlpha = a; g.strokeStyle = 'rgb(225,238,255)'; g.lineWidth = Math.max(1, 1.5 * s);
      g.beginPath(); g.moveTo(X(q.x), Y(y - q.len)); g.lineTo(X(q.x), Y(y)); g.stroke();
    }
    for (const [fx, fy, fw, fh] of FALLS) {                              // spray where each fall lands
      const a = 0.1 + 0.05 * Math.sin(t * 1.7 + fx);
      stamp(g, spray, X(fx + fw / 2), Y(fy + fh), fw * s * 0.9, a);
    }

    for (let i = 0; i < glints.length; i++) {
      const G = glints[i];
      if ((G.age += dt) > G.life) { glints[i] = glint(false); continue; }
      g.globalAlpha = G.a * Math.sin(Math.PI * G.age / G.life);
      g.strokeStyle = 'rgb(205,228,255)'; g.lineWidth = Math.max(0.7, s);
      g.beginPath(); g.moveTo(X(G.x - G.len / 2), Y(G.y)); g.lineTo(X(G.x + G.len / 2), Y(G.y)); g.stroke();
    }

    for (const o of GLOWS) {
      const k = 0.5 + 0.5 * Math.sin(t * 6.283 / o.p), a = o.lo + (o.hi - o.lo) * k, x = X(o.x), y = Y(o.y), r = o.r * s;
      stamp(g, glowSprite(o.c), x, y, r, a);
      if (o.flare) {                                                     // four-point glint on the star
        g.globalAlpha = a * 0.9;
        for (const [dx, dy, l] of [[1, 0, 1.3], [0, 1, 1]]) {
          const grad = g.createLinearGradient(x - dx * r * l, y - dy * r * l, x + dx * r * l, y + dy * r * l);
          grad.addColorStop(0, 'rgba(230,240,255,0)'); grad.addColorStop(0.5, 'rgba(230,240,255,.9)'); grad.addColorStop(1, 'rgba(230,240,255,0)');
          g.strokeStyle = grad; g.lineWidth = Math.max(1, 1.4 * s);
          g.beginPath(); g.moveTo(x - dx * r * l, y - dy * r * l); g.lineTo(x + dx * r * l, y + dy * r * l); g.stroke();
        }
      }
    }
    g.globalAlpha = 1; g.globalCompositeOperation = 'source-over';
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
    drawLights(dt);
    ctx.clearRect(0, 0, W, H);

    for (const s of stars) {
      const a = 0.25 + 0.55 * (0.5 + 0.5 * Math.sin(s.p + t * s.s));
      ctx.fillStyle = `rgba(225,238,255,${a * (1 - s.y / (H * 0.45))})`;
      ctx.beginPath(); ctx.arc(s.x, s.y, s.r, 0, 6.29); ctx.fill();
    }
    if ((nextShoot -= dt) <= 0 && !shoot) {
      shoot = { x: rnd(W * 0.3, W * 0.95), y: rnd(10, H * 0.22), vx: -rnd(380, 560), vy: rnd(120, 220), age: 0, life: rnd(0.6, 1) };
      nextShoot = rnd(12, 28);
    }
    if (shoot) {
      const S = shoot; S.age += dt; S.x += S.vx * dt; S.y += S.vy * dt;
      if (S.age > S.life) shoot = null;
      else {
        const a = Math.sin(Math.PI * S.age / S.life), tx = S.x - S.vx * 0.18, ty = S.y - S.vy * 0.18;
        const grad = ctx.createLinearGradient(S.x, S.y, tx, ty);
        grad.addColorStop(0, `rgba(235,245,255,${0.9 * a})`); grad.addColorStop(1, 'rgba(235,245,255,0)');
        ctx.strokeStyle = grad; ctx.lineWidth = 1.4; ctx.lineCap = 'round';
        ctx.beginPath(); ctx.moveTo(S.x, S.y); ctx.lineTo(tx, ty); ctx.stroke();
      }
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
    canvas.hidden = lcan.hidden = !allowed;
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
