// Hand-written SVG charts. Light and dark palettes from one stylesheet; every point has a hover <title>.
const SERIES = [
  ["#2a78d6", "#3987e5"],
  ["#eb6834", "#d95926"],
  ["#1baf7a", "#199e70"],
  ["#eda100", "#c98500"],
  ["#e87ba4", "#d55181"],
  ["#6250d6", "#9085e9"],
];
const W = 760, H = 420, M = { top: 64, right: 150, bottom: 52, left: 72 };
const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function fmt(x) {
  if (x == null || !Number.isFinite(x)) return "n/a";
  const a = Math.abs(x);
  if (a >= 1e9) return `${+(x / 1e9).toPrecision(3)}B`;
  if (a >= 1e6) return `${+(x / 1e6).toPrecision(3)}M`;
  if (a >= 1e4) return `${+(x / 1e3).toPrecision(3)}k`;
  if (a >= 100) return String(Math.round(x));
  return String(+x.toPrecision(3));
}

function style() {
  const light = SERIES.map(([l], i) => `--s${i}:${l};`).join("");
  const dark = SERIES.map(([, d], i) => `--s${i}:${d};`).join("");
  return `<style>
svg{--bg:#fcfcfb;--ink:#0b0b0b;--ink2:#52514e;--grid:#e4e3df;${light}font-family:system-ui,-apple-system,"Segoe UI",sans-serif}
@media (prefers-color-scheme: dark){svg{--bg:#1a1a19;--ink:#f0efec;--ink2:#c3c2b7;--grid:#34332f;${dark}}}
.bg{fill:var(--bg)}.t{fill:var(--ink);font-size:16px;font-weight:600}.st{fill:var(--ink2);font-size:12px}
.ax{fill:var(--ink2);font-size:11px}.g{stroke:var(--grid);stroke-width:1}.lbl{fill:var(--ink);font-size:12px}
.ln{fill:none;stroke-width:2;stroke-linejoin:round;stroke-linecap:round}.hit{fill:transparent}.hit:hover{fill:var(--ink2);fill-opacity:.35}
${SERIES.map((_, i) => `.c${i}{stroke:var(--s${i});fill:var(--s${i})}.c${i}.ln{fill:none}`).join("")}
</style>`;
}

function ticks(lo, hi, n = 5) {
  const step0 = (hi - lo) / n, mag = 10 ** Math.floor(Math.log10(step0 || 1));
  const step = [1, 2, 5, 10].map((k) => k * mag).find((s) => s >= step0) ?? 10 * mag;
  const out = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9 * step; v += step) out.push(+v.toPrecision(12));
  return out;
}

function frame(title, subtitle, body, h = H) {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${h}" width="${W}" height="${h}" role="img" aria-label="${esc(title)}">
${style()}<rect class="bg" width="${W}" height="${h}"/>
<text class="t" x="${M.left}" y="26">${esc(title)}</text><text class="st" x="${M.left}" y="44">${esc(subtitle)}</text>
${body}</svg>\n`;
}

/** series: [{ name, values: number[] }], x is the index (day). */
export function lineChart({ title, subtitle, xLabel, yLabel, series, log = false }) {
  const all = series.flatMap((s) => s.values).filter((v) => Number.isFinite(v) && (!log || v > 0));
  const n = Math.max(...series.map((s) => s.values.length));
  let lo = log ? Math.log10(Math.min(...all)) : Math.min(0, ...all), hi = log ? Math.log10(Math.max(...all)) : Math.max(...all);
  if (log) { lo = Math.floor(lo); hi = Math.ceil(hi); }
  if (hi === lo) hi = lo + 1;
  const pw = W - M.left - M.right, ph = H - M.top - M.bottom;
  const X = (i) => M.left + (n > 1 ? (i / (n - 1)) * pw : 0);
  const Y = (v) => M.top + ph - (((log ? Math.log10(v) : v) - lo) / (hi - lo)) * ph;
  const yt = log ? Array.from({ length: hi - lo + 1 }, (_, k) => 10 ** (lo + k)) : ticks(lo, hi);
  if (!log) hi = Math.max(hi, yt.at(-1));
  let b = yt.map((v) => `<line class="g" x1="${M.left}" x2="${M.left + pw}" y1="${Y(v)}" y2="${Y(v)}"/><text class="ax" x="${M.left - 8}" y="${Y(v) + 4}" text-anchor="end">${fmt(v)}</text>`).join("");
  b += ticks(0, n - 1, 6).map((i) => `<text class="ax" x="${X(i)}" y="${M.top + ph + 18}" text-anchor="middle">${i}</text>`).join("");
  b += `<text class="ax" x="${M.left + pw / 2}" y="${H - 12}" text-anchor="middle">${esc(xLabel)}</text>`;
  b += `<text class="ax" transform="translate(16 ${M.top + ph / 2}) rotate(-90)" text-anchor="middle">${esc(yLabel)}</text>`;
  const ends = [];
  series.forEach((s, k) => {
    const pts = s.values.map((v, i) => (Number.isFinite(v) && (!log || v > 0) ? [X(i), Y(v), v, i] : null)).filter(Boolean);
    b += `<path class="ln c${k}" d="${pts.map(([x, y], j) => `${j ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join("")}"/>`;
    b += pts.map(([x, y, v, i]) => `<circle class="hit" cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="5"><title>${esc(s.name)}, ${esc(xLabel)} ${i}: ${fmt(v)}</title></circle>`).join("");
    if (pts.length) ends.push({ k, y: pts.at(-1)[1], name: s.name });
  });
  ends.sort((a, c) => a.y - c.y);
  for (let i = 1; i < ends.length; i++) ends[i].y = Math.max(ends[i].y, ends[i - 1].y + 14);
  b += ends.map((e) => `<rect class="c${e.k}" x="${M.left + pw + 8}" y="${e.y - 5}" width="10" height="10" rx="2"/><text class="lbl" x="${M.left + pw + 22}" y="${e.y + 4}">${esc(e.name)}</text>`).join("");
  return frame(title, subtitle, b);
}

/** bars: [{ name, value, note?, series? }], horizontal, drawn from `base` (1 for ratios) so a value below it points left. */
export function barChart({ title, subtitle, xLabel, bars, base = 0, baseName }) {
  const h = Math.max(H, M.top + M.bottom + bars.length * 30);
  const left = M.left + 120, pw = W - left - M.right - 40, ph = bars.length * 30;
  const vals = bars.map((x) => x.value);
  let lo = Math.min(base, ...vals), hi = Math.max(base, ...vals);
  const pad = (hi - lo) * 0.08 || 0.1;
  lo = base === 0 ? 0 : lo - pad; hi += pad;
  const X = (v) => left + ((v - lo) / (hi - lo)) * pw;
  let b = ticks(lo, hi).map((v) => `<line class="g" x1="${X(v).toFixed(1)}" x2="${X(v).toFixed(1)}" y1="${M.top}" y2="${M.top + ph}"/><text class="ax" x="${X(v).toFixed(1)}" y="${M.top + ph + 16}" text-anchor="middle">${fmt(v)}</text>`).join("");
  bars.forEach((x, i) => {
    const y = M.top + i * 30 + 4, x0 = X(Math.min(base, x.value)), x1 = X(Math.max(base, x.value));
    const label = `${fmt(x.value)}${x.note ? `  ${esc(x.note)}` : ""}`;
    b += `<text class="lbl" x="${left - 8}" y="${y + 15}" text-anchor="end">${esc(x.name)}</text>`;
    b += `<rect class="c${x.series ?? 0}" x="${x0.toFixed(1)}" y="${y}" width="${Math.max(1, x1 - x0).toFixed(1)}" height="22" rx="4"><title>${esc(x.name)}: ${label}</title></rect>`;
    b += `<text class="ax" x="${(Math.max(x1, X(base)) + 6).toFixed(1)}" y="${y + 15}">${label}</text>`;
  });
  if (base !== 0) b += `<line class="c1" stroke-dasharray="4 3" stroke-width="1.5" x1="${X(base).toFixed(1)}" x2="${X(base).toFixed(1)}" y1="${M.top - 6}" y2="${M.top + ph}"/><text class="ax" x="${(X(base) + 4).toFixed(1)}" y="${M.top - 8}">${esc(baseName ?? String(base))}</text>`;
  b += `<text class="ax" x="${left + pw / 2}" y="${M.top + ph + 36}" text-anchor="middle">${esc(xLabel)}</text>`;
  return frame(title, subtitle, b, h);
}
