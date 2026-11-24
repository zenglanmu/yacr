# Domain chrome golden screenshots

`cad2d.png`, `bim.png`, `concept.png`, `mechanical.png` are **our own** captures of
the four application-domain shells at 1600×1000 CSS px (`apps/desktop-ui` built into
`apps/app-web/dist-ui`). They are layout evidence only:

- software Chromium (`chrome-headless-shell`, no GPU), not GPU/pixel/performance
  acceptance;
- the reference applications' manual screenshots are **not** stored here (copyright);
  the structural transcription lives in `apps/desktop-ui/test/fixtures/autocad-2024-drafting.json`;
- a diff means "the rendered chrome changed", not "the app is broken".

Update / verify:

```bash
bash scripts/build-web-ui.sh                       # Vite + wasm (fonts may need network)
python3 scripts/serve-web.py --directory apps/app-web/dist-ui --port 8090 &
node scripts/ui-golden.cjs --url http://127.0.0.1:8090/ --update   # rewrite baselines
node scripts/ui-golden.cjs --url http://127.0.0.1:8090/            # compare
```

Requires a Chromium build: `CHROME_BIN`, or the newest Playwright
`chromium_headless_shell` under `~/.cache/ms-playwright`. The script speaks CDP
directly (no Playwright dependency). CI wiring is **not done** yet; the font
package stage of `build-web-ui.sh` needs network.
