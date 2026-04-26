/* global React */
const { useEffect, useRef, useState, useMemo } = React;

// ─── ANIMATED SPARKLINE ─────────────────────────────────────────────
function resolveCSSVar(c) {
  if (typeof c !== 'string') return c;
  if (!c.startsWith('var(')) return c;
  const name = c.match(/var\((--[^)]+)\)/)?.[1];
  if (!name) return c;
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || '#2d8c5d';
}

function Sparkline({ data, height = 28, color = '#2d8c5d', fill = true, animate = true }) {
  color = resolveCSSVar(color);
  const ref = useRef(null);
  const dprRef = useRef(window.devicePixelRatio || 1);
  const [phase, setPhase] = useState(0);

  useEffect(() => {
    if (!animate) return;
    let raf;
    const tick = () => {
      setPhase(p => (p + 1) % 1000);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [animate]);

  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const dpr = dprRef.current;
    const w = c.clientWidth, h = c.clientHeight;
    c.width = w * dpr; c.height = h * dpr;
    const ctx = c.getContext('2d');
    ctx.scale(dpr, dpr);
    ctx.clearRect(0,0,w,h);

    if (!data.length) return;
    const min = Math.min(...data);
    const max = Math.max(...data);
    const span = (max - min) || 1;

    const pts = data.map((v, i) => [
      (i / (data.length - 1)) * w,
      h - 2 - ((v - min) / span) * (h - 4),
    ]);

    if (fill) {
      const grad = ctx.createLinearGradient(0,0,0,h);
      grad.addColorStop(0, color);
      grad.addColorStop(1, 'transparent');
      ctx.globalAlpha = 0.18;
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.moveTo(0, h);
      pts.forEach(p => ctx.lineTo(p[0], p[1]));
      ctx.lineTo(w, h);
      ctx.closePath();
      ctx.fill();
      ctx.globalAlpha = 1;
    }

    ctx.strokeStyle = color;
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    pts.forEach((p, i) => i === 0 ? ctx.moveTo(p[0], p[1]) : ctx.lineTo(p[0], p[1]));
    ctx.stroke();

    // animated cursor dot at end
    const last = pts[pts.length - 1];
    if (last) {
      const r = 2.2 + Math.sin(phase * 0.08) * 0.8;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(last[0], last[1], r, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 0.25;
      ctx.beginPath();
      ctx.arc(last[0], last[1], r + 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
    }
  }, [data, phase, color, fill]);

  return <canvas ref={ref} className="spark" style={{ height }} />;
}

// ─── HISTOGRAM (latency distribution) ───────────────────────────────
function Histogram({ buckets, height = 64, color = '#2d8c5d' }) {
  color = resolveCSSVar(color);
  const ref = useRef(null);
  useEffect(() => {
    const c = ref.current; if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, h = c.clientHeight;
    c.width = w * dpr; c.height = h * dpr;
    const ctx = c.getContext('2d');
    ctx.scale(dpr, dpr);
    ctx.clearRect(0,0,w,h);
    const max = Math.max(...buckets) || 1;
    const bw = w / buckets.length;
    buckets.forEach((v, i) => {
      const bh = (v / max) * (h - 4);
      ctx.fillStyle = color;
      ctx.globalAlpha = 0.85;
      ctx.fillRect(i * bw + 0.5, h - bh, bw - 1, bh);
    });
    ctx.globalAlpha = 1;
  }, [buckets, color]);
  return <canvas ref={ref} style={{ width: '100%', height, display: 'block' }} />;
}

// ─── ANIMATED LINE CHART (multi-series, time-axis) ──────────────────
function LineChart({ series, height = 200, yLabel = '', live = true }) {
  const ref = useRef(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => setTick(t => t + 1), 800);
    return () => clearInterval(id);
  }, [live]);

  useEffect(() => {
    const c = ref.current; if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, h = c.clientHeight;
    c.width = w * dpr; c.height = h * dpr;
    const ctx = c.getContext('2d');
    ctx.scale(dpr, dpr);
    ctx.clearRect(0,0,w,h);

    const padL = 38, padR = 12, padT = 10, padB = 22;
    const cw = w - padL - padR, ch = h - padT - padB;

    // grid
    ctx.strokeStyle = 'rgba(255,255,255,0.04)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      ctx.beginPath(); ctx.moveTo(padL, y); ctx.lineTo(padL + cw, y); ctx.stroke();
    }

    const all = series.flatMap(s => s.data);
    const max = Math.max(...all, 1);
    const min = 0;

    // y labels
    ctx.fillStyle = '#5a6373';
    ctx.font = '9.5px JetBrains Mono, monospace';
    ctx.textAlign = 'right';
    for (let i = 0; i <= 4; i++) {
      const v = max - (max / 4) * i;
      ctx.fillText(v.toFixed(0), padL - 6, padT + (ch / 4) * i + 3);
    }

    // x axis
    ctx.fillStyle = '#5a6373';
    ctx.textAlign = 'left';
    const xLabels = ['-60s', '-45s', '-30s', '-15s', 'now'];
    xLabels.forEach((lbl, i) => {
      const x = padL + (cw / 4) * i;
      ctx.fillText(lbl, x - (i === 4 ? 18 : 8), h - 6);
    });

    // each series
    series.forEach(s => {
      const pts = s.data.map((v, i) => [
        padL + (i / (s.data.length - 1)) * cw,
        padT + ch - ((v - min) / (max - min || 1)) * ch,
      ]);

      // fill
      const grad = ctx.createLinearGradient(0, padT, 0, padT + ch);
      grad.addColorStop(0, s.color);
      grad.addColorStop(1, 'transparent');
      ctx.globalAlpha = 0.12;
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.moveTo(pts[0][0], padT + ch);
      pts.forEach(p => ctx.lineTo(p[0], p[1]));
      ctx.lineTo(pts[pts.length-1][0], padT + ch);
      ctx.closePath();
      ctx.fill();
      ctx.globalAlpha = 1;

      // line
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      pts.forEach((p, i) => i === 0 ? ctx.moveTo(p[0], p[1]) : ctx.lineTo(p[0], p[1]));
      ctx.stroke();

      // pulsing endpoint
      const last = pts[pts.length - 1];
      const t = (Date.now() % 1600) / 1600;
      const r = 2 + Math.sin(t * Math.PI * 2) * 1.5;
      ctx.fillStyle = s.color;
      ctx.beginPath(); ctx.arc(last[0], last[1], r, 0, Math.PI * 2); ctx.fill();
      ctx.globalAlpha = 0.25;
      ctx.beginPath(); ctx.arc(last[0], last[1], r + 5, 0, Math.PI * 2); ctx.fill();
      ctx.globalAlpha = 1;
    });

    // legend
    ctx.font = '10px JetBrains Mono, monospace';
    ctx.textAlign = 'left';
    let lx = padL;
    series.forEach(s => {
      ctx.fillStyle = s.color;
      ctx.fillRect(lx, padT - 6, 8, 2);
      ctx.fillStyle = '#8a92a0';
      ctx.fillText(s.label, lx + 12, padT - 2);
      lx += ctx.measureText(s.label).width + 36;
    });

  }, [series, tick]);

  return <canvas ref={ref} style={{ width: '100%', height, display: 'block' }} />;
}

// ─── ANIMATED VECTOR FIELD (3D-feel, canvas WebGL-style) ────────────
// Ambient hero: rotating point cloud (the "vector substrate" feel)
function VectorField({ height = 180, points = 240, color = '#2d8c5d', speed = 1, perspective = 1, showRing = true, showHalo = false, density = 1 }) {
  const ref = useRef(null);
  useEffect(() => {
    const c = ref.current; if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    let w = c.clientWidth, h = c.clientHeight;
    c.width = w * dpr; c.height = h * dpr;
    const ctx = c.getContext('2d');
    ctx.scale(dpr, dpr);

    // Generate points on a unit sphere (cluster + outliers)
    const N = Math.max(20, Math.round(points * density));
    const pts = [];
    for (let i = 0; i < N; i++) {
      const u = Math.random(), v = Math.random();
      const theta = 2 * Math.PI * u;
      const phi = Math.acos(2 * v - 1);
      const r = 0.6 + Math.random() * 0.35;
      pts.push({
        x: r * Math.sin(phi) * Math.cos(theta),
        y: r * Math.sin(phi) * Math.sin(theta),
        z: r * Math.cos(phi),
        s: Math.random() * 0.7 + 0.3,
      });
    }

    let raf;
    const start = performance.now();
    const draw = () => {
      const t = (performance.now() - start) / 1000 * speed;
      const cx = w / 2, cy = h / 2;
      const scale = Math.min(w, h) * 0.34;
      ctx.fillStyle = 'rgba(10,14,20,0.32)';
      ctx.fillRect(0, 0, w, h);

      const cosA = Math.cos(t * 0.18), sinA = Math.sin(t * 0.18);
      const cosB = Math.cos(t * 0.11), sinB = Math.sin(t * 0.11);

      const proj = pts.map(p => {
        let x = p.x * cosA - p.z * sinA;
        let z = p.x * sinA + p.z * cosA;
        let y = p.y * cosB - z * sinB;
        z = p.y * sinB + z * cosB;
        return { x, y, z, s: p.s };
      }).sort((a, b) => a.z - b.z);

      proj.forEach(p => {
        const persp = 1 / (2 - p.z * perspective);
        const px = cx + p.x * scale * persp;
        const py = cy + p.y * scale * persp;
        const r = (0.6 + p.s * 1.2) * persp;
        const alpha = 0.25 + (p.z + 1) * 0.32;
        ctx.fillStyle = color;
        ctx.globalAlpha = alpha * p.s;
        ctx.beginPath(); ctx.arc(px, py, r, 0, Math.PI * 2); ctx.fill();

        if (showHalo) {
          ctx.globalAlpha = alpha * p.s * 0.18;
          ctx.beginPath(); ctx.arc(px, py, r * 3.2, 0, Math.PI * 2); ctx.fill();
        }
      });
      ctx.globalAlpha = 1;

      if (showRing) {
        const ringR = scale * 1.05;
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.18 + Math.sin(t * 1.2) * 0.08;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.arc(cx, cy, ringR, 0, Math.PI * 2);
        ctx.stroke();
        ctx.globalAlpha = 1;
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [points, color, speed, perspective, showRing, showHalo, density]);

  return <canvas ref={ref} style={{ width: '100%', height, display: 'block' }} />;
}

// ─── RECEIPT PRINT animation (lines drop in from top with mask) ─────
function ReceiptPrint({ children, trigger }) {
  const [printed, setPrinted] = useState(true);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; setPrinted(true); return; }
    setPrinted(false);
    const id = setTimeout(() => setPrinted(true), 50);
    return () => clearTimeout(id);
  }, [trigger]);
  return (
    <div style={{
      maxHeight: printed ? '2400px' : '0px',
      overflow: 'hidden',
      transition: 'max-height 900ms cubic-bezier(.2,.8,.2,1)',
    }}>
      {children}
    </div>
  );
}

// ─── HEX COMPONENT (elided + hover full) ────────────────────────────
function Hex({ value, head = 8, tail = 6, className = '' }) {
  if (!value) return <span className={`mono dim ${className}`}>—</span>;
  return (
    <span className={`hex ${className}`} title={value}>
      <span className="elided">{value.slice(0, head)}<span style={{opacity:0.5}}>…</span>{value.slice(-tail)}</span>
      <span className="full">{value}</span>
    </span>
  );
}

// ─── BAR ────────────────────────────────────────────────────────────
function Bar({ pct, amber = false, width = 60 }) {
  return (
    <div className="bar" style={{ width }}>
      <div className={'bar-fill' + (amber ? ' amber' : '')} style={{ width: Math.max(0, Math.min(100, pct)) + '%' }} />
    </div>
  );
}

window.RuComp = { Sparkline, Histogram, LineChart, VectorField, ReceiptPrint, Hex, Bar };
