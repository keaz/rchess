# Piece images

The twelve PNGs here are the Cburnett chess pieces by Colin M.L. Burnett
(Wikimedia Commons user [Cburnett](https://commons.wikimedia.org/wiki/User:Cburnett)),
rasterized once from the original SVGs and embedded in the binary by
`src/tui/pieces.rs` with `include_bytes!`, so the program needs no files at run time.

## Licence

Every file page offers the images under a choice of licences
(`{{self|GFDL|migration=relicense|BSD|GPL}}`: GFDL 1.2 or later, CC BY-SA 3.0,
3-clause BSD, GPL 2 or later; "You may select the license of your choice").
rchess uses them under the **BSD licence**; see [`LICENSE`](LICENSE) for the text
and attribution. The Commons API's `extmetadata` reports only the first listed
licence (`LicenseShortName` "CC BY-SA 3.0", `License` "cc-by-sa-3.0", `UsageTerms`
"Creative Commons Attribution-Share Alike 3.0"); the full list, BSD included, is
in the licence section of each file page. Checked on 2026-09-27.

## Sources

| PNG | Commons file page | Original SVG | SHA-1 of the SVG |
| --- | --- | --- | --- |
| `wK.png` | <https://commons.wikimedia.org/wiki/File:Chess_klt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/4/42/Chess_klt45.svg> | `2c7569b837971207e40f7148e2b2086aaf8e4bbd` |
| `bK.png` | <https://commons.wikimedia.org/wiki/File:Chess_kdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/f/f0/Chess_kdt45.svg> | `b1165ef85a3df6f1af2549ab0af78ab21b540e8a` |
| `wQ.png` | <https://commons.wikimedia.org/wiki/File:Chess_qlt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/1/15/Chess_qlt45.svg> | `e638eb28ec25007b9f8fac8476bdf6ae1fc5a0ee` |
| `bQ.png` | <https://commons.wikimedia.org/wiki/File:Chess_qdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/4/47/Chess_qdt45.svg> | `ed72e75b7bdbf880a3c9bee053c8786cfab8bdb9` |
| `wR.png` | <https://commons.wikimedia.org/wiki/File:Chess_rlt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/7/72/Chess_rlt45.svg> | `126b7779885b87e87acc713474587732711bc8d3` |
| `bR.png` | <https://commons.wikimedia.org/wiki/File:Chess_rdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/f/ff/Chess_rdt45.svg> | `bd0e866f1e6da8e3d6f9c0b356b60fc58391aff6` |
| `wB.png` | <https://commons.wikimedia.org/wiki/File:Chess_blt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/b/b1/Chess_blt45.svg> | `35dd477dc22636bfcc559a01f75b00369205ffda` |
| `bB.png` | <https://commons.wikimedia.org/wiki/File:Chess_bdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/9/98/Chess_bdt45.svg> | `da6dd1b5ef629bacebbd2ce26c7d81ba8a205587` |
| `wN.png` | <https://commons.wikimedia.org/wiki/File:Chess_nlt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/7/70/Chess_nlt45.svg> | `3a2253429c0e39863b3f5ecf447209dccecdc337` |
| `bN.png` | <https://commons.wikimedia.org/wiki/File:Chess_ndt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/e/ef/Chess_ndt45.svg> | `3c79cbbda76bcf1d4e4062cef6dd3b58a250a562` |
| `wP.png` | <https://commons.wikimedia.org/wiki/File:Chess_plt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/4/45/Chess_plt45.svg> | `09d59e2770fcee23722ac53b26e875f76d2c1eb1` |
| `bP.png` | <https://commons.wikimedia.org/wiki/File:Chess_pdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/c/c7/Chess_pdt45.svg> | `0a6d2f3dc6327a02ca591bc489d701fbff228138` |

## Conversion

Each SVG (45×45 user units) was rasterized to a 256×256 RGBA PNG with a
transparent background, using rsvg-convert version 2.63.2 (librsvg, with cairo
1.18.6):

```sh
for k in k q r b n p; do
  for c in l d; do
    case $c in l) side=w ;; d) side=b ;; esac
    K=$(echo "$k" | tr a-z A-Z)
    rsvg-convert -w 256 -h 256 -o "assets/pieces/$side$K.png" "Chess_$k${c}t45.svg"
  done
done
```

`w` is White (`lt`, light), `b` is Black (`dt`, dark); the letter is the piece
(`K`ing, `Q`ueen, `R`ook, `B`ishop, k`N`ight, `P`awn).
