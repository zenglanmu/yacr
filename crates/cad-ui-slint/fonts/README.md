# Bundled shell font (not a CAD drawing font)

`YacrUI-Regular.otf` is a renamed, static TrueType subset of **Noto Sans SC**,
version `2.004-H2`, copyright Adobe 2014–2021, SIL OFL 1.1 (`OFL.txt`). The
reserved name `Source` is not used by the modified family `Yacr UI`.

The original Google Fonts v41 text subset was retrieved on 2026-10-02 using
`https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400&text=...`, where
`text` is the sorted unique characters from both UI catalogs and Web Rust host
source, `web/index.html`, `cad-app/src/host.rs` plus ASCII. The ribbon refresh
was refreshed for the concept catalogs on 2026-10-03 using a 111,496-byte TTF whose SHA-256 is
`0995608b81d0d1b843b04e27b7557fe2cac453cac29fcfec7e8afcf086a0bc04`.
Only the generated 110,676-byte font (507 codepoints) is needed for normal/offline builds;
regeneration uses `scripts/subset-ui-font.py` with fonttools `4.61.1`.

The shell explicitly imports this font and uses its family, independent of
fonts installed on the browser device. The static browser chrome uses the
same file; the build copies it and its license to `web-dist/ui-font/`.
CAD TEXT/MTEXT, arbitrary document names and external font fallback remain
separate concerns. This small subset does not claim full CJK coverage.
