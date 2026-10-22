#!/usr/bin/env python3
"""Writes crates/tapetum/assets/third-party-licenses.txt: every crate Tapetum
links on any platform, with its license and the license texts it ships.
Run after changing dependencies:  python3 scripts/gen-licenses.py"""
import json, os, re, subprocess, sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
os.chdir(root)
tree = subprocess.run(
    ["cargo", "tree", "-p", "tapetum", "-e", "normal,build", "--target", "all", "--prefix", "none", "--format", "{p}"],
    capture_output=True, text=True, check=True).stdout
wanted = set()
for line in tree.splitlines():
    m = re.match(r"(\S+) v(\S+)", line.strip())
    if m: wanted.add((m.group(1), m.group(2)))
meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1"], capture_output=True, text=True, check=True).stdout)
ours = {"tapetum", "tapetum-project", "bk-olga", "bk-artnet"}
pat = re.compile(r"(licen[cs]e|copying|notice|copyright|^ofl|^ufl|authors)", re.I)
skip_dirs = {"tests", "test", "examples", "benches", "target", ".git", "node_modules"}
packages, texts = [], {}
for p in sorted(meta["packages"], key=lambda p: (p["name"].lower(), p["version"])):
    if (p["name"], p["version"]) not in wanted or p["name"] in ours:
        continue
    base = Path(p["manifest_path"]).parent
    files = []
    for dirpath, dirs, names in os.walk(base):
        dirs[:] = [d for d in dirs if d not in skip_dirs]
        if Path(dirpath).relative_to(base).parts.__len__() > 2: continue
        for n in sorted(names):
            if pat.search(n) and not n.endswith((".rs", ".toml", ".json", ".py", ".sh", ".html", ".js")):
                f = Path(dirpath) / n
                try: t = f.read_text(encoding="utf-8", errors="replace").strip()
                except Exception: continue
                if 30 < len(t) < 200_000:
                    files.append((str(f.relative_to(base)), t))
    keys = []
    for name, t in files:
        key = re.sub(r"\s+", " ", t)
        texts.setdefault(key, (t, []))[1].append(f"{p['name']} {p['version']} ({name})")
        keys.append(key)
    packages.append((p["name"], p["version"], p.get("license") or "see repository", p.get("repository") or ""))

out = ["Tapetum uses the following open source software. Thank you to all its authors.", "",
       f"{len(packages)} crates:", ""]
for n, v, lic, repo in packages:
    out.append(f"  {n} {v} — {lic}" + (f" — {repo}" if repo else ""))
out += ["", "", "License texts, as shipped by the crates:", ""]
for t, users in sorted(texts.values(), key=lambda x: x[1][0].lower()):
    out.append("=" * 78)
    out += [f"  {u}" for u in users]
    out.append("-" * 78)
    out.append(t)
    out.append("")
dest = root / "crates/tapetum/assets/third-party-licenses.txt"
dest.parent.mkdir(parents=True, exist_ok=True)
dest.write_text("\n".join(out) + "\n", encoding="utf-8")
print(f"{len(packages)} crates, {len(texts)} distinct license texts, {dest.stat().st_size // 1024} KiB -> {dest.relative_to(root)}")
