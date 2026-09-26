// Painted mountain scenery, drawn in code so the launcher ships no large images.
// Scenery.mount(canvas, opts) paints a scene and keeps its mist drifting.
// If the server or the art folder later provides real key art, it sits on top.
(() => {
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;

  function rng(seed) {
    let s = seed >>> 0 || 1;
    return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  }

  // Midpoint displacement: a jagged ridge line, values roughly 0..1.
  function ridge(r, levels, rough, peakiness) {
    const n = (1 << levels) + 1, h = new Float32Array(n);
    h[0] = r(); h[n - 1] = r();
    let step = n - 1, amp = 1;
    while (step > 1) {
      const half = step >> 1;
      for (let i = half; i < n; i += step) h[i] = (h[i - half] + h[i + half]) / 2 + (r() - 0.5) * amp;
      step = half; amp *= rough;
    }
    let lo = Infinity, hi = -Infinity;
    for (const v of h) { lo = Math.min(lo, v); hi = Math.max(hi, v); }
    return Array.from(h, v => Math.pow((v - lo) / (hi - lo || 1), peakiness));
  }

  // Ridge with a few dominant peaks plus jagged detail, values 0..1.
  function peaks(r, n, count, sharp) {
    const detail = ridge(r, Math.log2(n - 1), 0.7, 1);
    const ps = Array.from({ length: count }, () => ({ x: r(), h: 0.45 + r() * 0.55, w: 0.08 + r() * 0.16 }));
    const out = new Array(n);
    for (let i = 0; i < n; i++) {
      const x = i / (n - 1);
      let env = 0;
      for (const p of ps) env = Math.max(env, p.h * Math.pow(Math.max(0, 1 - Math.abs(x - p.x) / p.w), sharp));
      out[i] = Math.min(1, env * 0.72 + detail[i] * 0.38 * (0.4 + env));
    }
    return out;
  }

  const mix = (a, b, k) => a.map((v, i) => Math.round(v + (b[i] - v) * k));
  const rgb = (c, a = 1) => `rgba(${c[0]},${c[1]},${c[2]},${a})`;

  function paint(ctx, W, H, o) {
    const r = rng(o.seed);
    const skyTop = [20, 28, 38], skyMid = [58, 72, 88], haze = [150, 164, 176];

    // sky
    const g = ctx.createLinearGradient(0, 0, 0, H * 0.75);
    g.addColorStop(0, rgb(skyTop)); g.addColorStop(0.55, rgb(skyMid)); g.addColorStop(1, rgb(haze));
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    const sx = W * (o.sunX ?? 0.6), sy = H * 0.22;
    const sun = ctx.createRadialGradient(sx, sy, 0, sx, sy, W * 0.45);
    sun.addColorStop(0, 'rgba(226,234,240,0.55)'); sun.addColorStop(0.4, 'rgba(190,206,220,0.18)'); sun.addColorStop(1, 'rgba(190,206,220,0)');
    ctx.fillStyle = sun; ctx.fillRect(0, 0, W, H);

    // clouds: soft overlapping blobs
    for (let i = 0; i < 60; i++) {
      const x = r() * W, y = r() * H * 0.4, rad = (0.05 + r() * 0.12) * W;
      const c = ctx.createRadialGradient(x, y, 0, x, y, rad);
      const a = 0.05 + r() * 0.08, tone = r() < 0.4 ? '210,220,228' : '40,52,66';
      c.addColorStop(0, `rgba(${tone},${a})`); c.addColorStop(1, `rgba(${tone},0)`);
      ctx.fillStyle = c; ctx.fillRect(x - rad, y - rad, rad * 2, rad * 2);
    }

    // mountain layers, far to near
    const layers = o.layers ?? 5;
    for (let L = 0; L < layers; L++) {
      const k = L / (layers - 1);                     // 0 far .. 1 near
      const base = H * (0.52 + k * 0.30);
      const height = H * (0.62 - k * 0.34) * (o.tall ?? 1);
      const line = k > 0.8 ? ridge(r, 8, 0.55, 1.1) : peaks(r, 257, 3 + Math.floor(r() * 3), 1.15 - k * 0.3);
      const rock = mix([112, 128, 144], [16, 22, 30], Math.pow(k, 0.8));
      const snow = mix([232, 238, 243], [120, 134, 148], Math.pow(k, 1.1));
      const pts = line.map((v, i) => [i / (line.length - 1) * W, base - v * height]);

      ctx.save();
      ctx.beginPath(); ctx.moveTo(0, H);
      for (const [x, y] of pts) ctx.lineTo(x, y);
      ctx.lineTo(W, H); ctx.closePath();
      const fill = ctx.createLinearGradient(0, base - height, 0, base);
      fill.addColorStop(0, rgb(snow)); fill.addColorStop(0.35 + k * 0.2, rgb(mix(snow, rock, 0.75))); fill.addColorStop(1, rgb(rock));
      ctx.fillStyle = fill; ctx.fill();
      ctx.clip();
      // gullies: light snow and dark rock lines running down the slopes
      if (k < 0.8) {
        const gullies = Math.round(W / 14);
        for (let s = 0; s < gullies; s++) {
          const i = 1 + Math.floor(r() * (pts.length - 2)), [x, y] = pts[i];
          const slope = pts[i + 1][1] - pts[i - 1][1];        // >0: ground falls to the right
          const len = height * (0.06 + r() * 0.3);
          const dx = Math.sign(slope || 1) * len * (0.25 + r() * 0.35);
          ctx.strokeStyle = r() < 0.5 ? rgb(snow, 0.18 + r() * 0.25) : rgb(mix(rock, [0, 0, 0], 0.35), 0.25 + r() * 0.3);
          ctx.lineWidth = 0.5 + r() * 1.3;
          ctx.beginPath(); ctx.moveTo(x, y + 1);
          ctx.quadraticCurveTo(x + dx * 0.3, y + len * 0.5, x + dx, y + len); ctx.stroke();
        }
        // shade the side of each peak facing away from the light
        const shade = ctx.createLinearGradient(0, base - height, 0, base);
        shade.addColorStop(0, 'rgba(10,16,24,0)'); shade.addColorStop(1, `rgba(10,16,24,${0.25 * (1 - k)})`);
        ctx.fillStyle = shade; ctx.fillRect(0, base - height, W, height);
      }
      // lit side: brighten slopes facing the sun
      const lit = ctx.createLinearGradient(sx - W * 0.4, 0, sx + W * 0.4, 0);
      lit.addColorStop(0, 'rgba(255,255,255,0)'); lit.addColorStop(0.5, `rgba(235,242,248,${0.10 * (1 - k)})`); lit.addColorStop(1, 'rgba(255,255,255,0)');
      ctx.fillStyle = lit; ctx.fillRect(0, 0, W, H);
      ctx.restore();

      if (L >= layers - 2) forest(ctx, pts, rock, H, r, k, W, o.castle && L === layers - 2 ? W * o.castle : null);
      if (o.castle && L === layers - 2) castle(ctx, W * o.castle, pts, rock, H, r);

      // mist between layers
      const fog = ctx.createLinearGradient(0, base - height * 0.15, 0, base + H * 0.04);
      fog.addColorStop(0, rgb(haze, 0)); fog.addColorStop(0.7, rgb(haze, 0.2 * (1 - k))); fog.addColorStop(1, rgb(haze, 0));
      ctx.fillStyle = fog; ctx.fillRect(0, base - height, W, height + H * 0.1);
    }
  }

  function castle(ctx, cx, pts, rock, H, r) {
    const i = Math.round(cx / (pts[pts.length - 1][0] || 1) * (pts.length - 1));
    const y = pts[Math.max(0, Math.min(pts.length - 1, i))][1] + H * 0.02, u = H * 0.012;
    ctx.fillStyle = rgb(mix(rock, [10, 14, 20], 0.35));
    ctx.beginPath(); ctx.moveTo(cx - 16 * u, H); ctx.lineTo(cx - 14 * u, y + 2 * u); ctx.lineTo(cx - 9 * u, y); ctx.lineTo(cx + 12 * u, y);
    ctx.lineTo(cx + 18 * u, y + 4 * u); ctx.lineTo(cx + 22 * u, H); ctx.fill();
    const blocks = [[-9, 7, 5], [-4, 11, 8], [4, 9, 6], [10, 6, 5], [-1, 17, 3]];   // x, height, width in units
    ctx.fillRect(cx - 12 * u, y - 5 * u, 26 * u, 6 * u);
    for (const [bx, bh, bw] of blocks) {
      const x = cx + bx * u;
      ctx.fillRect(x, y - bh * u, bw * u, bh * u);
      ctx.beginPath(); ctx.moveTo(x - 0.4 * u, y - bh * u); ctx.lineTo(x + bw * u / 2, y - (bh + bw * 0.9) * u); ctx.lineTo(x + (bw + 0.4) * u, y - bh * u); ctx.fill();
    }
    ctx.fillStyle = 'rgba(255,214,150,0.55)';
    for (let w = 0; w < 7; w++) ctx.fillRect(cx + (r() * 22 - 11) * u, y - (2 + r() * 8) * u, 0.6 * u, 0.9 * u);
  }

  function forest(ctx, pts, rock, H, r, k, W, clearX) {
    ctx.fillStyle = rgb(mix(rock, [6, 9, 13], 0.5));
    const count = Math.round(W / (k > 0.9 ? 9 : 14));
    for (let t = 0; t < count; t++) {
      const i = Math.floor(r() * pts.length), [x, y] = pts[i];
      if (clearX != null && Math.abs(x - clearX) < H * 0.3) continue;
      const h = H * (0.03 + r() * 0.05) * (0.6 + k), w = h * 0.34;
      const yy = y + h * 0.35 + r() * H * 0.05;
      ctx.beginPath();
      for (let tier = 0; tier < 4; tier++) {
        const ty = yy - h * (tier / 4), tw = w * (1 - tier / 5);
        ctx.moveTo(x - tw, ty); ctx.lineTo(x, ty - h * 0.42); ctx.lineTo(x + tw, ty);
      }
      ctx.fill();
      ctx.fillRect(x - w * 0.08, yy, w * 0.16, h * 0.12);
    }
  }

  function mount(canvas, o = {}) {
    const ctx = canvas.getContext('2d');
    const base = document.createElement('canvas');
    let W = 0, H = 0, dpr = 1, blobs = [], raf = 0, last = 0;
    const r = rng(o.seed * 7 + 3);

    function build() {
      const b = canvas.getBoundingClientRect();
      if (!b.width || !b.height) return;
      dpr = Math.min(devicePixelRatio || 1, 2); W = b.width; H = b.height;
      for (const c of [canvas, base]) { c.width = Math.round(W * dpr); c.height = Math.round(H * dpr); }
      const bctx = base.getContext('2d'); bctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      paint(bctx, W, H, o);
      blobs = Array.from({ length: o.mist === false ? 0 : 9 }, () => ({ x: r() * W, y: H * (0.45 + r() * 0.45), rx: W * (0.2 + r() * 0.3), ry: H * (0.05 + r() * 0.08), v: 4 + r() * 10, a: 0.05 + r() * 0.07 }));
      frame(performance.now(), true);
    }

    function frame(now, force) {
      if (!force && now - last < 33) { raf = requestAnimationFrame(frame); return; }
      const dt = Math.min(0.1, (now - last) / 1000); last = now;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.drawImage(base, 0, 0);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      for (const b of blobs) {
        if (!reduce) b.x += b.v * dt;
        if (b.x - b.rx > W) b.x = -b.rx;
        ctx.save(); ctx.translate(b.x, b.y); ctx.scale(1, b.ry / b.rx);
        const g = ctx.createRadialGradient(0, 0, 0, 0, 0, b.rx);
        g.addColorStop(0, `rgba(200,212,222,${b.a})`); g.addColorStop(1, 'rgba(200,212,222,0)');
        ctx.fillStyle = g; ctx.fillRect(-b.rx, -b.rx, b.rx * 2, b.rx * 2); ctx.restore();
      }
      if (!reduce && blobs.length && !document.hidden) raf = requestAnimationFrame(frame);
    }

    new ResizeObserver(() => { cancelAnimationFrame(raf); build(); }).observe(canvas);
    document.addEventListener('visibilitychange', () => { if (!document.hidden && !reduce) { cancelAnimationFrame(raf); raf = requestAnimationFrame(frame); } });
  }

  window.Scenery = { mount };
})();
