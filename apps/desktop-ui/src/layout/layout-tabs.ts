// The drawing-space strip's pure model: Model plus one row per real layout.
//
// The wire is `cad_status.layouts` + `cad_status.activeSpace` (both additive on
// an older host). This module only projects those rows into what the desktop
// strip and the compact drawer render, so both layouts read the same tab list
// and the same command strings. It never invents a layout and never claims a
// switch succeeded: `MODEL` / `SPACE #<id>` is queue admission, and the host's
// next `cad_status` is the only evidence of the space change.
import { activeSpaceOf, layoutTabsOf } from '../cad/cad-status.ts';
import type { CadStatus } from '../cad/cad-status.ts';
import type { Translator } from '../i18n.ts';

/**
 * The command for a real paper layout. The host canonicalises the whole line
 * and resolves the `LayoutId` against the drawing's layout table, so an unknown
 * id is an explicit refusal, never a blank sheet.
 */
export function layoutSpaceCommand(id: number): string {
  return `SPACE #${id}`;
}

/**
 * The command back to model space. AutoCAD 2026's `MODEL` selector is the
 * official payload-free switch ("switches from a named layout tab to the Model
 * tab"), so the Model tab dispatches it directly; paper tabs keep the
 * `SPACE #<id>` grammar.
 */
export const MODEL_SPACE_COMMAND = 'MODEL';

/** One rendered tab: `id === null` is model space. */
export interface LayoutTabView {
  /** The wire `LayoutId`, or `null` for model space. */
  readonly id: number | null;
  /** The drawing's layout name, or `null` for model space. */
  readonly name: string | null;
  readonly supported: boolean;
  readonly active: boolean;
  /** Real viewport count, `null` for model space. */
  readonly viewportCount: number | null;
  /** The stable code from the host's `"<code>: <message>"` reason. */
  readonly reasonCode: string | null;
  /** The exact `SPACE` line to dispatch, `null` while this tab is active. */
  readonly command: string | null;
}

/**
 * The stable reason code: the text before the first `:`. A reason without a
 * separator is treated as the code itself; an empty reason is `null` rather
 * than a made-up explanation.
 */
export function layoutReasonCode(reason: string | null | undefined): string | null {
  if (typeof reason !== 'string') return null;
  const code = reason.split(':', 1)[0]?.trim() ?? '';
  return code.length > 0 ? code : null;
}

/**
 * The tab list, in drawing order: model first, then every layout the host
 * reported. An older host that omits both fields yields model alone and active.
 */
export function layoutTabsView(status: Partial<CadStatus> | null | undefined): readonly LayoutTabView[] {
  const active = activeSpaceOf(status);
  const modelActive = active.kind === 'model';
  const model: LayoutTabView = {
    id: null,
    name: null,
    supported: true,
    active: modelActive,
    viewportCount: null,
    reasonCode: null,
    command: modelActive ? null : MODEL_SPACE_COMMAND,
  };
  return [
    model,
    ...layoutTabsOf(status).map((layout): LayoutTabView => {
      const isActive = active.kind === 'paper' && active.id === layout.id;
      return {
        id: layout.id,
        name: layout.name,
        supported: layout.supported,
        active: isActive,
        viewportCount: layout.viewportCount,
        reasonCode: layoutReasonCode(layout.reason),
        command: isActive || !layout.supported ? null : layoutSpaceCommand(layout.id),
      };
    }),
  ];
}

/** The visible/tooltip title of an unsupported tab: marker plus stable code. */
export function layoutUnsupportedTitle(t: Translator, tab: LayoutTabView): string {
  const marker = t('layout.unsupported_marker');
  return tab.reasonCode === null
    ? marker
    : t('layout.tab.unsupported_title', { marker, reason: tab.reasonCode });
}
