"""Export approved Qingshu vectors to desktop application and tray icons.

Pillow is a development-only dependency. Android/iOS assets are never touched.
The SVG supplies the palette and 16/20/24/32px optical mark geometry.
"""
from pathlib import Path
import xml.etree.ElementTree as ET
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "desktop/src-tauri/icons"
RESOURCES = ROOT / "desktop/src-tauri/resources"
NS = {"s": "http://www.w3.org/2000/svg"}
SOURCE = ET.parse(ICONS / "qingshu.svg").getroot()
INK = SOURCE.find("s:g[@id='mark-32']", NS).get("fill")
TILE = SOURCE.find("s:rect[@id='tile']", NS)
STOPS = SOURCE.findall(".//s:linearGradient[@id='tile-fill']/s:stop", NS)
TILE_COLORS = [tuple(bytes.fromhex(stop.get("stop-color")[1:])) for stop in STOPS]
BADGES = {"recording": "#c54e49", "transcribing": "#23796c", "warning": "#b87525"}
SUPERSAMPLE = 8


def render(size: int, state: str = "idle", tray: bool = False, theme: str = "light") -> Image.Image:
    """Draw one transparent icon; tray variants retain a contrasting tile edge."""
    preset = next((value for value in (16, 20, 24) if size <= value), 32)
    extent = size * SUPERSAMPLE
    canvas = Image.new("RGBA", (extent, extent))
    mask = Image.new("L", canvas.size)
    edge = max(.75, size / 32) * SUPERSAMPLE
    inset = edge / 2
    bounds = (inset, inset, extent - inset - 1, extent - inset - 1)
    radius = size * .24 * SUPERSAMPLE
    ImageDraw.Draw(mask).rounded_rectangle(bounds, radius=radius, fill=255)
    gradient = Image.new("RGBA", canvas.size)
    rows = ImageDraw.Draw(gradient)
    for y in range(extent):
        fraction = y / max(1, extent - 1)
        rgb = tuple(round(top * (1 - fraction) + bottom * fraction)
                    for top, bottom in zip(*TILE_COLORS))
        rows.line((0, y, extent, y), fill=rgb + (255,))
    canvas.paste(gradient, mask=mask)
    draw = ImageDraw.Draw(canvas)
    # Light edge on a dark taskbar, darker edge on a light one. The tile and
    # ink keep the same brand colors; neither variant depends on bare white ink.
    border = "#d4eadb" if tray and theme == "dark" else TILE.get("stroke")
    draw.rounded_rectangle(bounds, radius=radius, outline=border, width=round(edge))
    group = SOURCE.find(f".//s:g[@id='mark-{preset}']", NS)
    if group is None:
        raise ValueError(f"Missing {preset}px optical geometry")
    # Every tray state reserves the same badge space, so its geometry never
    # shifts or changes size when recording/processing starts.
    mark_scale = .86 if tray or state != "idle" else 1
    scale = extent * mark_scale / preset
    offset = extent * (1 - mark_scale) / 2
    for rect in group:
        x, y, width, height, radius = [float(rect.get(key, "0")) * scale for key in ("x", "y", "width", "height", "rx")]
        bounds = (x + offset, y + offset, x + width + offset - 1, y + height + offset - 1)
        draw.rounded_rectangle(bounds, radius=radius, fill=INK)
    if state != "idle":
        center_x, center_y, radius = extent * .86, extent * .14, max(1.6, size * .10) * SUPERSAMPLE
        ring = max(.65, size * .035) * SUPERSAMPLE
        draw.ellipse((center_x-radius-ring, center_y-radius-ring, center_x+radius+ring, center_y+radius+ring), fill="#f3faf5")
        draw.ellipse((center_x-radius, center_y-radius, center_x+radius, center_y+radius), fill=BADGES[state])
    return canvas.resize((size, size), Image.Resampling.LANCZOS)


def main() -> None:
    outputs = {"16x16.png": 16, "20x20.png": 20, "24x24.png": 24,
               "32x32.png": 32, "64x64.png": 64, "128x128.png": 128,
               "128x128@2x.png": 256, "icon.png": 512, "logo.png": 256}
    # Windows shell derivatives must not retain upstream Handy artwork.
    for path in [*ICONS.glob("Square*Logo.png"), ICONS / "StoreLogo.png"]:
        with Image.open(path) as image:
            outputs[path.name] = image.width
    for name, size in outputs.items():
        render(size).save(ICONS / name)
    ico_sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
    render(256).save(ICONS / "icon.ico", sizes=[(size,size) for size in ico_sizes], append_images=[render(size) for size in ico_sizes[:-1]])
    render(1024).save(ICONS / "icon.icns")
    for state in ("idle", "recording", "transcribing", "warning"):
        resource_state = "idle_warning" if state == "warning" else state
        for suffix, theme in (("", "dark"), ("_dark", "light")):
            render(32, state=state, tray=True, theme=theme).save(RESOURCES / f"tray_{resource_state}{suffix}.png")
    for name, state in (("handy.png","idle"),("recording.png","recording"),("transcribing.png","transcribing"),("handy_warning.png","warning")):
        render(128, state=state).save(RESOURCES / name)
    print("Exported Qingshu desktop icons and tiled tray states; mobile assets untouched.")


if __name__ == "__main__":
    main()
