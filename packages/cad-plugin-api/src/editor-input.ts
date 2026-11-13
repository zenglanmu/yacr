// The EditorInput surface: user prompts and picking.
//
// Every member is `Promise`-returning, every member is capability-gated before
// the host is asked, and no member ever invents an answer. In v1 every *prompt*
// is `pending`, so a call refuses with `api.pending: EditorInput.editor.<op>`; it
// does not resolve `PromptStatus.None` as if the user had pressed Escape, and it
// does not resolve an empty selection.
//
// `currentSelection()` is the one served member: a reflected read of the
// selection the user has right now (not a prompt). An empty set is a real answer,
// and a host with no document refuses `cad_api.no_document` — this module never
// turns either into a fabricated selection.
//
// A host answer is validated: an unknown `status` is `api.invalid_argument:
// bridge.answer`, never coerced into one of the five known states. The one void
// prompt (`writeMessage`) must still answer `{ accepted: true }`: an ack is a real
// answer, and a silent return is indistinguishable from a dropped request.

import type { ObjectId } from './database-services.ts';
import { objectId } from './database-services.ts';
import { invalidArgument } from './errors.ts';
import type { Point3d } from './geometry.ts';
import { point3d } from './geometry.ts';
import type { CadApiWire } from './internal/capability.ts';
import { isRecord, requireAnswerObject } from './internal/capability.ts';

/** The five prompt outcomes AutoCAD's `PromptStatus` distinguishes. */
export const PromptStatus = {
  Ok: 'Ok',
  Cancel: 'Cancel',
  Error: 'Error',
  Keyword: 'Keyword',
  None: 'None',
} as const;

/** `'Ok' | 'Cancel' | 'Error' | 'Keyword' | 'None'`. */
export type PromptStatus = (typeof PromptStatus)[keyof typeof PromptStatus];

/**
 * One prompt outcome.
 *
 * `Ok` carries the value, `Cancel`/`None` carry nothing (the user dismissed the
 * prompt or it never ran), `Error` carries the host's message and `Keyword`
 * carries the matched keyword.
 */
export type PromptResult<T> =
  | { readonly status: 'Ok'; readonly value: T }
  | { readonly status: 'Cancel' }
  | { readonly status: 'Error'; readonly message: string }
  | { readonly status: 'Keyword'; readonly keyword: string }
  | { readonly status: 'None' };

/** A resolved selection: an ordered, immutable list of object ids. */
export interface SelectionSet {
  readonly count: number;
  getObjectIds(): readonly ObjectId[];
}

/** Build a selection set from ids the host returned (no wire involved). */
export function selectionSet(ids: readonly ObjectId[]): SelectionSet {
  const frozen = Object.freeze([...ids]);
  return {
    count: frozen.length,
    getObjectIds: () => frozen,
  };
}

/** Options for `Editor.getPoint`. */
export interface PromptPointOptions {
  readonly message?: string;
  readonly basePoint?: Point3d;
  readonly useBasePoint?: boolean;
}

/** Options for `Editor.getEntity`. */
export interface PromptEntityOptions {
  readonly message?: string;
  readonly allowNone?: boolean;
  readonly rejectMessage?: string;
}

/** Which objects a selection may contain. */
export interface SelectionFilter {
  readonly objectTypes?: readonly string[];
  readonly layerNames?: readonly string[];
}

/** Options for `Editor.getSelection` / `Editor.selectAll`. */
export interface PromptSelectionOptions {
  readonly message?: string;
  readonly singleOnly?: boolean;
  readonly filter?: SelectionFilter;
}

/** Options for `Editor.getString`. */
export interface PromptStringOptions {
  readonly message?: string;
  readonly allowSpaces?: boolean;
  readonly defaultValue?: string;
}

/** Options for `Editor.getDouble`. */
export interface PromptDoubleOptions {
  readonly message?: string;
  readonly defaultValue?: number;
  readonly minimum?: number;
  readonly maximum?: number;
}

/** Options for `Editor.getInteger`. */
export interface PromptIntegerOptions {
  readonly message?: string;
  readonly defaultValue?: number;
  readonly minimum?: number;
  readonly maximum?: number;
}

/** Options for `Editor.getKeyword`; at least one keyword is required. */
export interface PromptKeywordOptions {
  readonly message?: string;
  readonly keywords: readonly string[];
  readonly defaultKeyword?: string;
}

/** The editor for one document. Every member crosses the wire. */
export interface Editor {
  getPoint(options?: PromptPointOptions): Promise<PromptResult<Point3d>>;
  getEntity(options?: PromptEntityOptions): Promise<PromptResult<ObjectId>>;
  getSelection(options?: PromptSelectionOptions): Promise<PromptResult<SelectionSet>>;
  getString(options?: PromptStringOptions): Promise<PromptResult<string>>;
  getDouble(options?: PromptDoubleOptions): Promise<PromptResult<number>>;
  getInteger(options?: PromptIntegerOptions): Promise<PromptResult<number>>;
  getKeyword(options: PromptKeywordOptions): Promise<PromptResult<string>>;
  writeMessage(message: string): Promise<void>;
  selectAll(filter?: SelectionFilter): Promise<SelectionSet>;
  /**
   * The selection the user holds **right now**, without prompting.
   *
   * Gated on `EditorInput.editor.currentSelection` (served by `cad_api_call` in
   * this build). `objectIds` are the host's entity ids in selection order,
   * deduplicated by the host; an empty set is the real "nothing is selected", and
   * a host with no document refuses `cad_api.no_document` instead of answering an
   * empty set. Both answers reach the caller unchanged.
   */
  currentSelection(): Promise<SelectionSet>;
}

type Answer = Readonly<Record<string, unknown>>;

function requireString(record: Answer, field: string): string {
  const value = record[field];
  if (typeof value !== 'string') throw invalidArgument(`bridge.answer.${field}`);
  return value;
}

function requireNumber(record: Answer, field: string): number {
  const value = record[field];
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw invalidArgument(`bridge.answer.${field}`);
  }
  return value;
}

/** Map a host prompt answer to `PromptResult<T>`; unknown states are refused. */
function toPromptResult<T>(answer: unknown, read: (record: Answer) => T): PromptResult<T> {
  const record = requireAnswerObject(answer, 'bridge.answer');
  const status = record['status'];
  if (status === 'Cancel' || status === 'None') return { status };
  if (status === 'Error') return { status, message: requireString(record, 'message') };
  if (status === 'Keyword') return { status, keyword: requireString(record, 'keyword') };
  if (status === 'Ok') return { status, value: read(record) };
  throw invalidArgument('bridge.answer.status');
}

function readObjectIds(record: Answer): readonly ObjectId[] {
  const value = record['objectIds'];
  if (!Array.isArray(value)) throw invalidArgument('bridge.answer.objectIds');
  return value.map((entry, index) => {
    // The host sends decimal EntityId strings; anything else is a broken answer,
    // named by position so the host bug is diagnosable from the rejection.
    if (typeof entry !== 'string' || entry.trim().length === 0) {
      throw invalidArgument(`bridge.answer.objectIds[${index}]`);
    }
    return objectId(entry);
  });
}

function parametersOf(options: unknown, field: string): Readonly<Record<string, unknown>> {
  if (options === undefined) return {};
  if (!isRecord(options)) throw invalidArgument(field);
  return options;
}

async function ask<T>(
  wire: CadApiWire,
  operation: string,
  options: unknown,
  field: string,
  read: (record: Answer) => T,
): Promise<PromptResult<T>> {
  const parameters = parametersOf(options, field);
  const answer = await wire.call('EditorInput', operation, parameters);
  return toPromptResult(answer, read);
}

/** Build the editor proxy for one document. */
export function createEditor(wire: CadApiWire): Editor {
  return {
    getPoint(options?: PromptPointOptions): Promise<PromptResult<Point3d>> {
      return ask(wire, 'editor.getPoint', options, 'options', (record) => {
        const point = requireAnswerObject(record['point'], 'bridge.answer.point');
        return point3d(
          requireNumber(point, 'x'),
          requireNumber(point, 'y'),
          requireNumber(point, 'z'),
        );
      });
    },
    getEntity(options?: PromptEntityOptions): Promise<PromptResult<ObjectId>> {
      return ask(wire, 'editor.getEntity', options, 'options', (record) =>
        objectId(record['objectId']),
      );
    },
    getSelection(options?: PromptSelectionOptions): Promise<PromptResult<SelectionSet>> {
      return ask(wire, 'editor.getSelection', options, 'options', (record) =>
        selectionSet(readObjectIds(record)),
      );
    },
    getString(options?: PromptStringOptions): Promise<PromptResult<string>> {
      return ask(wire, 'editor.getString', options, 'options', (record) =>
        requireString(record, 'value'),
      );
    },
    getDouble(options?: PromptDoubleOptions): Promise<PromptResult<number>> {
      return ask(wire, 'editor.getDouble', options, 'options', (record) =>
        requireNumber(record, 'value'),
      );
    },
    getInteger(options?: PromptIntegerOptions): Promise<PromptResult<number>> {
      return ask(wire, 'editor.getInteger', options, 'options', (record) => {
        const value = requireNumber(record, 'value');
        if (!Number.isSafeInteger(value)) throw invalidArgument('bridge.answer.value');
        return value;
      });
    },
    getKeyword(options: PromptKeywordOptions): Promise<PromptResult<string>> {
      if (typeof options !== 'object' || options === null) throw invalidArgument('options');
      if (!Array.isArray(options.keywords) || options.keywords.length === 0) {
        throw invalidArgument('options.keywords');
      }
      return ask(wire, 'editor.getKeyword', options, 'options', (record) =>
        requireString(record, 'keyword'),
      );
    },
    async writeMessage(message: string): Promise<void> {
      if (typeof message !== 'string') throw invalidArgument('message');
      await wire.callVoid('EditorInput', 'editor.writeMessage', { message });
    },
    async selectAll(filter?: SelectionFilter): Promise<SelectionSet> {
      const parameters = parametersOf(filter, 'filter');
      const answer = await wire.call('EditorInput', 'editor.selectAll', parameters);
      return selectionSet(readObjectIds(requireAnswerObject(answer, 'bridge.answer')));
    },
    async currentSelection(): Promise<SelectionSet> {
      const answer = await wire.call('EditorInput', 'editor.currentSelection', {});
      return selectionSet(readObjectIds(requireAnswerObject(answer, 'bridge.answer')));
    },
  };
}
