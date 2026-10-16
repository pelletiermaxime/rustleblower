# Rustleblower icons

The approved mascot is Ferris blowing a whistle, in colour. The three PNGs in
`source/` are the original approved generated artwork. Do not derive the small
icons from the full mascot: the drawings deliberately omit different details.

| Installed size | Drawing |
| --- | --- |
| 16, 22, 24 px | Tiny face and whistle |
| 32 px | Compact mascot |
| 48, 64, 128, 256, 512 px | Full Ferris mascot |

The checked-in `hicolor/` exports are used by both the application and packages.
Regenerate them with `python3 scripts/generate-icons.py` (requires ImageMagick).
Normal builds and packaging do not need an image converter.

The tray publishes 16/22/24/32 px ARGB images through StatusNotifierItem; it
leaves IconName empty so the host selects these size-specific drawings. Native
X11 windows use the full 256 px image. Wayland identifies windows through the
existing desktop application ID and resolves the installed hicolor icon.

The three colour drawings were generated with the built-in image generator
using [Ferris's original artwork](https://rustacean.net/) as the identity reference.
Ferris was created by Karen Rustad Tölva and dedicated to the public domain
under CC0; see `LICENSES/Ferris-CC0.txt`. No monochrome variant is shipped.
