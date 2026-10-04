#!/usr/bin/env python3
"""心晴身份改造（产品书 14 §1 第 4 步，FR-OPS-06 / C-PLT-08）。

把 Windows 上会与官方清风输入法冲突的标识全部换成心晴自己的：
TSF 的 CLSID / Profile 等 GUID、命名管道、数据目录与注册表键、内核对象名、
窗口类名、显示名。目标是两者同时安装、互不干扰（TC-OPS-04）。

规则是幂等的：替换结果不会再匹配任何规则。每次合并上游后运行一次：

    python3 scripts/xinqing/rebrand.py          # 改写文件
    python3 scripts/xinqing/rebrand.py --check  # 只检查，有残留则退出码 1（CI 用）

改了规则要同步 docs/xinqing/identity.md。
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# ---------------------------------------------------------------- GUID
# 上游：{99C2DEBn-5C57-45A2-9C63-FB54B34FD90A}（dev）/ {99C2EE3n-…}（release），
# n = 0 CLSID、1 Profile、2 语言栏按钮、3/4 显示属性、5 PreservedKey。
# 心晴保留同样的“系列 + 序号”结构，只换前缀和后三段，序号含义不变。
OLD_HEAD, NEW_HEAD = "99C2", "EF62"
OLD_TAIL = "5C57-45A2-9C63-FB54B34FD90A"
NEW_TAIL = "5ECF-413A-A476-48D1F29E827C"


def _tail_bytes(tail: str) -> tuple[str, str, list[str]]:
    d2, d3, d4a, d4b = tail.split("-")
    raw = d4a + d4b
    return d2.lower(), d3.lower(), [raw[i : i + 2].lower() for i in range(0, 16, 2)]


def _c_tail(tail: str) -> str:
    d2, d3, b = _tail_bytes(tail)
    return f"0x{d2}, 0x{d3}, {{ " + ", ".join(f"0x{x}" for x in b) + " }"


@dataclass
class Rule:
    name: str
    pattern: str
    repl: str
    globs: list[str]
    exclude: list[str] = field(default_factory=list)
    flags: int = 0

    def compiled(self) -> re.Pattern[str]:
        return re.compile(self.pattern, self.flags)


WIN_RUST = ["wind_input/crates/**/*.rs", "wind_input/apps/**/*.rs"]
WIN_CPP = ["wind_tsf/**/*.cpp", "wind_tsf/**/*.h", "wind_tsf/**/*.in", "wind_tsf/**/*.rc"]
# 只服务 macOS / Linux 端、需与 Swift 侧逐字对齐的文件：心晴只做 Windows，不动它们。
NON_WINDOWS = [
    "wind_input/crates/wind-bridge/src/endpoint.rs",
    "wind_input/crates/wind-ui/src/mac_panel.rs",
]

# 只在测试里出现、与真实标识无关的 “WindInput” 字符串（测试输入或假路径），改了反而要连带改断言。
APP_NAME_EXCLUDE = [
    "wind_input/crates/wind-coordinator/src/handle_addword.rs",
    "wind_input/apps/service/src/cli_util.rs",
]

ARTIFACT_FILES = [
    "scripts/dev.ps1",
    "scripts/dev.sh",
    "scripts/remote-build.ps1",
    "scripts/lib/remote-build.sh",
    "scripts/pack-installer.sh",
    "scripts/sign.ps1",
    "scripts/build.local.example",
    "scripts/build.local.ps1.example",
    "config/app.toml",
]

RULES: list[Rule] = [
    # GUID 字符串形式（Rust、PowerShell、TOML、注册表路径）
    Rule(
        "guid-string",
        rf"\b{OLD_HEAD}(DEB|EE3)([0-9A-Fa-f])-{OLD_TAIL}",
        rf"{NEW_HEAD}\1\2-{NEW_TAIL}",
        WIN_RUST + WIN_CPP + ["scripts/dev.ps1", "scripts/ime-slot.ps1", "config/app.toml"],
        flags=re.IGNORECASE,
    ),
    # GUID C 初始化器形式（Globals.cpp）
    Rule(
        "guid-c-head",
        rf"\b0x{OLD_HEAD.lower()}(deb|ee3)([0-9a-f])\b",
        rf"0x{NEW_HEAD.lower()}\1\2",
        WIN_CPP,
    ),
    Rule("guid-c-tail", re.escape(_c_tail(OLD_TAIL)), _c_tail(NEW_TAIL), WIN_CPP),
    # 命名管道：wind_input / wind_input_push / wind_input_ctrl / wind_input*_events
    # 设置端 RPC 管道按 03 第 1 节命名为 xinqing_rpc_ctrl / xinqing_rpc{后缀}_events，须先于通用规则
    Rule(
        "pipe-rpc",
        r"pipe\\wind_input(_ctrl|\{suffix\}_ctrl|\{suffix\}_events|\{\}_events)",
        r"pipe\\xinqing_rpc\1",
        ["wind_input/crates/wind-rpc/**/*.rs"],
    ),
    Rule("pipe-rust", r"pipe\\wind_input", r"pipe\\xinqing", WIN_RUST, NON_WINDOWS),
    Rule("pipe-tsf", r'_BuildPipeName\(L"wind_input', r'_BuildPipeName(L"xinqing', WIN_CPP),
    # 数据目录、注册表键、内核对象（Local\…）、窗口类名、窗口消息名
    Rule(
        "app-name",
        r'(?<=["\\])WindInput(?=Dev\b|Dev"|"|_|[A-Z]|-|\{)',
        "XinQing",
        WIN_RUST + WIN_CPP,
        NON_WINDOWS + APP_NAME_EXCLUDE,
    ),
    # 显示名（输入法列表、语言栏提示、版本资源、提示文案）
    Rule(
        "display-name",
        "清风输入法",
        "心晴输入法",
        [
            "wind_tsf/include/Globals.h",
            "wind_tsf/src/LangBarItemButton.cpp",
            "wind_tsf/res/version.rc.in",
            "wind_input/apps/service/build.rs",
            "wind_input/crates/wind-coordinator/src/tsf_profile_name.rs",
            "wind_input/crates/wind-coordinator/src/coordinator/push_config.rs",
            "wind_input/crates/wind-coordinator/src/coordinator.rs",
            "wind_input/crates/wind-coordinator/src/handle_common_chars.rs",
            "wind_input/crates/wind-transfer/src/theme.rs",
            "config/app.toml",
        ],
    ),
    Rule("tsf-desc", r"\(WindInput(Dev)?\)", r"(XinQing\1)", ["wind_tsf/include/Globals.h"]),
    # 本地部署脚本与安装器清单
    Rule(
        "ps-app-name",
        r'(?<=["\\\'])WindInput(?=Dev\b|Dev"|"|\'|-Setup|Portable|\.app\.toml|\[Dev\])',
        "XinQing",
        ["scripts/dev.ps1"],
    ),
    Rule("ps-dir", r"(?<=Program Files\\)WindInput", "XinQing", ["scripts/dev.ps1"]),
    Rule("ps-display", r'(?<=["\'= ])清风输入法|(?<=切换到)清风输入法', "心晴输入法", ["scripts/dev.ps1"]),
    Rule("ps-proto", r'"windinput(dev)?"', r'"xinqing\1"', ["scripts/dev.ps1", "config/app.toml"]),
    # 产物名（03 第 5.1 节）：wind_tsf[_x86][_dev].dll → xinqing_tsf…，wind_input[_dev].exe → xinqing_core…。
    # cargo 的 bin 名与 crate 名不动，构建脚本把 target 里的 wind_input.exe 复制成 xinqing_core.exe，
    # 所以 “$prof\wind_input.exe” 这类指向 cargo 产物的路径要排除。
    Rule(
        "artifact-dll",
        r"\bwind_tsf(?=\$|_x86|_dev|\*\.dll|\.dll|\[)",
        "xinqing_tsf",
        ARTIFACT_FILES + ["wind_tsf/Makefile"],
    ),
    Rule("artifact-dll-cmake", r'TSF_TARGET_NAME "wind_tsf', 'TSF_TARGET_NAME "xinqing_tsf', ["wind_tsf/CMakeLists.txt"]),
    Rule(
        "artifact-dll-meta",
        r'"wind_tsf(\.dll)?"',
        r'"xinqing_tsf\1"',
        ["wind_tsf/res/version.rc.in", "wind_tsf/wind_tsf.def"],
    ),
    Rule(
        "artifact-exe",
        r'(?<!\$prof\\)(?<!\$prof/)(?<![A-Za-z_])wind_input(?=\$suffix|\$\{suffix\}|\$sfx|\$\{sfx\}|_dev\.exe|_dev"|\.exe|\[_dev\]|\*\.exe|"\])',
        "xinqing_core",
        ARTIFACT_FILES + ["wind_input/scripts/wind_cli.bat", "wind_tsf/include/Globals.h", "wind_input/apps/service/build.rs"],
    ),
    Rule("artifact-exe-meta", r'"InternalName", "wind_input"', '"InternalName", "xinqing_core"', ["wind_input/apps/service/build.rs"]),
    # 托盘菜单“设置”按 exe 名推导设置程序；心晴的设置页在 Hub 里
    Rule(
        "settings-app",
        r'stem\.replacen\("wind_input", "wind_setting", 1\)',
        'stem.replacen("xinqing_core", "xinqing_hub", 1)',
        ["wind_input/crates/wind-coordinator/src/coordinator.rs"],
    ),
    Rule(
        "app-toml",
        r'(?<=["/])WindInput(?=")',
        "XinQing",
        ["config/app.toml"],
    ),
]

# 改造后不应再出现的残留（--check 时检查，按文件范围）。
LEFTOVERS = [
    (
        rf"(?i:{OLD_TAIL}|0x{OLD_HEAD}(deb|ee3))",
        WIN_RUST + WIN_CPP + ["scripts/*.ps1", "config/app.toml"],
        NON_WINDOWS,
    ),
    (r"pipe\\wind_input|_BuildPipeName\(L\"wind_input", WIN_RUST + WIN_CPP, NON_WINDOWS),
    (
        r'(?<=["\\])WindInput(?=Dev\b|Dev"|"|_|[A-Z]|-|\{)',
        WIN_RUST + WIN_CPP,
        NON_WINDOWS + APP_NAME_EXCLUDE,
    ),
    (r'L"清风输入法|"清风输入法', ["wind_tsf/**/*.h", "wind_tsf/**/*.cpp"], []),
    (r"\bwind_tsf(?=\$|_x86|_dev|\.dll)", ARTIFACT_FILES + ["wind_tsf/Makefile"], []),
    (
        r'(?<!\$prof\\)(?<!\$prof/)(?<![A-Za-z_])wind_input(?=\$suffix|\$\{suffix\}|\$sfx|\$\{sfx\}|_dev\.exe|\.exe)',
        ARTIFACT_FILES + ["wind_tsf/include/Globals.h"],
        [],
    ),
]


def files_for(globs: list[str], exclude: list[str]) -> list[Path]:
    out: set[Path] = set()
    for g in globs:
        for p in ROOT.glob(g):
            if p.is_file() and "/target/" not in p.as_posix():
                out.add(p)
    ex = {ROOT / e for e in exclude}
    return sorted(p for p in out if p not in ex)


def apply(check: bool) -> int:
    changed: dict[Path, list[str]] = {}
    contents: dict[Path, str] = {}
    for rule in RULES:
        rx = rule.compiled()
        for p in files_for(rule.globs, rule.exclude):
            text = contents.get(p)
            if text is None:
                text = p.read_text(encoding="utf-8")
            new, n = rx.subn(rule.repl, text)
            if n:
                changed.setdefault(p, []).append(f"{rule.name}×{n}")
            contents[p] = new

    if check:
        bad = 0
        for p, rules in changed.items():
            print(f"未改造: {p.relative_to(ROOT)} ({', '.join(rules)})")
            bad += 1
        for pat, globs, exclude in LEFTOVERS:
            rx = re.compile(pat)
            for p in files_for(globs, exclude):
                for i, line in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
                    if rx.search(line):
                        print(f"残留: {p.relative_to(ROOT)}:{i}: {line.strip()[:120]}")
                        bad += 1
        if bad:
            print("\n请运行 python3 scripts/xinqing/rebrand.py 后提交；规则说明见 docs/xinqing/identity.md")
            return 1
        print("身份改造检查通过")
        return 0

    for p, rules in changed.items():
        p.write_text(contents[p], encoding="utf-8")
        print(f"{p.relative_to(ROOT)}: {', '.join(rules)}")
    if not changed:
        print("无需改动")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true", help="只检查不改写")
    return apply(ap.parse_args().check)


if __name__ == "__main__":
    sys.exit(main())
