"""Export the original approved Qingshu geometry to application/tray icons.

Requires Pillow for development only; the desktop app has no new dependency.
The SVG carries separate optical geometry for 16/24px and the master 32px mark.
"""
from pathlib import Path
import xml.etree.ElementTree as ET
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "desktop/src-tauri/icons"
RESOURCES = ROOT / "desktop/src-tauri/resources"
NS = {"s": "http://www.w3.org/2000/svg"}
SOURCE = ET.parse(ICONS / "qingshu.svg").getroot()
MINT = "#a9d6b9"
INK = "#28543b"


def render(size: int, tile: bool = True, color: str = INK, state: str = "idle") -> Image.Image:
    preset = 16 if size <= 20 else 24 if size <= 24 else 32
    scale = size * 4 / preset
    canvas = Image.new("RGBA", (size * 4, size * 4))
    draw = ImageDraw.Draw(canvas)
    if tile:
        draw.rounded_rectangle((0, 0, size * 4 - 1, size * 4 - 1), radius=size * .24 * 4, fill=MINT)
    group = SOURCE.find(f".//s:g[@id='mark-{preset}']", NS)
    if group is None:
        raise ValueError("Missing optical-size geometry")
    for rect in group:
        x, y, width, height, radius = [float(rect.get(key, "0")) * scale for key in ("x", "y", "width", "height", "rx")]
        bounds = (x, y, x + width - 1, y + height - 1)
        draw.rounded_rectangle(bounds, radius=radius, fill=color)
    if state != "idle":
        dot = {"recording": "#d76b61", "transcribing": MINT, "warning": "#e4ad51"}[state]
        center_x, center_y, radius = size * .84 * 4, size * .16 * 4, size * .105 * 4
        draw.ellipse((center_x-radius-3, center_y-radius-3, center_x+radius+3, center_y+radius+3), fill="#27352d" if color == "#eef5ef" else "#f0f2ee")
        draw.ellipse((center_x-radius, center_y-radius, center_x+radius, center_y+radius), fill=dot)
    return canvas.resize((size, size), Image.Resampling.LANCZOS)


def main() -> None:
    for name, size in (("32x32.png",32),("64x64.png",64),("128x128.png",128),("128x128@2x.png",256),("icon.png",512),("logo.png",256)):
        render(size).save(ICONS / name)
    ico_sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
    render(256).save(ICONS / "icon.ico", sizes=[(size,size) for size in ico_sizes], append_images=[render(size) for size in ico_sizes[:-1]])
    render(1024).save(ICONS / "icon.icns")
    for state in ("idle", "recording", "transcribing"):
        for suffix, color in (("", "#eef5ef"), ("_dark", "#263d2e")):
            render(32, tile=False, color=color, state=state).save(RESOURCES / f"tray_{state}{suffix}.png")
    for suffix, color in (("", "#eef5ef"), ("_dark", "#263d2e")):
        render(32, tile=False, color=color, state="warning").save(RESOURCES / f"tray_idle_warning{suffix}.png")
    for name, state in (("handy.png","idle"),("recording.png","recording"),("transcribing.png","transcribing"),("handy_warning.png","warning")):
        render(128, state=state).save(RESOURCES / name)
    print("Exported Qingshu application and tray icons from original SVG geometry.")


if __name__ == "__main__":
    main()
