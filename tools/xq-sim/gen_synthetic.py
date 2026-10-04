#!/usr/bin/env python3
"""心晴 · 合成回放脚本生成器（开发与 CI 用）

用法：python3 tools/xq-sim/gen_synthetic.py [输出目录，默认 tools/xq-sim/scripts]

生成的脚本只用于开发联调和自动化测试（固定随机种子，可复现）。
它们是按规则“造”出来的打字节奏，**不是** E-STATE 评测数据：评测脚本必须按
eval/datasets/e_state_scripts.md 由组员真实录制，放在 eval/datasets/state/。
脚本中不含任何文字（commit 不带 text），符合 FR-DMO-02 的录制约定。
"""
import json
import random
import sys
from pathlib import Path

OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent / "scripts"
LETTERS = "abcdefghijklmnopqrstuvwxyz"
NEIGHBOR = {"s": "d", "d": "f", "a": "s", "i": "o", "n": "m", "e": "r", "h": "j", "g": "h", "u": "i"}


class Script:
    def __init__(self, seed: int, app: str = "WeChat.exe"):
        self.rng = random.Random(seed)
        self.t = 0
        self.lines: list[dict] = []
        self.emit({"t": "hello", "v": 1, "ime_ver": "synthetic", "session": f"synthetic-{seed}", "caps": ["core_keys"]})
        self.emit({"t": "focus", "ts": 0, "app": app, "scope": "normal", "blocked": False})

    def emit(self, m: dict) -> None:
        self.lines.append(m)

    def wait(self, ms: float) -> None:
        self.t += max(1, int(ms))

    def key(self, kind: str, vk: int | None = None, in_comp: bool = False) -> None:
        m = {"t": "key", "ts": self.t, "kind": kind, "in_comp": in_comp, "src": "core"}
        if vk is not None:
            m["vk"] = vk
        self.emit(m)

    def comp(self, op: str, length: int | None = None) -> None:
        m = {"t": "comp", "ts": self.t, "op": op}
        if length is not None:
            m["len"] = length
        self.emit(m)

    def word(self, iki: float, jitter: float, pause_mid: float = 0.0, page_flips: int = 0, typo: bool = False) -> None:
        """打一个拼音词并选词上屏。"""
        n = self.rng.randint(2, 6)
        code = [self.rng.choice(LETTERS) for _ in range(n)]
        for i, ch in enumerate(code):
            self.wait(self.rng.gauss(iki, jitter))
            self.key("letter", ord(ch.upper()), in_comp=i > 0)
            self.comp("update", i + 1)
            if typo and i == 1 and ch in NEIGHBOR:
                self.wait(self.rng.uniform(150, 350))
                self.key("backspace", 8, in_comp=True)
                self.comp("update", i)
                self.wait(self.rng.uniform(150, 400))
                self.key("letter", ord(NEIGHBOR[ch].upper()), in_comp=True)
                self.comp("update", i + 1)
            if pause_mid and i == n // 2:
                self.wait(pause_mid)
        for _ in range(page_flips):
            self.wait(self.rng.gauss(500, 100))
            self.key("nav", in_comp=True)
            self.emit({"t": "cand", "ts": self.t, "op": "page"})
        self.wait(self.rng.gauss(iki * 1.2, jitter))
        self.key("space", in_comp=True)
        pos = self.rng.choice([0, 0, 0, 1]) if not page_flips else self.rng.randint(0, 4)
        self.emit({"t": "cand", "ts": self.t, "op": "select", "pos": pos})
        self.emit({"t": "commit", "ts": self.t, "chars": self.rng.randint(1, 3), "keystrokes": n + 1, "cand_pos": pos, "src": "candidate"})

    def abandon(self, iki: float) -> None:
        """打了一半按 Esc 取消（FR-SEN-04 cancel）。"""
        for i in range(self.rng.randint(3, 5)):
            self.wait(self.rng.gauss(iki, 30))
            self.key("letter", ord(self.rng.choice(LETTERS).upper()), in_comp=i > 0)
            self.comp("update", i + 1)
        self.wait(self.rng.uniform(1200, 1800))
        self.key("esc", in_comp=True)
        self.comp("cancel")

    def sentence(self, words: int, iki: float, jitter: float, **kw) -> None:
        for _ in range(words):
            self.word(iki, jitter, **kw)
        self.wait(self.rng.gauss(300, 60))
        self.key("punct")
        self.emit({"t": "commit", "ts": self.t, "chars": 1, "keystrokes": 1, "cand_pos": -1, "src": "punctuation"})
        self.wait(self.rng.gauss(400, 80))
        self.key("enter")

    def save(self, name: str) -> None:
        OUT.mkdir(parents=True, exist_ok=True)
        path = OUT / f"{name}.jsonl"
        path.write_text("".join(json.dumps(m, ensure_ascii=False, separators=(",", ":")) + "\n" for m in self.lines), encoding="utf-8")
        print(f"{path}: {len(self.lines)} 条，时长 {self.t / 1000:.0f} 秒")


def fluent() -> None:
    s = Script(1)
    for _ in range(40):
        s.sentence(s.rng.randint(3, 6), iki=190, jitter=35)
        s.wait(s.rng.uniform(2500, 5000))
    s.save("fluent")


def hesitant() -> None:
    """流畅开头 → 犹豫段（组字中长停顿、Esc 取消、翻页找词）→ 恢复。"""
    s = Script(2)
    for _ in range(8):
        s.sentence(s.rng.randint(3, 5), iki=190, jitter=35)
        s.wait(s.rng.uniform(2500, 4000))
    for i in range(14):
        kind = i % 3
        if kind == 0:
            s.word(260, 60, pause_mid=s.rng.uniform(3500, 6000))
            s.word(260, 60, pause_mid=s.rng.uniform(3500, 6000))
            s.wait(1200)
        elif kind == 1:
            s.abandon(250)
            s.wait(s.rng.uniform(800, 1500))
            s.word(250, 50)
            s.wait(1200)
        else:
            s.word(240, 50, page_flips=s.rng.randint(3, 5))
            s.word(240, 50)
            s.wait(1200)
        s.wait(s.rng.uniform(2500, 4000))
    for _ in range(10):
        s.sentence(s.rng.randint(3, 5), iki=190, jitter=35)
        s.wait(s.rng.uniform(2500, 4000))
    s.save("hesitant")


def typo_burst() -> None:
    s = Script(3)
    for _ in range(20):
        for _ in range(s.rng.randint(3, 5)):
            s.word(170, 30, typo=s.rng.random() < 0.6)
        s.wait(1500)
        s.wait(s.rng.uniform(2500, 4000))
    s.save("typo_burst")


if __name__ == "__main__":
    fluent()
    hesitant()
    typo_burst()
