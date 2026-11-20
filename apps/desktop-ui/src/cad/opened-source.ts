// The `cad_status.openedSource` provenance projection (ADR-0041 D5).
//
// Every connector declares how its format is implemented: from a published
// specification (`Public`) or by reconstruction (`ReverseEngineered`). Only the
// second kind produces this block, so it is **absent** for a public-spec
// document, a native `.ymodel`, and after New/Close. The Rust wire is the
// authority for that absence (`cad_ui_core::session::status::CadStatus`); this
// module is the block's single reader and the notice's copy builder, so the card
// (`components/FormatProvenanceNotice.tsx`) renders exactly what is returned
// here and invents nothing.
//
// The block is additive and display-only, and the status reader is the one
// place that owns the core snapshot. A present block is therefore validated
// here: anything that is not the documented `{ connector, spec, canWrite,
// experimental }` shape reads as "no notice" rather than as a guessed format, a
// guessed writer capability, or an English sentence assembled from a bad field.
// No capability is claimed either way, so silence is not a false success.
//
// Dismissal is per **open**, not per file: the key is the document label plus
// the connector, and the host keeps it in memory only (nothing is written to
// disk, so a restart asks again).
import type { Translator } from '../i18n.ts';

/** The two spec visibilities the host puts on the wire. */
export type CadSpecVisibility = 'public' | 'reverse_engineered';

/**
 * Provenance of the connector that opened the current document.
 *
 * Wire shape: `{ connector, spec, canWrite, experimental }`. `canWrite` is
 * whether the connector declares an export direction at all; `experimental` is
 * whether that writer is unverified (and is `false` without a writer).
 */
export interface CadOpenedSource {
  /** Stable registry connector id (`dwg`, `dgn`, `skp`, `rvt`). */
  readonly connector: string;
  /** The format's spec visibility (always `reverse_engineered` for a notice). */
  readonly spec: CadSpecVisibility;
  /** Whether the connector can write its format back at all. */
  readonly canWrite: boolean;
  /** Whether that writer is experimental/unverified. */
  readonly experimental: boolean;
}

/**
 * The provenance whose notice is due, or `null` when nothing should show.
 *
 * `null` covers the absent key (no document, New/Close, a native `.ymodel`, a
 * public-spec connector), an explicit `null`, a block that fails validation, and
 * a `spec` this build does not warn about. Only `reverse_engineered` ever shows
 * the notice, which is also the only value the host emits today.
 */
export function openedSourceOf(
  status: { readonly openedSource?: unknown } | null | undefined,
): CadOpenedSource | null {
  const source = status?.openedSource;
  if (!source || typeof source !== 'object' || Array.isArray(source)) return null;
  const block = source as Record<string, unknown>;
  if (typeof block.connector !== 'string' || block.connector.length === 0) return null;
  if (block.spec !== 'reverse_engineered') return null;
  // A boolean capability is the host's own answer; a truthy string must never
  // become "writes back experimentally" or "cannot write" in the copy.
  if (typeof block.canWrite !== 'boolean' || typeof block.experimental !== 'boolean') return null;
  return {
    connector: block.connector,
    spec: 'reverse_engineered',
    canWrite: block.canWrite,
    experimental: block.experimental,
  };
}

/** The line keys the card renders below its title, in display order. */
export type OpenNoticeLineKey = 'body' | 'detail' | 'saveBack' | 'ymodel';

/** One body paragraph of the open-time notice. */
export interface OpenNoticeLine {
  /** Stable line key; the card styles `ymodel` as the recommendation. */
  readonly key: OpenNoticeLineKey;
  readonly text: string;
}

/** The whole notice, already translated: title first, then the body lines. */
export interface OpenNoticeView {
  /** The connector's localized name (or its raw id when there is no key). */
  readonly format: string;
  readonly title: string;
  readonly lines: readonly OpenNoticeLine[];
}

/** Prefix of the per-format detail sentence (`open.notice.detail.<connector>`). */
const DETAIL_KEY_PREFIX = 'open.notice.detail.';

/**
 * The connector name from the catalog, falling back to the raw registry id.
 *
 * `t` returns the key itself for a missing message, which is the honest marker
 * of a gap; showing `dwg` is better than showing `connector.dwg.name` or an
 * empty `{format}` placeholder.
 */
export function connectorLabel(connector: string, t: Translator): string {
  const key = `connector.${connector}.name`;
  const label = t(key);
  return label === key ? connector : label;
}

/**
 * Build the notice for `source`.
 *
 * Order is fixed (ADR-0041 D5): the "not a public format" sentence, what that
 * costs on export, the per-format detail when the catalog has one, then the
 * save-back truth, then the `.ymodel` recommendation. The save-back line is
 * never both kinds: an absent writer is `unsupported`, a writer that exists but
 * is unverified is `experimental`. A verified writer (the one combination that
 * has no sentence today) omits the line instead of borrowing one that would be
 * false, and the `.ymodel` line still shows.
 */
export function openNoticeView(source: CadOpenedSource, t: Translator): OpenNoticeView {
  const format = connectorLabel(source.connector, t);
  const lines: OpenNoticeLine[] = [
    { key: 'body', text: t('open.notice.reverse.body', { format }) },
  ];
  const detailKey = `${DETAIL_KEY_PREFIX}${source.connector}`;
  const detail = t(detailKey);
  if (detail !== detailKey) lines.push({ key: 'detail', text: detail });
  if (!source.canWrite) {
    lines.push({ key: 'saveBack', text: t('open.notice.save_back.unsupported', { format }) });
  } else if (source.experimental) {
    lines.push({ key: 'saveBack', text: t('open.notice.save_back.experimental', { format }) });
  }
  lines.push({ key: 'ymodel', text: t('open.notice.save_back.ymodel') });
  return {
    format,
    title: t('open.notice.reverse.title', { format }),
    lines,
  };
}

/**
 * The in-memory dismissal key for one open.
 *
 * The document label is the host's own (`cad_status.documentLabel`), so two
 * different files opened from the same connector produce different keys, and
 * the same file reopened after New/Close produces the same one — which is why
 * the host clears the stored key when the document goes away instead of letting
 * a stale dismissal silence the next open.
 */
export function openNoticeKey(label: string | null | undefined, connector: string): string {
  return `${label ?? ''}\u0000${connector}`;
}

/** Whether the notice for the current open was already dismissed. */
export function isOpenNoticeDismissed(
  dismissedKey: string | null,
  currentKey: string | null,
): boolean {
  return currentKey !== null && dismissedKey === currentKey;
}
