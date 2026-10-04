#!/usr/bin/env python3
"""心晴 · XQP 回放脚本契约校验（CI contracts 任务，14 第 5、6 节）

用法：python3 tools/check_xqp_scripts.py [脚本文件或目录 ...]（默认 tools/xq-sim/scripts）
每一行必须通过 protocol/xqp.schema.json 中上行消息（#/$defs/up）的校验，且 ts 不倒退。
依赖：pip install jsonschema
"""
import json
import sys
from pathlib import Path

from jsonschema import Draft202012Validator
from referencing import Registry, Resource

root = Path(__file__).resolve().parent.parent
schema = json.loads((root / "protocol" / "xqp.schema.json").read_text(encoding="utf-8"))
Draft202012Validator.check_schema(schema)
registry = Registry().with_resource(schema["$id"], Resource.from_contents(schema))
up = Draft202012Validator({"$ref": schema["$id"] + "#/$defs/up"}, registry=registry)

targets = [Path(a) for a in sys.argv[1:]] or [root / "tools" / "xq-sim" / "scripts"]
files = sorted(p for t in targets for p in ([t] if t.is_file() else t.glob("*.jsonl")))
errors = 0
for f in files:
    last = 0
    for n, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        msg = json.loads(line)
        errs = list(up.iter_errors(msg))
        if errs:
            errors += 1
            print(f"{f}:{n}: {errs[0].message}")
        ts = msg.get("ts")
        if isinstance(ts, int):
            if ts < last:
                errors += 1
                print(f"{f}:{n}: ts 倒退 {ts} < {last}")
            last = ts
    print(f"{f.relative_to(root) if f.is_relative_to(root) else f}: 已检查")
if not files:
    print("没有找到脚本"); sys.exit(1)
sys.exit(1 if errors else 0)
