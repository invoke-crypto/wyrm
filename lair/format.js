export const YOCTO_PER_NEAR = 10n ** 24n;
export const NS_PER_MS = 1_000_000n;
export const NS_PER_DAY = 86_400_000_000_000n;

export function formatNear(yocto, digits = 2) {
  if (yocto === 0n) return '0';
  const negative = yocto < 0n;
  const abs = negative ? -yocto : yocto;
  const scale = 10n ** BigInt(digits);
  const scaled = (abs * scale + YOCTO_PER_NEAR / 2n) / YOCTO_PER_NEAR;
  if (scaled === 0n) return `${negative ? '>-' : '<'}${digits ? '0.' + '0'.repeat(digits - 1) + '1' : '1'}`;
  return (negative ? '-' : '') + fixed(scaled, digits);
}

export function formatToken(amount, decimals, digits = 0) {
  const unit = 10n ** BigInt(decimals);
  const scale = 10n ** BigInt(digits);
  return fixed((amount * scale + unit / 2n) / unit, digits);
}

function fixed(scaled, digits) {
  const s = scaled.toString().padStart(digits + 1, '0');
  const whole = s.slice(0, s.length - digits);
  const frac = digits ? s.slice(s.length - digits).replace(/0+$/, '') : '';
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return frac ? `${grouped}.${frac}` : grouped;
}

export function parseNearInput(text) {
  const m = /^\s*(\d+)(?:\.(\d{0,24}))?\s*$/.exec(text ?? '');
  if (!m) return null;
  return BigInt(m[1]) * YOCTO_PER_NEAR + BigInt((m[2] ?? '').padEnd(24, '0') || '0');
}

export function parseTokenInput(text, decimals) {
  const m = new RegExp(`^\\s*(\\d+)(?:\\.(\\d{0,${decimals}}))?\\s*$`).exec(text ?? '');
  if (!m) return null;
  return BigInt(m[1]) * 10n ** BigInt(decimals) + BigInt((m[2] ?? '').padEnd(decimals, '0') || '0');
}

export function formatBps(bps) {
  const s = (Number(bps) / 100).toFixed(2).replace(/\.?0+$/, '');
  return `${s}%`;
}

export function percentToBps(text) {
  const m = /^\s*(\d+)(?:\.(\d{0,2}))?\s*$/.exec(text ?? '');
  if (!m) return null;
  return Number(m[1]) * 100 + Number((m[2] ?? '').padEnd(2, '0') || '0');
}

export function nsToDate(ns) {
  return new Date(Number(ns / NS_PER_MS));
}

export function formatAgo(ns, nowNs) {
  return formatSpan(nowNs - ns) + ' ago';
}

export function formatIn(ns, nowNs) {
  return ns <= nowNs ? 'now' : 'in ' + formatSpan(ns - nowNs);
}

export function formatSpan(ns) {
  const s = Number((ns < 0n ? -ns : ns) / 1_000_000_000n);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
}

export function formatUtc(ns) {
  return nsToDate(ns).toISOString().slice(0, 16).replace('T', ' ') + ' UTC';
}

export function shortAccount(id) {
  if (!id) return '';
  if (id.length <= 22) return id;
  return /^[0-9a-f]{64}$/.test(id) ? `${id.slice(0, 6)}…${id.slice(-4)}` : `${id.slice(0, 12)}…${id.slice(-8)}`;
}

export function padScaleId(id) {
  return String(id).padStart(3, '0');
}

export function formatLength(ns) {
  const minutes = Number(ns / 60_000_000_000n);
  const [n, unit] = minutes % 1440 === 0 ? [minutes / 1440, 'day'] : minutes % 60 === 0 ? [minutes / 60, 'hour'] : [minutes, 'minute'];
  return `${n} ${unit}${n === 1 ? '' : 's'}`;
}

// The Dragon writes its refusals in base units and basis points. A deposit names its unit, yoctoNEAR, and rounds up
// to 0.0001 NEAR, so an "at least" is never shown below what it asks. Any other number of 19 or more digits is WYRM.
export function formatReason(text, decimals = 18) {
  return text
    .replace(/\b(\d+) yoctoNEAR\b/g, (_, n) => `${formatNear(ceilTo(BigInt(n), YOCTO_PER_NEAR / 10_000n), 4)} NEAR`)
    .replace(/\b\d{19,}\b/g, (n) => `${formatToken(BigInt(n), decimals, 2)} WYRM`)
    .replace(/\b(\d+) bps\b/g, (_, n) => formatBps(n));
}

const ceilTo = (n, step) => ((n + step - 1n) / step) * step;
