"""Builds app icons from assets/logo.png (square crop, black glow -> alpha)."""
from pathlib import Path
from PIL import Image, ImageChops

root = Path(__file__).resolve().parent.parent
src = Image.open(root / "assets" / "logo.png").convert("RGB")
w, h = src.size
side = 1000  # the glow ends well inside this square
box = ((w - side) // 2, (h - side) // 2, (w + side) // 2, (h + side) // 2)
sq = src.crop(box)

# The art sits on pure black: turn brightness into alpha so the icon works on any background.
alpha = sq.convert("L").point(lambda v: min(255, int(v * 1.6)))
rgba = sq.convert("RGBA")
rgba.putalpha(alpha)

icons = root / "app" / "src-tauri" / "icons"
icons.mkdir(parents=True, exist_ok=True)
big = rgba.resize((512, 512), Image.LANCZOS)
big.save(icons / "icon.png")
for s in (32, 128):
    rgba.resize((s, s), Image.LANCZOS).save(icons / f"{s}x{s}.png")
big.save(icons / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])

assets = root / "app" / "src" / "assets"
assets.mkdir(parents=True, exist_ok=True)
rgba.resize((96, 96), Image.LANCZOS).save(assets / "logo.png")
print("ok", sorted(p.name for p in icons.iterdir()))
