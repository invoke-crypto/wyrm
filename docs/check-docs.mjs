// Checks the published Markdown: every relative link and #anchor resolves, no v1 number survives outside a history
// section, and docs/SPEC.md's method tables list exactly the methods each contract exports. Exits 1 on any fault.
//
//   node docs/check-docs.mjs [--root DIR]
//
// Published means what `git ls-files -co --exclude-standard` lists, so a link to a gitignored working doc is broken.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const CONTRACTS = ["tithe", "scale-vault", "scales", "dragon"];

const GYM_GENERATED_WITH_V1_COLUMNS = ["docs/PARAMETERS.md", "docs/GAS.md"];
const DATED_RESEARCH_NOTES = "docs/research/";
const staleExempt = (f) => GYM_GENERATED_WITH_V1_COLUMNS.includes(f) || f.startsWith(DATED_RESEARCH_NOTES);
const namesV1 = (text) => /\bv1\b/i.test(text);
const isHistory = (headingStack, line) => headingStack.some((h) => /history/i.test(h.text) || namesV1(h.text)) || namesV1(line);

export const STALE = [
  { what: 'a "70" sale count', re: /\b70\s+(?:sale\s+|sold\s+)?Scales?\b|\b(?:ids|Scales)\s+1\s+to\s+70\b/i },
  { what: 'a "30" creator Scales count', re: /\b30\s+creator\b|\bcreator's\s+30\b|\b71\s+to\s+100\b/i },
  { what: "a 20% budget", re: /\b20%\s+(?:daily\s+)?budget\b|\bbudget\s+(?:of\s+)?20%|\b2,000\s+bps\s+of\s+the\s+reserve\b/i },
  { what: "a 2.5% cap", re: /\b2\.5%|\b250\s+bps\b/ },
  { what: "pause_buys", re: /\bpause_buys\b/ },
];

export function published(root) {
  return execFileSync("git", ["ls-files", "-co", "--exclude-standard", "-z"], { cwd: root, encoding: "utf8" })
    .split("\0").filter(Boolean).filter((f) => fs.existsSync(path.join(root, f)));
}

// GitHub's heading ids: rendered text, lower case, punctuation dropped, spaces to hyphens, repeats numbered.
export function anchors(markdown) {
  const seen = new Map();
  const ids = new Set();
  for (const { text } of headings(markdown)) {
    const plain = text.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1").replace(/[`*_]{1,2}([^`*_]+)[`*_]{1,2}/g, "$1");
    const base = plain.toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, "").trim().replace(/\s/g, "-");
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    ids.add(n ? `${base}-${n}` : base);
  }
  return ids;
}

function headings(markdown) {
  const out = [];
  let fenced = false;
  markdown.split("\n").forEach((line, i) => {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const m = !fenced && line.match(/^(#{1,6})\s+(.*?)\s*#*\s*$/);
    if (m) out.push({ line: i, level: m[1].length, text: m[2] });
  });
  return out;
}

function links(markdown) {
  const out = [];
  let fenced = false;
  markdown.split("\n").forEach((line, i) => {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
      return;
    }
    if (fenced) return;
    for (const m of line.replace(/`[^`]*`/g, "").matchAll(/!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)/g)) {
      out.push({ line: i + 1, target: m[1] });
    }
  });
  return out;
}

export function brokenLinks(root, files) {
  const set = new Set(files);
  const dirs = new Set(files.flatMap((f) => f.split("/").slice(0, -1).map((_, i, parts) => parts.slice(0, i + 1).join("/"))));
  const ids = new Map();
  const idsOf = (f) => {
    if (!ids.has(f)) ids.set(f, anchors(fs.readFileSync(path.join(root, f), "utf8")));
    return ids.get(f);
  };
  const out = [];
  for (const file of files.filter((f) => f.endsWith(".md"))) {
    for (const { line, target } of links(fs.readFileSync(path.join(root, file), "utf8"))) {
      if (/^[a-z][a-z0-9+.-]*:/i.test(target)) continue;
      const [rawPath, anchor] = target.split("#");
      const to = rawPath ? path.posix.normalize(path.posix.join(path.posix.dirname(file), decodeURIComponent(rawPath))).replace(/\/$/, "") : file;
      const where = `${file}:${line} ${target}`;
      if (!set.has(to) && !dirs.has(to)) out.push(`broken link ${where}: ${to} is not published`);
      else if (anchor && set.has(to) && to.endsWith(".md") && !idsOf(to).has(anchor.toLowerCase())) out.push(`broken link ${where}: ${to} has no heading #${anchor}`);
    }
  }
  return out;
}

export function staleNumbers(root, files) {
  const out = [];
  for (const file of files.filter((f) => f.endsWith(".md") && !staleExempt(f))) {
    const text = fs.readFileSync(path.join(root, file), "utf8");
    const heads = headings(text);
    const stack = [];
    text.split("\n").forEach((line, i) => {
      const h = heads.find((x) => x.line === i);
      if (h) {
        while (stack.length && stack.at(-1).level >= h.level) stack.pop();
        stack.push(h);
      }
      if (isHistory(stack, line)) return;
      for (const { what, re } of STALE) if (re.test(line)) out.push(`stale number ${file}:${i + 1}: ${what}: ${line.trim().slice(0, 120)}`);
    });
  }
  return out;
}

export function contractMethods(srcDir) {
  const names = new Set();
  const rsFiles = (dir) => fs.readdirSync(dir, { withFileTypes: true })
    .flatMap((e) => (e.isDirectory() ? rsFiles(path.join(dir, e.name)) : e.name.endsWith(".rs") ? [path.join(dir, e.name)] : []));
  for (const file of rsFiles(srcDir)) {
    const lines = fs.readFileSync(file, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      const ext = lines[i].match(/^\s*pub extern "C" fn (\w+)/);
      if (ext && /#\[no_mangle\]/.test(lines[i - 1] ?? "")) names.add(ext[1]);
      if (!/^#\[near\]\s*$/.test(lines[i]) || !/^impl\b/.test(lines[i + 1] ?? "")) continue;
      let depth = 0;
      for (let j = i + 1; j < lines.length; j++) {
        const m = depth === 1 && lines[j].match(/^\s*pub fn (\w+)/);
        if (m) names.add(m[1]);
        for (const c of lines[j].replace(/r(#+)".*?"\1/g, "").replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)'|\/\/.*$/g, "")) depth += c === "{" ? 1 : c === "}" ? -1 : 0;
        if (depth === 0 && lines[j].includes("}")) break;
      }
    }
  }
  return names;
}

export function specMethods(spec, contract) {
  const lines = spec.split("\n");
  const start = lines.findIndex((l) => l.trim() === `## ${contract}`);
  if (start < 0) return null;
  const names = new Set();
  let inTable = false;
  for (const line of lines.slice(start + 1)) {
    if (/^## /.test(line)) break;
    if (!line.startsWith("|")) {
      inTable = false;
      continue;
    }
    const first = line.split("|")[1].trim();
    if (/^Method$/i.test(first)) inTable = true;
    else if (inTable && !/^-+$/.test(first)) for (const m of first.matchAll(/`(\w+)(?:\(|`)/g)) names.add(m[1]);
  }
  return names;
}

export function specDiff(root) {
  const spec = fs.readFileSync(path.join(root, "docs/SPEC.md"), "utf8");
  const out = [];
  for (const c of CONTRACTS) {
    const code = contractMethods(path.join(root, "contracts", c, "src"));
    const doc = specMethods(spec, c);
    if (!doc) {
      out.push(`SPEC.md has no "## ${c}" section`);
      continue;
    }
    for (const m of [...code].filter((m) => !doc.has(m)).sort()) out.push(`SPEC.md ${c} lacks ${m}, which contracts/${c} exports`);
    for (const m of [...doc].filter((m) => !code.has(m)).sort()) out.push(`SPEC.md ${c} lists ${m}, which contracts/${c} does not export`);
  }
  return out;
}

export function checkDocs(root) {
  const files = published(root);
  const broken = brokenLinks(root, files);
  const stale = staleNumbers(root, files);
  const methods = specDiff(root);
  const summary = `${broken.length} broken links, ${stale.length} stale numbers, ${methods.length ? `SPEC methods differ (${methods.length})` : "SPEC methods match"}`;
  return { faults: [...broken, ...stale, ...methods], summary };
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const i = process.argv.indexOf("--root");
  const root = i > 0 ? path.resolve(process.argv[i + 1]) : path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const { faults, summary } = checkDocs(root);
  for (const f of faults) console.log(f);
  console.log(summary);
  process.exit(faults.length ? 1 : 0);
}
