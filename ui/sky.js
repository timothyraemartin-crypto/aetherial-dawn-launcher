// Live pre-dawn sky behind the launcher. Sky.setDawn(0..1) moves toward sunrise.
(() => {

  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const cv = document.getElementById('sky'), ctx = cv.getContext('2d');
  let W = 0, H = 0, dpr = 1, stars = [], ridges = [], dawn = 0.15, dawnTarget = 0.15, t0 = performance.now();

  function rand(seed) { let s = seed; return () => (s = (s * 16807) % 2147483647) / 2147483647; }
  function build() {
    const r = cv.getBoundingClientRect(); dpr = Math.min(devicePixelRatio || 1, 2);
    W = r.width; H = r.height; cv.width = W * dpr; cv.height = H * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const R = rand(7); stars = Array.from({ length: Math.round(W * H / 2600) }, () => ({ x: R() * W, y: R() * H * 0.62, r: R() * 1.1 + 0.2, p: R() * 6.28 }));
    ridges = [0.50, 0.60, 0.70, 0.80].map((base, i) => {
      const Rr = rand(31 + i * 13), pts = [], n = 90; let y = 0, v = 0;
      for (let k = 0; k <= n; k++) { v += (Rr() - 0.5) * (0.06 - i * 0.008); v *= 0.86; y += v; y *= 0.97; pts.push(y); }
      const m = Math.max(...pts.map(Math.abs)) || 1;
      return { base, pts: pts.map(p => Math.abs(p) / m), amp: H * (0.20 - i * 0.035) };
    });
  }
  function mix(a, b, k) { return a.map((v, i) => Math.round(v + (b[i] - v) * k)); }
  function draw(now) {
    const t = (now - t0) / 1000; dawn += (dawnTarget - dawn) * 0.02;
    const top = mix([8, 11, 22], [22, 30, 58], dawn), mid = mix([20, 26, 48], [92, 78, 96], dawn), low = mix([58, 48, 60], [237, 170, 104], dawn);
    const g = ctx.createLinearGradient(0, 0, 0, H * 0.78);
    g.addColorStop(0, `rgb(${top})`); g.addColorStop(0.55, `rgb(${mid})`); g.addColorStop(1, `rgb(${low})`);
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    // sun glow behind the peaks
    const sx = W * 0.64, sy = H * (0.70 - dawn * 0.06);
    const sg = ctx.createRadialGradient(sx, sy, 0, sx, sy, W * (0.25 + dawn * 0.25));
    sg.addColorStop(0, `rgba(255,214,150,${0.25 + dawn * 0.55})`); sg.addColorStop(1, 'rgba(255,214,150,0)');
    ctx.fillStyle = sg; ctx.fillRect(0, 0, W, H);
    // aurora ribbon
    ctx.save(); ctx.globalCompositeOperation = 'screen';
    for (let b = 0; b < 3; b++) {
      ctx.beginPath();
      for (let x = 0; x <= W; x += 8) {
        const y = H * (0.18 + b * 0.05) + Math.sin(x * 0.004 + t * 0.15 + b) * H * 0.05 + Math.sin(x * 0.011 - t * 0.1) * H * 0.015;
        x ? ctx.lineTo(x, y) : ctx.moveTo(x, y);
      }
      ctx.strokeStyle = `rgba(116,201,188,${(0.06 - b * 0.015) * (1 - dawn * 0.8)})`; ctx.lineWidth = H * 0.06; ctx.stroke();
    }
    ctx.restore();
    // stars fade with dawn
    for (const s of stars) {
      const a = (0.45 + 0.45 * Math.sin(s.p + t * 0.8)) * (1 - dawn) * (1 - s.y / (H * 0.7));
      if (a <= 0.02) continue; ctx.fillStyle = `rgba(228,234,242,${a})`; ctx.beginPath(); ctx.arc(s.x, s.y, s.r, 0, 6.29); ctx.fill();
    }
    // mountain ridges, far to near
    ridges.forEach((rg, i) => {
      const shade = mix(mix([46, 52, 82], [120, 96, 110], dawn), [8, 10, 18], i / 3.2);
      ctx.fillStyle = `rgb(${shade})`; ctx.beginPath(); ctx.moveTo(0, H);
      rg.pts.forEach((p, k) => ctx.lineTo(k / (rg.pts.length - 1) * W, H * rg.base - p * rg.amp - (i === 0 ? Math.max(0, 1 - Math.abs(k / 90 - 0.64) * 4) * H * 0.10 : 0)));
      ctx.lineTo(W, H); ctx.closePath(); ctx.fill();
    });
    if (!reduce || Math.abs(dawnTarget - dawn) > 0.002) requestAnimationFrame(draw);
  }
  build(); requestAnimationFrame(draw);
  new ResizeObserver(() => { build(); if (reduce) requestAnimationFrame(draw); }).observe(cv);
  const wake = () => { if (reduce) requestAnimationFrame(draw); };
  window.Sky = { setDawn(v) { dawnTarget = v; wake(); } };
})();
