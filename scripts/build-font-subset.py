#!/usr/bin/env python3
"""Rebuild the font embedded into the wasm build of `rdp-gui`.

Browsers expose no system fonts, so GPUI cannot resolve its default family
(`.SystemUIFont`) there. This script takes an OFL-licensed font, keeps only the
glyphs the UI actually draws, and renames the family to `.SystemUIFont`, so the
framework's default resolves to the embedded face on both targets.

Run it after adding new UI text:

    python scripts/build-font-subset.py <path-to-NotoSansSC-Regular.otf>

The output is `rdp-gui/assets/fonts/SystemUIFont.otf`, plus a `glyphs.txt`
listing what the subset covers.
"""

from __future__ import annotations

import pathlib
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC_DIR = ROOT / "rdp-gui" / "src"
OUT_DIR = ROOT / "rdp-gui" / "assets" / "fonts"
FAMILY = "RdpUI"

# Characters that appear in UI strings but not in the Rust sources.
EXTRA = "：·…●◉⇄≡◇⚙▸—–、，。！？（）【】「」"

# Latin letters that a future locale or a server-provided string may reach for.
ASCII = "".join(chr(code) for code in range(0x20, 0x7F))


def collect_characters() -> set[str]:
    characters = set(ASCII) | set(EXTRA)
    for path in SRC_DIR.rglob("*.rs"):
        characters.update(path.read_text(encoding="utf-8"))
    return {character for character in characters if character.isprintable()}


def rename_family(font: TTFont) -> None:
    names = font["name"]
    replacements = {
        1: FAMILY,  # family
        3: f"{FAMILY};1.0",  # unique id
        4: FAMILY,  # full name
        6: f"{FAMILY}-Regular",  # postscript name
        16: FAMILY,  # typographic family
        17: "Regular",  # typographic subfamily
    }
    for record in list(names.names):
        if record.nameID not in replacements:
            continue
        value = replacements[record.nameID]
        names.removeNames(nameID=record.nameID)
        names.setName(value, record.nameID, record.platformID, record.platEncID, record.langID)
        break_on_platform = None  # noqa: F841 - keeps the intent of a single pass visible
    for name_id, value in replacements.items():
        if not names.getDebugName(name_id):
            names.setName(value, name_id, 3, 1, 0x409)


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2

    source = pathlib.Path(sys.argv[1])
    if not source.exists():
        print(f"missing source font: {source}")
        return 1

    characters = collect_characters()
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.recalc_bounds = True
    options.hinting = False
    options.desubroutinize = True

    font = subset.load_font(str(source), options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=[ord(character) for character in characters])
    subsetter.subset(font)
    rename_family(font)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    target = OUT_DIR / "RdpUI.otf"
    font.flavor = None
    font.save(str(target))

    glyphs = OUT_DIR / "glyphs.txt"
    glyphs.write_text(
        "Characters the embedded UI font covers.\n"
        "Regenerate with scripts/build-font-subset.py after changing UI text.\n\n"
        + "".join(sorted(characters)),
        encoding="utf-8",
    )

    print(f"wrote {target.relative_to(ROOT)} ({target.stat().st_size / 1024:.0f} KiB)")
    print(f"covered {len(characters)} characters")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
