# `fixtures/proxy` — SYNTHETIC proxy-cache byte corpus

> **These files are SYNTHETIC, not vendor evidence.**
> They were hand-authored from the documented record layout and were **not**
> produced by Tianzheng (天正), TSSD (探索者), AutoCAD, or any real DWG. Passing
> tests over them does **not** establish compatibility with any vendor format.

## Why it exists

`cad-proxy` must fail closed: an unknown, malformed, over-budget, or
unterminated record has to surface as `Partial`/`Missing` with the raw bytes
retained, never as guessed geometry. These small byte-level files pin that
contract at the framing and record level so the checks cannot silently regress.

## Format

Each `.hex` file is one complete proxy metafile, lower-case hex, no separators
(a trailing newline is fine). The layout matches acadrust 0.6.3
`ProxyGraphics::decode`:

```text
u32 total_size       (includes the 8-byte header)
u32 record_count
repeat record_count:
  u32 record_size    (includes the 8-byte record header)
  u32 record_type
  [u8; record_size - 8] payload
```

Type 36 (`UnicodeText`) payload layout, 96-byte fixed prefix then UTF-16LE
units terminated by `0x0000` (acadrust's encoder also pads to a 4-byte
boundary):

```text
0..24    position   f64 x, y, z
24..48   normal     f64 x, y, z
48..72   direction  f64 x, y, z
72..80   height
80..88   width_factor
88..96   oblique_angle
96..     UTF-16LE text, terminated by 0x0000
```

## Files and expected outcome

| File | Content | Expected |
| --- | --- | --- |
| `known_filloff_text.hex` | type 21 empty + type 36 `"A"` | `Complete`, one Text |
| `unknown_opcode.hex` | unknown type 999, 4 bytes | `Missing`, raw `[9,9,9,9]` retained |
| `mixed_known_unknown.hex` | type 21 + type 999 + type 36 `"B"` | `Missing`, unknown reported, trailing text not claimed |
| `truncated_text.hex` | type 36 with an 8-byte payload | unsupported, raw retained |
| `no_terminator.hex` | type 36 `"hi"` with no `0x0000` | unsupported (missing terminator) |
| `lying_total_size.hex` | header claims 4096 bytes | rejected at framing |

Exercised by `crates/cad-proxy/tests/synthetic_corpus.rs`.

## Adding real samples

Real vendor samples must be authorized and registered under
`fixtures/manifest` with provenance (software, version, source). Do not add
them here as `.hex`; keep this directory for synthetic, reviewable bytes only.
