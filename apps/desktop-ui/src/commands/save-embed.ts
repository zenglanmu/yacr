// The optional pre-save "embed external resources" question.
//
// SAVE goes straight to the host's picker. Before that picker appears the chrome
// may ask one question: should the external files the drawing uses (fonts,
// external-reference sources, images) travel *inside* the saved file? The host
// answers whether the question applies and what it would cost — this module is
// the whole model of that answer, so the dialog and the shell stay dumb:
//
//   * `readEmbedPreview` asks `cad_ymodel_resource_preview`. A **refusal is the
//     honest answer** on a carrier that cannot resolve external resources (Web and
//     the X11 `dom-canvas` path have no local directory to read a font/xref/image
//     from), even though such a carrier can open and save the archive itself: it
//     returns `null`, the dialog never opens and the save proceeds exactly as
//     before. It is never rewritten into a fake "nothing to embed" answer — that
//     is why `null` and an empty list are two different results.
//   * `parseEmbedPreview` validates the payload; a row it cannot name is
//     dropped, and a payload without an `entries` array is not a preview. A
//     `found` flag counts only as `true`: a resource is never claimed as
//     embeddable because the host was ambiguous.
//   * `resolveSaveEmbed` is the confirmation step. Confirm applies the choice
//     (`cad_set_embed_resources { embed }`) and only then hands control back to
//     the funnel that sends `cad_command`; a refusal returns `failed` and the
//     caller must **not** save, so a choice is never silently ignored. Cancel
//     sends nothing at all.
import type { Invoke } from '../host/invoke.ts';

/**
 * The router names whose real host path is the SAVE funnel.
 *
 * They are the only lines that write the document itself (always `.ymodel`,
 * ADR-0041), so they are the only ones this question applies to. `EXPORT` is
 * deliberately absent: it opens the host's export picker and writes a copy in
 * the chosen extension, embedding nothing.
 */
export const SAVE_FAMILY_COMMANDS: readonly string[] = Object.freeze(['SAVE', 'SAVEAS', 'QSAVE']);

/** The IPC that reports what would be embedded (and whether the host asks). */
export const EMBED_PREVIEW_COMMAND = 'cad_ymodel_resource_preview';

/** The IPC that stores the user's choice on the document. */
export const EMBED_APPLY_COMMAND = 'cad_set_embed_resources';

/** The resource kinds the host reports, in display order. */
export const EMBED_KINDS: readonly string[] = Object.freeze(['font', 'xref', 'image']);

/** One external resource the save would embed. */
export interface EmbedResourceEntry {
  /** `font` | `xref` | `image` (see [`EMBED_KINDS`]). */
  readonly kind: string;
  /** The display name the host reported (never empty). */
  readonly name: string;
  /** Where it was looked for, or `null` when the host did not say. */
  readonly source: string | null;
  /** Reported size in bytes, or `null` when the host reported no usable count. */
  readonly bytes: number | null;
  /** Whether the host actually found it (only an explicit `true` counts). */
  readonly found: boolean;
}

/** The host's answer to [`EMBED_PREVIEW_COMMAND`]. */
export interface EmbedPreview {
  /** The host's own default for the toggle, or `null` when it reported none. */
  readonly policy: boolean | null;
  readonly entries: readonly EmbedResourceEntry[];
}

/** The two lists the dialog renders, in host order. */
export interface EmbedGroups {
  readonly found: readonly EmbedResourceEntry[];
  readonly missing: readonly EmbedResourceEntry[];
}

/** What the dialog's two actions resolve to. */
export type SaveEmbedAction = 'confirm' | 'cancel';

/** The two calls one confirmation makes; both are injected so this stays pure. */
export interface SaveEmbedDeps {
  readonly invoke: Invoke;
  /** Hand the SAVE line back to the shell's one command funnel. */
  readonly dispatchSave: () => void;
}

/** The outcome of one action. `failed` means "the save must not happen". */
export type SaveEmbedResult =
  | { readonly kind: 'saved' }
  | { readonly kind: 'cancelled' }
  | { readonly kind: 'failed'; readonly error: string };

/** A plain JSON object, or `null` for anything else (arrays included). */
function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Whether `text` is one of the router names the host saves through. */
export function isSaveFamilyCommand(text: unknown): boolean {
  return SAVE_FAMILY_COMMANDS.includes(String(text ?? '').trim().toUpperCase());
}

/** The catalog key of one kind's label (`save.embed.kind.<kind>`). */
export function embedKindKey(kind: string): string {
  return `save.embed.kind.${kind}`;
}

/** One reported row, or `null` when it cannot be shown honestly. */
function parseEntry(value: unknown): EmbedResourceEntry | null {
  const row = recordOf(value);
  if (row === null) return null;
  const kind = typeof row.kind === 'string' && EMBED_KINDS.includes(row.kind) ? row.kind : null;
  const name = typeof row.name === 'string' ? row.name.trim() : '';
  if (kind === null || name.length === 0) return null;
  const source = typeof row.source === 'string' && row.source.trim().length > 0
    ? row.source
    : null;
  const bytes = typeof row.bytes === 'number' && Number.isFinite(row.bytes) && row.bytes >= 0
    ? Math.round(row.bytes)
    : null;
  return { kind, name, source, bytes, found: row.found === true };
}

/**
 * The validated preview, or `null` when the answer is not one.
 *
 * A payload without an `entries` array is *not* "nothing to embed": it is an
 * answer the chrome cannot read, and the caller proceeds without a question.
 */
export function parseEmbedPreview(value: unknown): EmbedPreview | null {
  const wire = recordOf(value);
  if (wire === null || !Array.isArray(wire.entries)) return null;
  const entries: EmbedResourceEntry[] = [];
  for (const raw of wire.entries) {
    const entry = parseEntry(raw);
    if (entry !== null) entries.push(entry);
  }
  return {
    policy: typeof wire.policy === 'boolean' ? wire.policy : null,
    entries,
  };
}

/**
 * Ask the host what it would embed.
 *
 * A refusal returns `null`: the caller saves exactly as it does today, without a
 * dialog and without an error banner — the refusal is a fact about this carrier,
 * not a failure of the user's save.
 */
export async function readEmbedPreview(invoke: Invoke): Promise<EmbedPreview | null> {
  try {
    return parseEmbedPreview(await invoke<unknown>(EMBED_PREVIEW_COMMAND));
  } catch {
    return null;
  }
}

/** Whether the preview earns a question (it lists at least one resource). */
export function embedNeedsDialog(preview: EmbedPreview | null): boolean {
  return preview !== null && preview.entries.length > 0;
}

/** Split the rows into the two groups the dialog renders (host order kept). */
export function embedGroups(entries: readonly EmbedResourceEntry[]): EmbedGroups {
  return {
    found: entries.filter(entry => entry.found),
    missing: entries.filter(entry => !entry.found),
  };
}

/**
 * A human-readable size (`1.4 MB`, `512 KB`, `64 B`), or `''` for no answer.
 *
 * Binary units (1024) with one decimal below 100 so a small difference stays
 * visible; a reported non-count is `''` rather than `0 B`, which would claim the
 * file is empty.
 */
export function formatByteSize(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return '';
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const text = value >= 100 ? String(Math.round(value)) : value.toFixed(1).replace(/\.0$/, '');
  return `${text} ${units[unit]}`;
}

/** The found rows' count and byte sum (what the file would actually carry). */
export function embedTotal(entries: readonly EmbedResourceEntry[]): { count: number; bytes: number } {
  const found = entries.filter(entry => entry.found);
  return {
    count: found.length,
    bytes: found.reduce((sum, entry) => sum + (entry.bytes ?? 0), 0),
  };
}

/**
 * The toggle's initial state.
 *
 * The host's `policy` wins when it reported one. When it did not, the question
 * defaults to **on iff something was found**: asking "embed?" for a list of
 * missing files would pre-select an action that embeds nothing.
 */
export function embedInitialToggle(preview: EmbedPreview | null): boolean {
  if (preview === null) return false;
  if (preview.policy !== null) return preview.policy;
  return preview.entries.some(entry => entry.found);
}

/** The raw refusal text of a failed call (never invented copy). */
function refusalText(error: unknown): string {
  const text = String(error ?? '').trim();
  return text.length > 0 ? text : EMBED_APPLY_COMMAND;
}

/**
 * Apply one action.
 *
 * Cancel sends nothing. Confirm applies the choice and then dispatches SAVE; a
 * refusal of the apply returns `failed` **without** dispatching, so the shell
 * shows the reason instead of saving a file that ignores the user's answer.
 */
export async function resolveSaveEmbed(
  action: SaveEmbedAction,
  embed: boolean,
  deps: SaveEmbedDeps,
): Promise<SaveEmbedResult> {
  if (action === 'cancel') return { kind: 'cancelled' };
  try {
    await deps.invoke(EMBED_APPLY_COMMAND, { embed });
  } catch (error) {
    return { kind: 'failed', error: refusalText(error) };
  }
  deps.dispatchSave();
  return { kind: 'saved' };
}
