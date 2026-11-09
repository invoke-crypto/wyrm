// Renders media/src/*.svg to 512x512 PNGs with headless Chrome over CDP.
// `node media/render.mjs` writes the PNGs; `--preview <file.png>` also draws a sheet of every image
// at 512, 128, 64 and 32 px on dark and light grounds, with the 64 and 32 px renders magnified.
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { constants, crc32, deflateSync, inflateSync } from 'node:zlib';

const CHROME = process.env.CHROME ?? 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const SIZE = 512;
const MAX_BYTES = 100 * 1024;
const here = dirname(fileURLToPath(import.meta.url));
const NAMES = ['coin', 'scale-ash', 'scale-ember', 'scale-molten', 'scale-obsidian'];
const previewAt = process.argv.includes('--preview') ? resolve(process.argv[process.argv.indexOf('--preview') + 1]) : null;
const pause = (ms) => new Promise((r) => setTimeout(r, ms));

async function openChrome() {
  const port = 9300 + Math.floor(Math.random() * 500);
  const profile = mkdtempSync(join(tmpdir(), 'wyrm-media-'));
  const chrome = spawn(CHROME, ['--headless=new', `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, '--hide-scrollbars', '--no-first-run', '--allow-file-access-from-files', 'about:blank'], { stdio: 'ignore' });
  const close = () => {
    chrome.kill();
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  };
  try {
    let target;
    for (let i = 0; i < 50 && !target; i++) {
      await pause(200);
      try {
        target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === 'page');
      } catch {}
    }
    if (!target) throw new Error(`Chrome did not open a debugging port at ${CHROME}`);
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((r) => ws.addEventListener('open', r));
    let id = 0;
    const pending = new Map();
    ws.addEventListener('message', (m) => {
      const msg = JSON.parse(m.data);
      pending.get(msg.id)?.(msg);
      pending.delete(msg.id);
    });
    const send = (method, params = {}) =>
      new Promise((res, rej) => {
        const n = ++id;
        pending.set(n, (msg) => (msg.error ? rej(new Error(`${method}: ${msg.error.message}`)) : res(msg.result)));
        ws.send(JSON.stringify({ id: n, method, params }));
      });
    await send('Page.enable');
    await send('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } });
    return { send, close: () => (ws.close(), close()) };
  } catch (e) {
    close();
    throw e;
  }
}

// Each image is drawn at twice its size and averaged down. Chrome dithers SVG gradients in a 2x2 pattern;
// averaging cancels that noise, which otherwise keeps a 512 px PNG from compressing under 100 KB.
function decode(png) {
  const width = png.readUInt32BE(16);
  const height = png.readUInt32BE(20);
  const [depth, type, , , interlace] = png.subarray(24, 29);
  if (depth !== 8 || (type !== 6 && type !== 2) || interlace) throw new Error(`expected 8-bit RGB or RGBA from Chrome, got depth ${depth} type ${type}`);
  const idat = [];
  for (let at = 8; at < png.length; ) {
    const len = png.readUInt32BE(at);
    if (png.toString('latin1', at + 4, at + 8) === 'IDAT') idat.push(png.subarray(at + 8, at + 8 + len));
    at += 12 + len;
  }
  const bpp = type === 6 ? 4 : 3;
  const stride = width * bpp;
  const raw = inflateSync(Buffer.concat(idat));
  const px = Buffer.alloc(height * stride);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)];
    const src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    for (let i = 0; i < stride; i++) {
      const a = i >= bpp ? px[y * stride + i - bpp] : 0;
      const b = y ? px[(y - 1) * stride + i] : 0;
      const c = i >= bpp && y ? px[(y - 1) * stride + i - bpp] : 0;
      px[y * stride + i] = (src[i] + [0, a, b, (a + b) >> 1, paeth(a, b, c)][f]) & 255;
    }
  }
  if (bpp === 4) return { width, height, px };
  const rgba = Buffer.alloc(width * height * 4, 255);
  for (let i = 0, j = 0; i < px.length; i += 3, j += 4) px.copy(rgba, j, i, i + 3);
  return { width, height, px: rgba };
}

function halve({ width, height, px }) {
  const w = width / 2;
  const h = height / 2;
  const out = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const at = [0, 1].flatMap((dy) => [0, 1].map((dx) => ((2 * y + dy) * width + 2 * x + dx) * 4));
      const alpha = at.reduce((sum, i) => sum + px[i + 3], 0);
      const o = (y * w + x) * 4;
      for (let ch = 0; ch < 3 && alpha; ch++) out[o + ch] = Math.round(at.reduce((sum, i) => sum + px[i + ch] * px[i + 3], 0) / alpha);
      out[o + 3] = Math.round(alpha / 4);
    }
  return { width: w, height: h, px: out };
}

function encode({ width, height, px }) {
  const opaque = px.every((v, i) => i % 4 !== 3 || v === 255);
  const bpp = opaque ? 3 : 4;
  const pixels = opaque ? Buffer.from(px.filter((_, i) => i % 4 !== 3)) : px;
  const stride = width * bpp;
  const rows = [];
  for (let y = 0; y < height; y++) {
    const row = pixels.subarray(y * stride, (y + 1) * stride);
    const up = y ? pixels.subarray((y - 1) * stride, y * stride) : Buffer.alloc(stride);
    let best;
    for (let f = 0; f < 5; f++) {
      const line = Buffer.alloc(stride + 1);
      line[0] = f;
      let cost = 0;
      for (let i = 0; i < stride; i++) {
        const a = i >= bpp ? row[i - bpp] : 0;
        const c = i >= bpp ? up[i - bpp] : 0;
        const v = (row[i] - [0, a, up[i], (a + up[i]) >> 1, paeth(a, up[i], c)][f]) & 255;
        line[i + 1] = v;
        cost += v < 128 ? v : 256 - v;
      }
      if (!best || cost < best.cost) best = { line, cost };
    }
    rows.push(best.line);
  }
  const chunk = (name, data) => {
    const body = Buffer.concat([Buffer.from(name, 'latin1'), data]);
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, opaque ? 2 : 6, 0, 0, 0], 8);
  const data = deflateSync(Buffer.concat(rows), { level: 9, memLevel: 9, strategy: constants.Z_FILTERED });
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', data), chunk('IEND', Buffer.alloc(0))]);
}

function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

async function shoot(page, url, width, height, scale = 1) {
  await page.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: scale, mobile: false });
  await page.send('Page.navigate', { url });
  await page.send('Runtime.evaluate', { expression: `new Promise((r) => (document.readyState === 'complete' ? r() : addEventListener('load', r))).then(() => document.fonts?.ready).then(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))))`, awaitPromise: true });
  const { data } = await page.send('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width, height, scale: 1 } });
  return Buffer.from(data, 'base64');
}

// Each small size is drawn the way a wallet would show it: the 512 PNG downscaled by the browser.
function sheet(pngs) {
  const cell = (png, size, ground) => `<div style="background:${ground}"><img src="${pathToFileURL(png)}" width="${size}" height="${size}"></div>`;
  const zoom = (png, size, ground) => `<canvas data-src="${pathToFileURL(png)}" data-size="${size}" data-ground="${ground}" width="256" height="256"></canvas>`;
  const grounds = ['#14101d', '#ffffff'];
  const rows = pngs
    .map((png) => `<section>${grounds.map((g) => `<div class="g" style="background:${g}">${cell(png, 512, g)}<div class="small">${[128, 64, 32].map((s) => cell(png, s, g)).join('')}</div><div class="zoom">${[64, 32].map((s) => zoom(png, s, g)).join('')}</div></div>`).join('')}</section>`)
    .join('');
  return `<!doctype html><meta charset="utf-8"><style>
body { margin: 0; background: #888; }
section { display: flex; gap: 8px; margin-bottom: 8px; }
.g { display: flex; gap: 24px; align-items: center; padding: 24px; }
.small { display: flex; gap: 24px; align-items: center; }
.zoom { display: flex; flex-direction: column; gap: 8px; }
canvas { image-rendering: pixelated; width: 256px; height: 256px; }
</style>${rows}<script>
for (const c of document.querySelectorAll('canvas')) {
  const img = new Image();
  img.src = c.dataset.src;
  img.decode().then(() => {
    const n = +c.dataset.size, tiny = document.createElement('canvas');
    tiny.width = tiny.height = n;
    const t = tiny.getContext('2d');
    t.fillStyle = c.dataset.ground; t.fillRect(0, 0, n, n);
    t.imageSmoothingQuality = 'high';
    t.drawImage(img, 0, 0, n, n);
    const x = c.getContext('2d');
    x.imageSmoothingEnabled = false;
    x.drawImage(tiny, 0, 0, c.width, c.height);
  });
}
</script>`;
}

const page = await openChrome();
try {
  const pngs = [];
  for (const name of NAMES) {
    const png = join(here, `${name}.png`);
    writeFileSync(png, encode(halve(decode(await shoot(page, pathToFileURL(join(here, 'src', `${name}.svg`)).href, SIZE, SIZE, 2)))));
    const bytes = statSync(png).size;
    console.log(`${name}.svg -> ${png} ${(bytes / 1024).toFixed(1)} KB${bytes > MAX_BYTES ? ' OVER 100 KB' : ''}`);
    if (bytes > MAX_BYTES) process.exitCode = 1;
    pngs.push(png);
  }
  if (previewAt) {
    const html = join(dirname(previewAt), 'preview.html');
    writeFileSync(html, sheet(pngs));
    writeFileSync(previewAt, await shoot(page, pathToFileURL(html).href, 2280, pngs.length * 568));
    console.log(`preview -> ${previewAt}`);
  }
} finally {
  page.close();
}
