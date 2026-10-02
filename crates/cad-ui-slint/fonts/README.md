# Bundled shell font (not a CAD drawing font)

`YacrUI-Regular.otf` is a renamed, static TrueType subset of **Noto Sans SC**,
version `2.004-H2`, copyright Adobe 2014–2021, SIL OFL 1.1 (`OFL.txt`). The
reserved name `Source` is not used by the modified family `Yacr UI`.

The original Google Fonts v41 text subset was retrieved on 2026-10-02 using
`https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400&text=...`, where
`text` is the sorted unique characters from both UI catalogs and Web Rust host
source, `web/index.html`, `cad-app/src/host.rs` plus ASCII. Its 92,016-byte TTF SHA-256 is
`ad97de5dcb3579828a10354f9a2d2e57d67ab3676507460fc051f0a1f726eddd`.
Only the generated 91,192-byte font is needed for normal/offline builds;
regeneration uses `scripts/subset-ui-font.py` with fonttools `4.61.1`.

The shell explicitly imports this font and uses its family, independent of
fonts installed on the browser device. The static browser chrome uses the
same file; the build copies it and its license to `web-dist/ui-font/`.
CAD TEXT/MTEXT, arbitrary document names and external font fallback remain
separate concerns. This small subset does not claim full CJK coverage.
