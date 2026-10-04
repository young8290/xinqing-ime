#!/usr/bin/env python3
"""生成心晴的占位图标（产品书 14 §1 第 4 步“替换图标”）。

输出三份品牌资产（规格见 assets/README.md）：
  wind_tsf/res/wind_input.ico  TSF DLL 与核心 exe 的图标，10 层 16→256
  assets/installer.ico         安装程序图标，同上
  assets/logo.png              安装向导 logo，144×144

图案是暖色圆角方块上的白色“晴”字，只是占位，正式图标由设计稿替换后重新生成
或直接覆盖这三个文件。需要 Pillow 和一款含“晴”字的中文字体：

    python3 scripts/xinqing/gen_icons.py --font /usr/share/fonts/truetype/wqy/wqy-zenhei.ttc
"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
TOP = (255, 196, 92)  # 晨光琥珀
BOTTOM = (240, 128, 96)  # 暖珊瑚
SS = 4  # 超采样倍数


def render(size: int, font_path: str) -> Image.Image:
    big = size * SS
    grad = Image.new("RGB", (big, big))
    px = grad.load()
    for y in range(big):
        for x in range(big):
            t = (x + y) / (2 * (big - 1))
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=round(big * 0.22), fill=255
    )
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)

    # 16/20 像素下笔画太密，字放大到几乎满幅以保留轮廓
    scale = 0.86 if size <= 20 else 0.72
    font = ImageFont.truetype(font_path, round(big * scale))
    d = ImageDraw.Draw(img)
    l, t, r, b = d.textbbox((0, 0), "晴", font=font)
    d.text(((big - (r - l)) / 2 - l, (big - (b - t)) / 2 - t), "晴", font=font, fill="white")

    # 预乘 alpha 后缩放，避免透明区的黑色渗进圆角边缘
    return img.convert("RGBa").resize((size, size), Image.LANCZOS).convert("RGBA")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--font", required=True, help="含“晴”字的 TTF/TTC 字体路径")
    font = ap.parse_args().font

    layers = [render(s, font) for s in ICO_SIZES]
    biggest = layers[-1]
    for out in ["wind_tsf/res/wind_input.ico", "assets/installer.ico"]:
        biggest.save(
            ROOT / out, format="ICO", sizes=[(s, s) for s in ICO_SIZES], append_images=layers[:-1]
        )
        print(out)
    render(144, font).save(ROOT / "assets/logo.png")
    print("assets/logo.png")


if __name__ == "__main__":
    main()
