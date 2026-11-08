"""Write a self-contained graph.html: the crate graph and each crate's module graph.

Inputs: graph.json (from module_graph.py, in the output folder) and `cargo metadata` (for the crate graph).
The page has no network dependencies: the layout is a small force simulation run in the browser.
"""

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "target" / "modgraph"
GRAPH = OUT_DIR / "graph.json"
OUT = OUT_DIR / "graph.html"

graph = json.loads(GRAPH.read_text(encoding="utf-8"))
meta = json.loads(
    subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
)

# Crate-level graph: normal (non-dev) edges between workspace crates.
names = {p["name"] for p in meta["packages"]}
crate_nodes = []
crate_edges = []
crate_lines = {}
for c, data in graph["crates"].items():
    crate_lines[c] = sum(n["lines"] for n in data["nodes"])
for p in meta["packages"]:
    short = p["name"].removeprefix("pg-")
    crate_nodes.append({"id": p["name"], "label": short, "lines": crate_lines.get(short, 0)})
    crate_edges.extend(
        {"from": p["name"], "to": d["name"], "weight": 1} for d in p["dependencies"] if d["name"] in names and d["kind"] is None
    )

views = {"crates": {"nodes": crate_nodes, "edges": crate_edges, "cycles": []}}
for c, data in graph["crates"].items():
    views[c] = {"nodes": data["nodes"], "edges": data["edges"], "cycles": data["cycles"]}

data_json = json.dumps(views)

HTML = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Module graph</title>
<style>
:root { --bg:#fbfaf7; --fg:#1d1d1f; --muted:#6b6b70; --edge:#8a8a90; --node:#dbe8f5; --node-stroke:#3d6fa3;
        --root:#e6e6e8; --cycle:#fbe3c8; --cycle-stroke:#c4661c; --panel:#ffffff; --line:#e3e3e6; }
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) { --bg:#16171b; --fg:#e8e8ea; --muted:#9a9aa2; --edge:#6e6e78; --node:#243447;
        --node-stroke:#6b9fd6; --root:#2c2d33; --cycle:#4a3320; --cycle-stroke:#e09150; --panel:#1f2026; --line:#34353d; }
}
:root[data-theme="dark"] { --bg:#16171b; --fg:#e8e8ea; --muted:#9a9aa2; --edge:#6e6e78; --node:#243447;
        --node-stroke:#6b9fd6; --root:#2c2d33; --cycle:#4a3320; --cycle-stroke:#e09150; --panel:#1f2026; --line:#34353d; }
html, body { margin:0; background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, sans-serif; }
header { display:flex; flex-wrap:wrap; gap:12px; align-items:center; padding:12px 16px; border-bottom:1px solid var(--line); }
header h1 { font-size:16px; margin:0 12px 0 0; }
select, label { font:inherit; color:var(--fg); }
select { background:var(--panel); border:1px solid var(--line); border-radius:6px; padding:4px 8px; }
main { display:grid; grid-template-columns: minmax(0, 1fr) 320px; min-height: calc(100vh - 52px); }
#canvas { width:100%; height:calc(100vh - 52px); min-height:560px; display:block; }
aside { border-left:1px solid var(--line); padding:12px 14px; overflow:auto; background:var(--panel); }
aside h2 { font-size:13px; margin:14px 0 6px; text-transform:uppercase; letter-spacing:.04em; color:var(--muted); }
aside h2:first-child { margin-top:0; }
aside ul { margin:0; padding-left:18px; }
aside li { margin:2px 0; }
code { font-family: ui-monospace, Consolas, monospace; font-size:12.5px; }
.muted { color:var(--muted); }
.node rect { fill:var(--node); stroke:var(--node-stroke); stroke-width:1.2; }
.node.root rect { fill:var(--root); }
.node.cycle rect { fill:var(--cycle); stroke:var(--cycle-stroke); stroke-width:1.6; }
.node text { fill:var(--fg); font-size:11.5px; pointer-events:none; }
.node .sub { fill:var(--muted); font-size:10px; }
.edge { fill:none; stroke:var(--edge); }
.dim { opacity:.12; }
@media (max-width: 860px) { main { grid-template-columns: 1fr; } aside { border-left:none; border-top:1px solid var(--line); } }
</style>
</head>
<body>
<header>
  <h1>Module graph</h1>
  <label>View <select id="view"></select></label>
  <span class="muted" id="stats"></span>
  <span class="muted" style="margin-left:auto">Node size: lines. Orange: in a cycle. Hover a node to see its neighbours.</span>
</header>
<main>
  <svg id="canvas" role="img" aria-label="dependency graph"></svg>
  <aside id="side"></aside>
</main>
<script>
const VIEWS = __DATA__;
const NS = "http://www.w3.org/2000/svg";
const svg = document.getElementById("canvas");
const sel = document.getElementById("view");
const side = document.getElementById("side");

for (const key of Object.keys(VIEWS)) {
  const o = document.createElement("option");
  o.value = key;
  o.textContent = key === "crates" ? "Crates (workspace)" : key;
  sel.appendChild(o);
}

function el(tag, attrs, parent) {
  const e = document.createElementNS(NS, tag);
  for (const k in attrs) e.setAttribute(k, attrs[k]);
  if (parent) parent.appendChild(e);
  return e;
}

function layout(nodes, edges, W, H) {
  const idx = new Map(nodes.map((n, i) => [n.id, i]));
  nodes.forEach((n, i) => {
    const a = (2 * Math.PI * i) / nodes.length;
    n.x = W / 2 + Math.cos(a) * W * 0.32;
    n.y = H / 2 + Math.sin(a) * H * 0.32;
    n.w = Math.max(96, Math.min(230, 9 * n.label.length + 22));
    n.h = 36;
  });
  const E = edges
    .map(e => [idx.get(e.from), idx.get(e.to), e.weight])
    .filter(e => e[0] !== undefined && e[1] !== undefined && e[0] !== e[1]);
  // Fruchterman-Reingold: every pair repels, every edge attracts, and each step moves a node by at most
  // the current temperature, which cools to zero. A rectangle pass then pushes overlapping boxes apart.
  const n = nodes.length;
  const k = Math.sqrt((W * H) / Math.max(n, 1)) * 1.15;
  const ITER = 800;
  const GAP = 14;
  const minSide = Math.min(W, H);
  for (let it = 0; it < ITER; it++) {
    const temp = minSide * 0.1 * (1 - it / ITER) + 0.5;
    const fx = new Float64Array(n), fy = new Float64Array(n);
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const dx = nodes[i].x - nodes[j].x, dy = nodes[i].y - nodes[j].y;
        const d = Math.sqrt(dx * dx + dy * dy) + 0.01;
        const f = (k * k) / d;
        fx[i] += (dx / d) * f; fy[i] += (dy / d) * f;
        fx[j] -= (dx / d) * f; fy[j] -= (dy / d) * f;
      }
    }
    for (const [a, b, w] of E) {
      const dx = nodes[b].x - nodes[a].x, dy = nodes[b].y - nodes[a].y;
      const d = Math.sqrt(dx * dx + dy * dy) + 0.01;
      const f = ((d * d) / k) * Math.min(w, 4) * 0.5;
      fx[a] += (dx / d) * f; fy[a] += (dy / d) * f;
      fx[b] -= (dx / d) * f; fy[b] -= (dy / d) * f;
    }
    for (let i = 0; i < n; i++) {
      fx[i] += (W / 2 - nodes[i].x) * 0.01;
      fy[i] += (H / 2 - nodes[i].y) * 0.01;
      const disp = Math.sqrt(fx[i] * fx[i] + fy[i] * fy[i]) + 1e-9;
      const step = Math.min(disp, temp);
      nodes[i].x += (fx[i] / disp) * step;
      nodes[i].y += (fy[i] / disp) * step;
    }
    // Boxes must not overlap: push each overlapping pair apart along the axis with less overlap.
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        const dx = nodes[j].x - nodes[i].x, dy = nodes[j].y - nodes[i].y;
        const ox = (nodes[i].w + nodes[j].w) / 2 + GAP - Math.abs(dx);
        const oy = (nodes[i].h + nodes[j].h) / 2 + GAP - Math.abs(dy);
        if (ox <= 0 || oy <= 0) continue;
        if (ox < oy) {
          const s = (dx < 0 ? -1 : 1) * ox / 2;
          nodes[i].x -= s; nodes[j].x += s;
        } else {
          const s = (dy < 0 ? -1 : 1) * oy / 2;
          nodes[i].y -= s; nodes[j].y += s;
        }
      }
    }
    for (let i = 0; i < n; i++) {
      nodes[i].x = Math.max(nodes[i].w / 2 + 8, Math.min(W - nodes[i].w / 2 - 8, nodes[i].x));
      nodes[i].y = Math.max(nodes[i].h / 2 + 8, Math.min(H - nodes[i].h / 2 - 8, nodes[i].y));
    }
  }
  return E;
}

function largest(nodes) {
  return nodes.slice().sort((a, b) => (b.lines || 0) - (a.lines || 0)).slice(0, 8)
    .map(n => `<li><code>${n.id || "(crate root)"}</code> ${n.lines} lines</li>`).join("");
}

function render(key) {
  const view = VIEWS[key];
  svg.innerHTML = "";
  const rect = svg.getBoundingClientRect();
  const W = Math.max(640, rect.width), H = Math.max(560, rect.height);
  svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
  const nodes = view.nodes.map(n => ({ ...n }));
  const edges = view.edges;
  const inCycle = new Set(view.cycles.flat());
  const E = layout(nodes, edges, W, H);

  const defs = el("defs", {}, svg);
  const marker = el("marker", {
    id: "arrow", viewBox: "0 0 10 10", refX: 10, refY: 5, markerWidth: 7, markerHeight: 7,
    orient: "auto-start-reverse",
  }, defs);
  const edgeColour = getComputedStyle(document.documentElement).getPropertyValue("--edge") || "#888";
  el("path", { d: "M0,0 L10,5 L0,10 z", fill: edgeColour }, marker);

  const edgeLayer = el("g", {}, svg);
  const nodeLayer = el("g", {}, svg);
  const pairs = new Set(edges.map(e => e.from + "\u0000" + e.to));
  const edgeEls = [];
  for (const e of edges) {
    const a = nodes.find(n => n.id === e.from), b = nodes.find(n => n.id === e.to);
    if (!a || !b) continue;
    const dx = b.x - a.x, dy = b.y - a.y, len = Math.hypot(dx, dy) || 1;
    const bend = pairs.has(e.to + "\u0000" + e.from) ? 0.12 : 0.04;
    const mx = (a.x + b.x) / 2 - (dy / len) * len * bend, my = (a.y + b.y) / 2 + (dx / len) * len * bend;
    const p = el("path", {
      class: "edge",
      d: `M${a.x},${a.y} Q${mx},${my} ${b.x},${b.y}`,
      "stroke-width": 1 + Math.min(e.weight, 6) * 0.45,
      "marker-end": "url(#arrow)",
      opacity: 0.8,
    }, edgeLayer);
    const t = el("title", {}, p);
    t.textContent = `${e.from || "(crate root)"} -> ${e.to || "(crate root)"}: ${e.weight} reference(s)`;
    edgeEls.push({ e, p });
  }

  const nodeEls = new Map();
  for (const n of nodes) {
    const cls = ["node"];
    if (n.label === "(crate root)") cls.push("root");
    if (inCycle.has(n.id)) cls.push("cycle");
    const g = el("g", { class: cls.join(" "), transform: `translate(${n.x - n.w / 2},${n.y - n.h / 2})` }, nodeLayer);
    el("rect", { width: n.w, height: n.h, rx: 6 }, g);
    const t = el("text", { x: n.w / 2, y: 15, "text-anchor": "middle" }, g);
    t.textContent = n.label;
    const s = el("text", { x: n.w / 2, y: 28, "text-anchor": "middle", class: "sub" }, g);
    s.textContent = n.lines ? `${n.lines} lines` : "";
    nodeEls.set(n.id, g);
    g.addEventListener("mouseenter", () => focus(n.id));
    g.addEventListener("mouseleave", () => focus(null));
  }

  function focus(id) {
    for (const [nid, g] of nodeEls) {
      const near = !id || nid === id || edges.some(e => (e.from === id && e.to === nid) || (e.to === id && e.from === nid));
      g.classList.toggle("dim", !near);
    }
    for (const { e, p } of edgeEls) {
      const on = !id || e.from === id || e.to === id;
      p.classList.toggle("dim", !on);
    }
  }

  const label = id => id || "(crate root)";
  const cyc = view.cycles.map(c => `<li>${c.map(x => `<code>${label(x)}</code>`).join(" &harr; ")}</li>`).join("");
  const degIn = new Map(), degOut = new Map();
  for (const e of edges) { degOut.set(e.from, (degOut.get(e.from) || 0) + 1); degIn.set(e.to, (degIn.get(e.to) || 0) + 1); }
  const top = (m) => [...m.entries()].sort((x, y) => y[1] - x[1]).slice(0, 6)
    .map(([k, v]) => `<li><code>${label(k)}</code> (${v})</li>`).join("");
  const totalLines = nodes.reduce((s, n) => s + (n.lines || 0), 0);
  document.getElementById("stats").textContent =
    `${nodes.length} ${key === "crates" ? "crates" : "modules"}, ${edges.length} edges, ${view.cycles.length} cycle group(s)` +
    (totalLines ? `, ${totalLines.toLocaleString()} lines` : "");
  side.innerHTML = `
    <h2>Cycles (mutual references)</h2>
    ${view.cycles.length ? `<ul>${cyc}</ul>` : `<p class="muted">None.</p>`}
    <h2>Most referenced (fan-in)</h2><ul>${top(degIn) || "<li class='muted'>none</li>"}</ul>
    <h2>Most dependencies (fan-out)</h2><ul>${top(degOut) || "<li class='muted'>none</li>"}</ul>
    ${key === "crates" ? "" : `<h2>Largest modules</h2><ul>${largest(nodes)}</ul>`}
  `;
}

sel.addEventListener("change", () => render(sel.value));
window.addEventListener("resize", () => render(sel.value));
sel.value = "crates";
render("crates");
</script>
</body>
</html>
"""

OUT.write_text(HTML.replace("__DATA__", data_json), encoding="utf-8")
print("wrote", OUT, "views:", ", ".join(views))
