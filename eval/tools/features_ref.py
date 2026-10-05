#!/usr/bin/env python3
"""心晴 · 窗口切分、特征计算与个人基线的 Python 参考实现（FR-STA-01～03），用于与 Rust 对拍。

按产品书 04 FR-STA-01～03 与 ADR 0008 的解释独立实现，不调用 Hub 代码。对拍内容：
1. 窗口特征：回放 tools/xq-sim/scripts 的合成脚本，以及本文件 `cases()` 中的 10 组固定事件序列
   （FR-STA-02 验收：“对 10 组固定事件序列，特征计算结果与手算值误差 < 1%”）；
2. 个人基线：确定性生成 7 天的窗口特征样本，按白天 / 夜间分桶算中位数与 MAD
   （FR-STA-03 验收：“注入 7 天模拟数据后，基线与离线 Python 脚本计算结果一致”）。

用法（仓库根目录，只用标准库）：
  python3 eval/tools/features_ref.py --write        # 重新生成下面三个文件
  python3 eval/tools/features_ref.py --out DIR      # 写到 DIR（CI 用来与仓库里的文件比较）
生成：eval/datasets/e_features_cases.jsonl、e_features.golden.json、e_baseline.golden.json
Rust 对拍：xinqing_hub/core/tests/features_parity.rs
"""
import argparse
import json
import math
import random
import statistics
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = ["fluent", "hesitant", "typo_burst"]
# 回放时会话时间 0 对应的本地时刻（分钟，0–1439）；脚本统一从 14:00 开始
DEFAULT_START_MIN = 14 * 60

# ---- FR-STA-01 窗口切分（含 ADR 0008 第 1 条：组字中的停顿不切窗）----
COMMIT_IDLE_MS = 1_000
PAUSE_MS = 2_000
MAX_WINDOW_MS = 30_000
MIN_KEYS = 5
SMALL_MERGE_MS = 60_000
TICK_MS = 250  # 17 第 2.3 节：Hub 每 250 ms 推进一次

# ---- FR-STA-02 特征 ----
GAP_MS = 2_000
COMP_PAUSE_MS = 3_000
SESSION_BREAK_MS = 120_000

# ---- FR-STA-03 基线 ----
BASE_FEATURES = ["kpm", "iki_med", "iki_iqr", "bs_rate", "dwell_med"]
COLD_START_WINDOWS = 200
MAD_TO_SD = 1.4826
Z_CLIP = 5.0
MIN_BUCKET_SAMPLES = 30

# ---- FR-STA-04 R1 ----
BACKSPACE_WITHIN_MS = 600
RETYPE_WITHIN_MS = 800
TYPO_DEDUP_MS = 1_000
ROWS = ["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"]
ROW_OFFSET_HALF = [0, 1, 3]  # 每行相对上一行的错位，以半键为单位


def key_pos(vk):
    c = chr(vk).upper()
    for r, row in enumerate(ROWS):
        i = row.find(c)
        if i >= 0:
            return r, i * 2 + ROW_OFFSET_HALF[r]
    return None


def adjacent(a, b):
    pa, pb = key_pos(a), key_pos(b)
    if pa is None or pb is None or chr(a).upper() == chr(b).upper():
        return False
    dr, dx = abs(pa[0] - pb[0]), abs(pa[1] - pb[1])
    return (dr == 0 and dx == 2) or (dr == 1 and dx <= 2)


class Typo:
    """字母 X → 600 ms 内退格 → 800 ms 内相邻字母 Y（Y ≠ X），1 秒内最多一次。"""

    def __init__(self):
        self.last_letter = None  # (ts, vk)
        self.bs_after = None  # (ts, x)
        self.last_fire = None

    def on_key(self, ts, kind, vk):
        if kind == "backspace":
            ll = self.last_letter
            self.bs_after = (ts, ll[1]) if ll and ts - ll[0] <= BACKSPACE_WITHIN_MS else None
            self.last_letter = None
            return False
        if kind == "letter" and vk is not None:
            hit = False
            if self.bs_after is not None:
                tb, x = self.bs_after
                hit = ts - tb <= RETYPE_WITHIN_MS and adjacent(x, vk)
            self.bs_after = None
            self.last_letter = (ts, vk)
            if hit and (self.last_fire is None or ts - self.last_fire >= TYPO_DEDUP_MS):
                self.last_fire = ts
                return True
            return False
        self.last_letter = None
        self.bs_after = None
        return False


def key_src(win):
    """窗口里有 tsf 按键（FR-SEN-08）时按键统计只用 tsf，避免与 core 重复（ADR 0008 第 6 条）。"""
    return "tsf" if any(e["t"] == "key" and e["src"] == "tsf" for e in win) else "core"


def selected_keys(win):
    src = key_src(win)
    return [e for e in win if e["t"] == "key" and e["src"] == src and e["kind"] != "other"]


class Cutter:
    def __init__(self):
        self.cur = None
        self.pending = None
        self.last_key = None
        self.commit_at = None
        self.in_comp = False
        self.last_app = None

    def close(self):
        w, self.cur = self.cur, None
        if w is None:
            return None
        self.commit_at = None
        p, self.pending = self.pending, None
        if p is not None and w[0]["ts"] - p[-1]["ts"] <= SMALL_MERGE_MS:
            w = p + w
        if len(selected_keys(w)) < MIN_KEYS:
            self.pending = w  # 少于 5 键：合并到下一个窗口（FR-STA-01）
            return None
        return w

    def discard(self):
        self.cur = self.pending = self.last_key = self.commit_at = None
        self.in_comp = False

    def push(self, ev):
        t = ev["t"]
        if t == "key":
            out = None
            if self.cur is not None:
                ts = ev["ts"]
                if self.commit_at is not None and ts - self.commit_at >= COMMIT_IDLE_MS:
                    out = self.close()
                elif self.last_key is not None and not self.in_comp and ts - self.last_key >= PAUSE_MS:
                    out = self.close()
                elif ts - self.cur[0]["ts"] >= MAX_WINDOW_MS:
                    out = self.close()
            if self.cur is None:
                self.cur = []
            self.cur.append(ev)
            self.last_key = ev["ts"]
            self.commit_at = None
            return out
        if t in ("key_up", "cand"):
            if self.cur is not None:
                self.cur.append(ev)
            return None
        if t == "comp":
            self.in_comp = ev["op"] == "update" and ev.get("len", 1) > 0
            if self.cur is not None:
                self.cur.append(ev)
            return None
        if t == "commit":
            self.in_comp = False
            if self.cur is not None:
                self.cur.append(ev)
                self.commit_at = ev["ts"]
            return None
        if t == "focus":
            app = None if ev["blocked"] else ev.get("app")
            changed = app != self.last_app
            self.last_app = app
            if changed:
                self.in_comp = False
                return self.close()
            return None
        if t == "ime" and not ev["active"]:
            self.in_comp = False
            return self.close()
        if t == "pause_changed" and ev["on"]:
            self.discard()
        return None

    def tick(self, now):
        if self.pending is not None and self.cur is None and now - self.pending[-1]["ts"] > SMALL_MERGE_MS:
            self.pending = None
        if self.cur is None:
            return None
        if self.commit_at is not None and now - self.commit_at >= COMMIT_IDLE_MS:
            return self.close()
        if self.last_key is not None and not self.in_comp and now - self.last_key >= PAUSE_MS:
            return self.close()
        if now - self.cur[0]["ts"] >= MAX_WINDOW_MS:
            return self.close()
        return None


def quantile(xs, q):
    """线性插值分位数（numpy 默认 linear）。"""
    if not xs:
        return None
    s = sorted(xs)
    pos = q * (len(s) - 1)
    lo, hi = math.floor(pos), math.ceil(pos)
    return s[lo] + (s[hi] - s[lo]) * (pos - lo)


class Baseline:
    def __init__(self, defaults):
        self.defaults = defaults  # {(bucket, feature): (med, mad)}
        self.windows = 0

    def z(self, bucket, feature, x):
        if x is None or (bucket, feature) not in self.defaults:
            return None
        med, mad = self.defaults[(bucket, feature)]  # 对拍只走冷启动（窗口数 < 200）
        denom = max(MAD_TO_SD * mad, MAD_TO_SD * mad * 0.1)
        return max(-Z_CLIP, min(Z_CLIP, (x - med) / denom))


def bucket_of(hour):
    return "day" if 6 <= hour < 22 else "night"


def compute(win, base, session_min, start_min):
    src = key_src(win)
    tsf = src == "tsf"
    keys, in_comp_at, ups, cand_pos = [], [], [], []
    comp_active = abandon = False
    page_flips = 0
    for e in win:
        t = e["t"]
        if t == "key":
            if e["src"] == src and e["kind"] != "other":
                keys.append(e)
                in_comp_at.append(e["in_comp"] or comp_active)
        elif t == "key_up":
            ups.append(e)
        elif t == "comp":
            comp_active = e["op"] == "update" and e.get("len", 1) > 0
            if e["op"] in ("cancel", "clear"):
                abandon = True
        elif t == "cand":
            if e["op"] == "page":
                page_flips += 1
            elif "pos" in e:
                cand_pos.append(float(e["pos"]))
        elif t == "commit":
            comp_active = False

    n = len(keys)
    ikis, gap_sum, pause_cnt = [], 0, 0
    for i in range(n - 1):
        d = keys[i + 1]["ts"] - keys[i]["ts"]
        if d <= GAP_MS:
            ikis.append(float(d))
        else:
            gap_sum += d
        if d > COMP_PAUSE_MS and in_comp_at[i]:
            pause_cnt += 1
    span = keys[-1]["ts"] - keys[0]["ts"] if keys else 0
    active_ms = span - gap_sum
    kpm = n / active_ms * 60_000 if active_ms > 0 else None
    q1, q3 = quantile(ikis, 0.25), quantile(ikis, 0.75)

    bs = run = burst = 0
    typo = Typo()
    typo_cnt = del_committed = 0
    for k in keys:
        if k["kind"] == "backspace":
            bs += 1
            run += 1
            burst = max(burst, run)
        else:
            run = 0
        if typo.on_key(k["ts"], k["kind"], k.get("vk")):
            typo_cnt += 1
        if k.get("del_committed") is True:
            del_committed += 1

    dwell = None
    if tsf:
        used = [False] * len(ups)
        dws = []
        for k in keys:
            if k.get("vk") is None:
                continue
            for i, u in enumerate(ups):
                if not used[i] and u.get("vk") == k["vk"] and u["ts"] >= k["ts"]:
                    used[i] = True
                    dws.append(float(u["ts"] - k["ts"]))
                    break
        dwell = quantile(dws, 0.5)

    end_ts = win[-1]["ts"]
    minute_of_day = (start_min + end_ts // 60_000) % 1440
    hour = minute_of_day // 60
    b = bucket_of(hour)
    f = {
        "n_keys": n,
        "active_ms": active_ms,
        "kpm": kpm,
        "iki_med": quantile(ikis, 0.5),
        "iki_iqr": None if q1 is None else q3 - q1,
        "dwell_med": dwell,
        "bs_rate": bs / n if n else 0.0,
        "bs_burst_max": burst,
        "typo_cnt": typo_cnt,
        "pause_cnt": pause_cnt,
        "abandon": int(abandon),
        "delete_committed": del_committed if tsf else None,
        "page_flips": page_flips,
        "cand_pos_mean": sum(cand_pos) / len(cand_pos) if cand_pos else None,
        "session_min": session_min,
        "hour": hour,
        "minute_of_day": minute_of_day,
    }
    for name in BASE_FEATURES:
        f[name + "_z"] = base.z(b, name, f[name])
    return f


def replay(events, base, start_min):
    """与 Hub 的 `pipeline::replay` 同一推进方式：每条事件前按 250 ms 补 tick，最后再推进 60 秒。"""
    cutter = Cutter()
    sess_start = sess_last = None
    out = []

    def finish(w):
        if w is None:
            return
        session_min = 0.0 if sess_start is None else (sess_last - sess_start) / 60_000
        f = compute(w, base, session_min, start_min)
        base.windows += 1
        out.append({"start_ts": w[0]["ts"], "end_ts": w[-1]["ts"], "features": f})

    last_ts = 0
    for ev in events:
        ts = ev.get("ts")
        if ts is not None:
            t = last_ts + TICK_MS
            while t < ts:
                finish(cutter.tick(t))
                t += TICK_MS
            last_ts = ts
        if ev["t"] == "key":
            # 连续输入时长：相邻按键间隔 < 2 分钟算连续（FR-STA-02 session_min）
            if sess_last is None or ev["ts"] - sess_last >= SESSION_BREAK_MS:
                sess_start = ev["ts"]
            sess_last = ev["ts"]
        finish(cutter.push(ev))
    finish(cutter.tick(last_ts + 60_000))
    return out


def load_defaults():
    d = tomllib.loads((ROOT / "hub_templates" / "baseline_default.toml").read_text(encoding="utf-8"))
    return {(b, f): (d[b][f]["med"], d[b][f]["mad"]) for b in ("day", "night") for f in BASE_FEATURES if f in d.get(b, {})}


# ---------------------------------------------------------------------------
# 10 组固定事件序列（每组覆盖 FR-STA-01/02 的一类规则）
# ---------------------------------------------------------------------------

def k(ts, kind="letter", vk=None, in_comp=False, src="core", **kw):
    e = {"t": "key", "ts": ts, "kind": kind, "in_comp": in_comp, "src": src}
    if vk is not None:
        e["vk"] = vk
    e.update(kw)
    return e


def up(ts, vk, kind="letter"):
    return {"t": "key_up", "ts": ts, "kind": kind, "vk": vk}


def comp(ts, op="update", length=None):
    e = {"t": "comp", "ts": ts, "op": op}
    if length is not None:
        e["len"] = length
    return e


def commit(ts, chars=2, keystrokes=4, cand_pos=0):
    return {"t": "commit", "ts": ts, "chars": chars, "keystrokes": keystrokes, "cand_pos": cand_pos, "src": "candidate"}


def focus(ts, app, blocked=False):
    return {"t": "focus", "ts": ts, "app": app, "scope": "normal", "blocked": blocked}


def hello(caps=("core_keys",)):
    return {"t": "hello", "v": 1, "ime_ver": "parity", "session": "parity", "caps": list(caps)}


def letters(t0, step, word, comp_from=None):
    """按 `step` 毫秒一个字母打出 `word`；`comp_from` 起算组字中（拼音）。"""
    evs = []
    for i, c in enumerate(word):
        ts = t0 + i * step
        inc = comp_from is not None and i > comp_from
        evs.append(k(ts, vk=ord(c), in_comp=inc))
        if comp_from is not None and i >= comp_from:
            evs.append(comp(ts, "update", i - comp_from + 1))
    return evs


def cases():
    cs = []

    # 1 规格例：流畅拼音，上屏后 1 秒切窗，两个窗口
    e = [hello(), focus(0, "WeChat.exe")]
    e += letters(100, 150, "NIHAO", comp_from=0)
    e += [k(850, "space", in_comp=True), {"t": "cand", "ts": 850, "op": "select", "pos": 0}, commit(850)]
    e += letters(2000, 160, "WOMEN", comp_from=0)
    e += [k(2800, "space", in_comp=True), {"t": "cand", "ts": 2800, "op": "select", "pos": 1}, commit(2800)]
    cs.append(("c01_commit_cut", DEFAULT_START_MIN, e))

    # 2 停顿 ≥ 2 秒（不在组字中）切窗；> 2 秒的间隔不计入 active_ms
    e = [hello(), focus(0, "notepad.exe")]
    e += [k(100 + i * 200, "digit") for i in range(8)]
    e += [k(4000 + i * 180, "digit") for i in range(8)]
    cs.append(("c02_pause_cut", DEFAULT_START_MIN, e))

    # 3 组字中的长停顿（> 3 秒）不切窗、计入 pause_cnt（ADR 0008 第 1 条），并从 active_ms 扣除
    e = [hello(), focus(0, "WeChat.exe")]
    e += letters(100, 150, "ZHE", comp_from=0)
    e += [k(4000, vk=ord("G"), in_comp=True), comp(4000, "update", 4)]
    e += [k(7600, vk=ord("E"), in_comp=True), comp(7600, "update", 5)]
    e += [k(7800, "space", in_comp=True), commit(7800)]
    cs.append(("c03_comp_pause", DEFAULT_START_MIN, e))

    # 4 打了又删：组字取消（abandon）、成串退格（bs_burst_max）
    e = [hello(), focus(0, "WeChat.exe")]
    e += letters(100, 140, "WOXIANG", comp_from=0)
    e += [k(1100, "esc", in_comp=True), comp(1100, "cancel")]
    e += [k(1300 + i * 90, "backspace", vk=8) for i in range(6)]
    e += letters(2000, 150, "SUANLE", comp_from=0)
    e += [k(2900, "space", in_comp=True), commit(2900)]
    cs.append(("c04_abandon_backspace", DEFAULT_START_MIN, e))

    # 5 翻页选词：page_flips 与 cand_pos_mean
    e = [hello(), focus(0, "WINWORD.EXE")]
    e += letters(100, 150, "SHIJIAN", comp_from=0)
    e += [k(1200, "nav", in_comp=True), {"t": "cand", "ts": 1200, "op": "page"}]
    e += [k(1500, "nav", in_comp=True), {"t": "cand", "ts": 1500, "op": "page"}]
    e += [k(1800, "nav", in_comp=True), {"t": "cand", "ts": 1800, "op": "page"}]
    e += [k(2100, "digit", in_comp=True), {"t": "cand", "ts": 2100, "op": "select", "pos": 7}, commit(2100, cand_pos=7)]
    e += letters(3000, 150, "HAO", comp_from=0)
    e += [k(3500, "space", in_comp=True), {"t": "cand", "ts": 3500, "op": "select", "pos": 0}, commit(3500)]
    cs.append(("c05_page_flips", DEFAULT_START_MIN, e))

    # 6 打错字 R1：S→退格→D（相邻）两次，间隔 > 1 秒；中间一次 S→退格→P（不相邻）
    e = [hello(), focus(0, "Code.exe")]
    t = 100
    for x, y in [("S", "D"), ("Q", "P"), ("F", "G")]:
        e += [k(t, vk=ord(x)), k(t + 200, "backspace", vk=8), k(t + 500, vk=ord(y)), k(t + 700, vk=ord("A"))]
        t += 1300
    cs.append(("c06_typo_r1", DEFAULT_START_MIN, e))

    # 7 FR-SEN-08：同一窗口有 core 与 tsf，只用 tsf；按下—抬起算 dwell，删已上屏计数
    e = [hello(("core_keys", "tsf_trace")), focus(0, "chrome.exe")]
    t = 100
    for i, c in enumerate("HELLOWORLD"):
        vk = ord(c)
        e += [k(t, vk=vk, src="core"), k(t, vk=vk, src="tsf"), up(t + 80 + i * 3, vk)]
        t += 170
    for i in range(6):
        e += [k(t, "backspace", vk=8, src="tsf", del_committed=True), up(t + 60, 8, "backspace")]
        t += 120
    cs.append(("c07_tsf_trace", DEFAULT_START_MIN, e))

    # 8 小于 5 键的窗口合并到 60 秒内的下一个窗口；超过 60 秒则丢弃
    e = [hello(), focus(0, "notepad.exe")]
    e += [k(100 + i * 200, "digit") for i in range(3)]          # 3 键，停顿后暂存
    e += [k(5000 + i * 200, "digit") for i in range(4)]         # 4 键，与上一段合并为 7 键
    e += [k(9000 + i * 200, "digit") for i in range(2)]         # 2 键，暂存
    e += [k(80_000 + i * 200, "digit") for i in range(6)]       # 70 秒后：暂存已丢弃，单独成窗
    cs.append(("c08_small_merge", DEFAULT_START_MIN, e))

    # 9 焦点切换与输入法停用切窗；窗口最长 30 秒
    e = [hello(), focus(0, "WeChat.exe")]
    e += [k(100 + i * 150, "punct") for i in range(10)]
    e += [focus(1700, "WINWORD.EXE")]
    e += [k(1800 + i * 150, "punct") for i in range(8)]
    e += [{"t": "ime", "ts": 3100, "active": False, "chinese": False}]
    e += [{"t": "ime", "ts": 3200, "active": True, "chinese": True}]
    e += [k(3300 + i * 1500, "digit") for i in range(25)]       # 每 1.5 秒一键，约 36 秒，30 秒处切开
    cs.append(("c09_focus_ime_max", DEFAULT_START_MIN, e))

    # 10 深夜时段（夜间基线桶）与跨过 2 分钟的连续输入时长
    e = [hello(), focus(0, "WeChat.exe")]
    e += [k(100 + i * 200, "letter", vk=ord("A")) for i in range(10)]
    e += [k(60_000 + i * 200, "letter", vk=ord("B")) for i in range(10)]   # 间隔 < 2 分钟：连续
    e += [k(200_000 + i * 220, "letter", vk=ord("C")) for i in range(10)]  # 间隔 > 2 分钟：重新计
    cs.append(("c10_night_session", 23 * 60 + 40, e))
    return cs


# ---------------------------------------------------------------------------
# 7 天基线样本（FR-STA-03 验收）
# ---------------------------------------------------------------------------

def baseline_samples():
    rng = random.Random(20261005)
    out = []
    for day in range(7):
        for _ in range(rng.randint(60, 90)):
            hour = rng.choice(range(8, 22))
            out.append(sample(rng, hour, day))
        for _ in range(rng.randint(1, 3)):  # 夜里打字很少：夜间桶样本 < 30，继续用人群默认值
            out.append(sample(rng, rng.choice([22, 23, 0, 1]), day))
    return out


def sample(rng, hour, day):
    kpm = round(rng.gauss(210, 40), 3)
    iki = round(rng.gauss(200, 35), 3)
    return {
        "day": day,
        "hour": hour,
        "kpm": kpm,
        "iki_med": iki,
        "iki_iqr": round(abs(rng.gauss(150, 40)), 3),
        "bs_rate": round(min(1.0, abs(rng.gauss(0.08, 0.03))), 5),
        "dwell_med": None,  # 未启用 FR-SEN-08：null 不计入样本
    }


def baseline_stats(samples):
    groups = {}
    for s in samples:
        b = bucket_of(s["hour"])
        for f in BASE_FEATURES:
            v = s[f]
            if v is not None:
                groups.setdefault((b, f), []).append(v)
    rows = []
    for (b, f), vs in sorted(groups.items()):
        if len(vs) < MIN_BUCKET_SAMPLES:
            continue
        med = statistics.median(vs)
        mad = statistics.median([abs(v - med) for v in vs])
        rows.append({"bucket": b, "feature": f, "med": med, "mad": mad, "n": len(vs)})
    return {"windows": len(samples), "rows": rows}


def build():
    defaults = load_defaults()
    golden = {}
    for name in SCRIPTS:
        lines = (ROOT / "tools" / "xq-sim" / "scripts" / f"{name}.jsonl").read_text(encoding="utf-8").splitlines()
        events = [json.loads(l) for l in lines if l.strip()]
        golden[name] = {"start_min": DEFAULT_START_MIN, "windows": replay(events, Baseline(defaults), DEFAULT_START_MIN)}
    case_lines = []
    for cid, start_min, events in cases():
        case_lines.append(json.dumps({"id": cid, "start_min": start_min, "events": events}, ensure_ascii=False))
        golden[cid] = {"start_min": start_min, "windows": replay(events, Baseline(defaults), start_min)}
    samples = baseline_samples()
    base = {"samples": samples, "expected": baseline_stats(samples)}
    return "\n".join(case_lines) + "\n", golden, base


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--write", action="store_true", help="写入 eval/datasets/")
    g.add_argument("--out", type=Path, help="写到指定目录（CI 比较用）")
    a = ap.parse_args()
    out = ROOT / "eval" / "datasets" if a.write else a.out
    out.mkdir(parents=True, exist_ok=True)
    cases_text, golden, base = build()
    (out / "e_features_cases.jsonl").write_text(cases_text, encoding="utf-8")
    (out / "e_features.golden.json").write_text(json.dumps(golden, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    (out / "e_baseline.golden.json").write_text(json.dumps(base, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    n = sum(len(v["windows"]) for v in golden.values())
    print(f"{len(golden)} 组事件序列、{n} 个窗口；基线样本 {len(base['samples'])} 个、{len(base['expected']['rows'])} 项统计", file=sys.stderr)


if __name__ == "__main__":
    main()
