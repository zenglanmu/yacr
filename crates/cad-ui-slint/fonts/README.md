# Bundled shell font (not a CAD drawing font)

`YacrUI-Regular.otf` is a renamed, static TrueType subset of **Noto Sans SC**,
version `2.004-H2`, copyright Adobe 2014–2021, SIL OFL 1.1 (`OFL.txt`). The
reserved name `Source` is not used by the modified family `Yacr UI`.

The original Google Fonts v41 text subset was retrieved on 2026-10-02 using
`https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400&text=...`, where
`text` is the sorted unique characters from both UI catalogs and Web Rust host
source, `web/index.html`, `cad-app/src/host.rs` plus ASCII. The ribbon refresh
uses a 107,408-byte TTF whose SHA-256 is
`1cad1b5d140cf5ec5bdf56b78af293690b2a713962231b9376e439e26b0af603`.
Only the generated 106,588-byte font is needed for normal/offline builds;
regeneration uses `scripts/subset-ui-font.py` with fonttools `4.61.1`.

The shell explicitly imports this font and uses its family, independent of
fonts installed on the browser device. The static browser chrome uses the
same file; the build copies it and its license to `web-dist/ui-font/`.
CAD TEXT/MTEXT, arbitrary document names and external font fallback remain
separate concerns. This small subset does not claim full CJK coverage.
