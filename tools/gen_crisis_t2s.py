#!/usr/bin/env python3
"""心晴 · 生成危机词表用的繁转简字表（hub_templates/crisis_t2s.toml）

用法：pip install opencc-python-reimplemented && python3 tools/gen_crisis_t2s.py [--check]
只收录“转换结果出现在 crisis_lexicon.toml 中”的繁体字，保证词表里的每个字都能从繁体写法匹配到。
修改 crisis_lexicon.toml 后必须重新生成；CI 用 --check 确认文件是最新的。
字表数据来自 OpenCC（Apache-2.0）的 TSCharacters.txt。
"""
import os
import sys
from pathlib import Path

import opencc

root = Path(__file__).resolve().parent.parent
lex = (root / "hub_templates" / "crisis_lexicon.toml").read_text(encoding="utf-8")
used = {c for c in lex if "一" <= c <= "鿿"}
src = Path(os.path.dirname(opencc.__file__)) / "dictionary" / "TSCharacters.txt"
pairs: dict[str, str] = {}
for line in src.read_text(encoding="utf-8").splitlines():
    if not line.strip():
        continue
    trad, simp = line.split("\t")
    simp = simp.split(" ")[0]
    if trad != simp and len(trad) == 1 and simp in used:
        pairs[trad] = simp
# OpenCC 按词组转换、单字表不收的常见写法；危机识别宁可多报，按字补上
EXTRA = {"著": "着"}
pairs.update({t: v for t, v in EXTRA.items() if v in used})
body = "".join(f'"{t}" = "{s}"\n' for t, s in sorted(pairs.items()))
out = (
    "# 心晴 · 危机词表繁转简字表（由 tools/gen_crisis_t2s.py 生成，不要手改）\n"
    "# 只含转换结果出现在 crisis_lexicon.toml 中的字；数据来自 OpenCC TSCharacters（Apache-2.0）。\n"
    "# Hub safety 模块与 eval/tools/check_crisis.py 共用本表（判定算法第 1 步“繁转简”）。\n\n"
    "[map]\n" + body
)
path = root / "hub_templates" / "crisis_t2s.toml"
if "--check" in sys.argv:
    if path.read_text(encoding="utf-8") != out:
        print("crisis_t2s.toml 不是最新的，请运行 python3 tools/gen_crisis_t2s.py"); sys.exit(1)
    print("crisis_t2s.toml 是最新的"); sys.exit(0)
path.write_text(out, encoding="utf-8")
print(f"{path}: {len(pairs)} 个字")
