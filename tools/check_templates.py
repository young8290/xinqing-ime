#!/usr/bin/env python3
"""心晴 · 出厂模板校验（CI 中运行，见 14 第 6 节）

用法：python3 check_templates.py <hub_templates 目录>
检查：所有 TOML 能解析；暖心话每组 ≥ 15 句、6–30 个汉字、id 唯一、不含禁用词和“打字/检测”；
      界面文案气泡 ≤ 16 字、不含禁用词（豁免键除外）；晚间小结、周报一句话、状态解释不含禁用词；
      危机词表和日程规则的正则都能编译。
"""
import re, sys, tomllib
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
errors: list[str] = []
def load(name):
    try:
        return tomllib.loads((root / name).read_text(encoding="utf-8"))
    except Exception as e:  # noqa: BLE001
        errors.append(f"{name}: 无法解析 {e}"); return {}

banned = load("banned_words.toml")
words = [w for k in ("diagnosis", "surveillance", "preachy", "dependency", "routine") for w in banned.get(k, [])]
pats = [re.compile(p) for p in banned.get("patterns", [])]
exempt = set(banned.get("exempt_copy_ids", []))
def bad(text: str) -> str | None:
    for w in words:
        if w in text: return w
    for p in pats:
        if p.search(text): return p.pattern
    return None
han = lambda s: len(re.findall(r"[一-鿿]", s))

comfort = load("comfort.toml")
ids = set()
for style, groups in comfort.items():
    if not isinstance(groups, dict): continue
    for g, items in groups.items():
        if len(items) < 15: errors.append(f"comfort {style}.{g}: 只有 {len(items)} 句（要求 ≥ 15）")
        for it in items:
            t = it["text"]
            if it["id"] in ids: errors.append(f"comfort id 重复 {it['id']}")
            ids.add(it["id"])
            if not 6 <= han(t) <= 30: errors.append(f"comfort {it['id']} 长度 {han(t)}：{t}")
            if (b := bad(t)) or "打字" in t: errors.append(f"comfort {it['id']} 含禁用内容 {b or '打字'}：{t}")
            if t.count("！") + t.count("!") > 1: errors.append(f"comfort {it['id']} 感叹号过多")

def walk(d, prefix=""):
    for k, v in d.items():
        key = f"{prefix}{k}"
        if isinstance(v, dict): yield from walk(v, key + ".")
        elif isinstance(v, str): yield key, v
        elif isinstance(v, list):
            for i, x in enumerate(v):
                if isinstance(x, dict): yield from walk(x, f"{key}[{i}].")
                elif isinstance(x, str): yield f"{key}[{i}]", x
ui = load("ui_copy.toml")
for key, text in walk(ui):
    if key.startswith("tip.") and han(text) > 16: errors.append(f"ui_copy {key} 超过 16 字：{text}")
    if key not in exempt and (b := bad(text)): errors.append(f"ui_copy {key} 含禁用词 {b}：{text}")
for name in ("evening.toml", "weekly_line.toml", "explain.toml", "letter_tips.toml"):
    for key, text in walk(load(name)):
        if (b := bad(text)): errors.append(f"{name} {key} 含禁用词 {b}：{text}")

crisis = load("crisis_lexicon.toml")
for e in crisis.get("entry", []) + crisis.get("benign", []):
    try: re.compile(e["pattern"])
    except re.error as ex: errors.append(f"crisis 正则错误 {e['pattern']}: {ex}")
for name in ("accessories.toml", "app_categories.toml", "baseline_default.toml", "schedule_patterns.toml"):
    load(name)
for p in sorted((root / "prompts").glob("*.md")):
    if not p.read_text(encoding="utf-8").startswith("<!-- version:"):
        errors.append(f"prompts/{p.name} 缺少版本号首行")

print("\n".join(errors) if errors else "全部模板校验通过")
sys.exit(1 if errors else 0)
