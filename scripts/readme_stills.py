"""Regenerate the static SVGs used by the README's reduced motion fallback."""

from pathlib import Path
from xml.etree import ElementTree


ASSETS = Path(__file__).resolve().parents[1] / "assets"
SVG = "http://www.w3.org/2000/svg"
ElementTree.register_namespace("", SVG)

for name in ("lake-snapshot-flow", "console-startup"):
    tree = ElementTree.parse(ASSETS / f"{name}.svg")
    defs = tree.find(f"{{{SVG}}}defs")
    motion = defs.find(f"{{{SVG}}}style[@id='motion']") if defs is not None else None
    if motion is None:
        raise ValueError(f"{name}.svg is missing its motion stylesheet")
    defs.remove(motion)
    ElementTree.indent(tree, space="  ")
    with (ASSETS / f"{name}-still.svg").open("w", encoding="utf-8", newline="\n") as output:
        output.write(f"<!-- Generated from {name}.svg by scripts/readme_stills.py -->\n")
        tree.write(output, encoding="unicode")
        output.write("\n")
