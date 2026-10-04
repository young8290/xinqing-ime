#!/usr/bin/env python3
"""生成心晴 Hub 的占位图标（07 DS-BRAND-02：一朵圆润的云后面露出半个太阳）。

输出到 xinqing_hub/src-tauri/icons/：32x32.png、128x128.png、128x128@2x.png、icon.png（512）、
icon.ico（16→256 多层）。只是几何占位，正式设计稿到位后直接覆盖这些文件即可。需要 Pillow：

    python3 tools/gen_hub_icons.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parents[1] / "xinqing_hub" / "src-tauri" / "icons"
SUN = (246, 195, 68, 255)  # --xq-w-sunny
CLOUD = (255, 255, 255, 255)
OUTLINE = (107, 102, 94, 255)  # --xq-text-2，保证浅色任务栏上也看得见云的轮廓
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
SS = 8  # 超采样倍数


def cloud_shapes(u: float) -> list[tuple[float, float, float, float]]:
    """云由三个圆和一个圆角底座组成，坐标以 u（画布的 1/100）为单位。"""
    return [
        (14 * u, 50 * u, 50 * u, 86 * u),  # 左圆
        (30 * u, 34 * u, 74 * u, 78 * u),  # 中间高圆
        (54 * u, 50 * u, 88 * u, 84 * u),  # 右圆
    ]


def render(size: int) -> Image.Image:
    big = size * SS
    u = big / 100
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.ellipse((44 * u, 8 * u, 92 * u, 56 * u), fill=SUN)  # 太阳在右上，被云挡住下半部分

    stroke = max(round(4 * u), SS)  # 16 像素下描边至少 1 像素
    base = (14 * u, 62 * u, 88 * u, 86 * u)
    for box in cloud_shapes(u):
        d.ellipse(box, fill=OUTLINE)
    d.rounded_rectangle(base, radius=12 * u, fill=OUTLINE)
    inset = lambda b: (b[0] + stroke, b[1] + stroke, b[2] - stroke, b[3] - stroke)  # noqa: E731
    for box in cloud_shapes(u):
        d.ellipse(inset(box), fill=CLOUD)
    d.rounded_rectangle(inset(base), radius=12 * u - stroke, fill=CLOUD)
    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, size in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)]:
        render(size).save(OUT / name)
    frames = [render(s) for s in ICO_SIZES]
    frames[-1].save(OUT / "icon.ico", sizes=[(s, s) for s in ICO_SIZES], append_images=frames[:-1])
    print(f"已生成图标到 {OUT}")


if __name__ == "__main__":
    main()
