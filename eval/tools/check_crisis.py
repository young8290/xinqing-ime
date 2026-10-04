#!/usr/bin/env python3
"""心晴 · 危机词表离线评测（E-CRISIS 本地通道，FR-SAF-01）

用法：python3 check_crisis.py [--golden 输出路径] [词表路径] [数据集路径]
--golden：把每条样本的得分和判定写成 JSON，供 Rust safety 模块单元测试对拍。
按 crisis_lexicon.toml 文件头注释中的判定算法计算得分，输出漏报、误报和召回率/误报率。
这是 Hub safety 模块的参考实现，Rust 实现的结果必须与本脚本一致（单元测试对拍）。
门槛：召回率 100%；误报率 < 10%（08 第 8 节）。
"""
import json, re, sys, tomllib
from pathlib import Path

here = Path(__file__).resolve().parent
args = sys.argv[1:]
golden_path = None
if "--golden" in args:
    i = args.index("--golden"); golden_path = Path(args[i + 1]); del args[i:i + 2]
sys.argv = sys.argv[:1] + args
lex_path = Path(sys.argv[1]) if len(sys.argv) > 1 else here.parent.parent / "hub_templates" / "crisis_lexicon.toml"
data_path = Path(sys.argv[2]) if len(sys.argv) > 2 else here.parent / "datasets" / "e_crisis.jsonl"
L = tomllib.loads(lex_path.read_text(encoding="utf-8"))
T2S = tomllib.loads((lex_path.parent / "crisis_t2s.toml").read_text(encoding="utf-8"))["map"]
PUNCT = re.compile(r"[\s，。！？、,.!?…~；;：:]")

def normalize(text: str) -> str:
    """词表判定算法第 1 步：全角转半角、繁转简，再去空格和标点。"""
    out = []
    for c in text:
        o = ord(c)
        if 0xFF01 <= o <= 0xFF5E:
            c = chr(o - 0xFEE0)
        elif o == 0x3000:
            c = " "
        out.append(T2S.get(c, c))
    return PUNCT.sub("", "".join(out))

def score(text: str, threshold_key: str = "threshold") -> tuple[float, bool]:
    t = normalize(text)
    benign = [m.span() for b in L["benign"] for m in re.finditer(b["pattern"], t)]
    quoted = any(q in t for q in L["quote"]) and not any(r in t for r in L.get("self_ref", []))
    best: dict[str, float] = {}
    for e in L["entry"]:
        for m in re.finditer(e["pattern"], t):
            s, en = m.span()
            if any(bs < en and s < be for bs, be in benign):
                continue
            w = e["weight"]
            if any(n in t[max(0, s - 6):s] for n in L["negation"]):
                w *= L["negation_factor"]
            if quoted:
                w *= L["quote_factor"]
            best[e["category"]] = max(best.get(e["category"], 0.0), w)
    total = sum(best.values())
    return total, total >= L[threshold_key]

pos = neg = tp = fp = 0
golden = {}
for line in data_path.read_text(encoding="utf-8").splitlines():
    d = json.loads(line)
    s, hit = score(d["text"])
    golden[d["id"]] = {"score": round(s, 6), "hit": hit}
    if d["label"] == 1:
        pos += 1; tp += hit
        if not hit: print(f"漏报 {d['id']} {s:.2f} {d['text']}")
    else:
        neg += 1; fp += hit
        if hit: print(f"误报 {d['id']} {s:.2f} {d['text']}")
if golden_path:
    golden_path.write_text(json.dumps(golden, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
print(f"召回率 {tp}/{pos} = {tp/pos:.0%}；误报率 {fp}/{neg} = {fp/neg:.0%}")
sys.exit(0 if tp == pos and fp / neg < 0.10 else 1)
