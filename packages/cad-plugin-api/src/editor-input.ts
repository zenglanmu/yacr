// The EditorInput surface: user prompts and picking.
//
// Every member is `Promise`-returning and capability-gated before the host is
// asked, and no member ever invents an answer.
//
// # Two lanes, one namespace
//
// * The **prompts** (`getPoint`, `getEntity`, `getSelection`, `getString`,
//   `getDouble`, `getInteger`, `getKeyword`) and the immediate `selectAll` read
//   use the **mutating** lane (`wire.submit`): a prompt completes only when the
//   user answers it, so the answer arrives on a later poll, not in one synchronous
//   reply. `PromptStatus.None` is only ever the host's own "Enter with no default"
//   — never this client's guess at a cancelled prompt.
// * `currentSelection()` is a reflected read on the synchronous lane: the
//   selection the user has right now (not a prompt).
// * `writeMessage()` is **client-local**: it hands the text to the host's message
//   sink (`CadApiOptions.onMessage`). There is no wire call and no descriptor row;
//   without a sink it refuses with
//   `unsupported-runtime: cad_api.EditorInput.editor.writeMessage`.
//
// # What the prompt wire carries
//
// The prompt body is exactly `message`, `keywords`, `defaultValue` and
// `rejectMessage`, present only when the caller set them. The other option fields
// are part of the AutoCAD-shaped surface but are **not transmitted** by this
// contract yet (they are recorded in the interfaces and ignored on the wire), so
// none of them changes what the host asks.
//
// A host answer is validated: an unknown `status` is `api.invalid_argument:
// bridge.answer`, never coerced into one of the five known states.

import type { ObjectId } from './database-services.ts';
import { objectId } from './database-services.ts';
import { invalidArgument } from './errors.ts';
import type { Point3d } from './geometry.ts';
import { point3d } from './geometry.ts';
import type { CadApiWire } from './internal/capability.ts';
import { isRecord, requireAnswerObject } from './internal/capability.ts';
import { CadApiError } from './errors.ts';
import { unsupportedReason } from './abi.ts';

/** What an `Editor` needs beyond the wire. */
export interface EditorOptions {
  /** The host's plugin message sink; absent means `writeMessage` refuses. */
  readonly onMessage?: (text: string) => void;
}

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

/**
 * Options for `Editor.getPoint`.
 *
 * Only `message` is transmitted; `basePoint`/`useBasePoint` are part of the
 * AutoCAD shape and are not part of this contract's prompt wire yet.
 */
export interface PromptPointOptions {
  readonly message?: string;
  readonly basePoint?: Point3d;
  readonly useBasePoint?: boolean;
}

/**
 * Options for `Editor.getEntity`.
 *
 * `message` and `rejectMessage` are transmitted; `allowNone` is not part of this
 * contract's prompt wire yet.
 */
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

/**
 * Options for `Editor.getSelection`.
 *
 * Only `message` is transmitted; `singleOnly`/`filter` are not part of this
 * contract's prompt wire yet.
 */
export interface PromptSelectionOptions {
  readonly message?: string;
  readonly singleOnly?: boolean;
  readonly filter?: SelectionFilter;
}

/** Options for `Editor.getString`; `message` and `defaultValue` are transmitted. */
export interface PromptStringOptions {
  readonly message?: string;
  readonly allowSpaces?: boolean;
  readonly defaultValue?: string;
}

/** Options for `Editor.getDouble`; `message` and `defaultValue` are transmitted. */
export interface PromptDoubleOptions {
  readonly message?: string;
  readonly defaultValue?: number;
  readonly minimum?: number;
  readonly maximum?: number;
}

/** Options for `Editor.getInteger`; `message` and `defaultValue` are transmitted. */
export interface PromptIntegerOptions {
  readonly message?: string;
  readonly defaultValue?: number;
  readonly minimum?: number;
  readonly maximum?: number;
}

/**
 * Options for `Editor.getKeyword`; at least one keyword is required.
 *
 * `message` and `keywords` are transmitted. `defaultKeyword` is not part of this
 * contract's prompt wire yet (the host's keyword prompt has no Enter default).
 */
export interface PromptKeywordOptions {
  readonly message?: string;
  readonly keywords: readonly string[];
  readonly defaultKeyword?: string;
}

/**
 * The editor for one document.
 *
 * Prompts and `selectAll` cross the wire (submit lane); `currentSelection` is a
 * reflected read; `writeMessage` is local to the host's message sink.
 */
export interface Editor {
  getPoint(options?: PromptPointOptions): Promise<PromptResult<Point3d>>;
  getEntity(options?: PromptEntityOptions): Promise<PromptResult<ObjectId>>;
  getSelection(options?: PromptSelectionOptions): Promise<PromptResult<SelectionSet>>;
  getString(options?: PromptStringOptions): Promise<PromptResult<string>>;
  getDouble(options?: PromptDoubleOptions): Promise<PromptResult<number>>;
  getInteger(options?: PromptIntegerOptions): Promise<PromptResult<number>>;
  getKeyword(options: PromptKeywordOptions): Promise<PromptResult<string>>;
  writeMessage(message: string): Promise<void>;
  /**
   * Every object in the drawing, immediately (no prompt).
   *
   * Goes over the submit lane; the host's answer is the prompt envelope
   * (`{status: "Ok", objectIds}`). The optional filter is part of the AutoCAD
   * shape and is not part of this contract's prompt wire yet.
   */
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

/** The wire parameters a prompt can carry; every field is optional. */
interface PromptWireOptions {
  readonly message?: unknown;
  readonly keywords?: unknown;
  readonly defaultValue?: unknown;
  readonly rejectMessage?: unknown;
}

/**
 * Project one prompt's options onto the wire body.
 *
 * Exactly four keys travel (`message`, `keywords`, `defaultValue`,
 * `rejectMessage`) and each is present only when the caller set it. A value of
 * the wrong shape is a caller error here, instead of something the host has to
 * guess at from a JSON body.
 */
function promptParameters(
  options: PromptWireOptions | undefined,
): Readonly<Record<string, unknown>> {
  if (options === undefined) return {};
  if (!isRecord(options)) throw invalidArgument('options');
  const parameters: Record<string, unknown> = {};

  const message = options.message;
  if (message !== undefined) {
    if (typeof message !== 'string') throw invalidArgument('options.message');
    parameters['message'] = message;
  }
  const rejectMessage = options.rejectMessage;
  if (rejectMessage !== undefined) {
    if (typeof rejectMessage !== 'string') throw invalidArgument('options.rejectMessage');
    parameters['rejectMessage'] = rejectMessage;
  }
  const keywords = options.keywords;
  if (keywords !== undefined) {
    if (!Array.isArray(keywords) || keywords.length === 0) {
      throw invalidArgument('options.keywords');
    }
    for (const keyword of keywords) {
      if (typeof keyword !== 'string' || keyword.trim().length === 0) {
        throw invalidArgument('options.keywords');
      }
    }
    parameters['keywords'] = keywords;
  }
  const defaultValue = options.defaultValue;
  if (defaultValue !== undefined) {
    if (typeof defaultValue !== 'string' && typeof defaultValue !== 'number') {
      throw invalidArgument('options.defaultValue');
    }
    parameters['defaultValue'] = defaultValue;
  }
  return parameters;
}

/**
 * Start one prompt and decode its answer.
 *
 * The **submit** lane is deliberate: a prompt finishes only when the user
 * answers, so the answer is polled instead of arriving in one synchronous reply.
 */
async function ask<T>(
  wire: CadApiWire,
  operation: string,
  options: PromptWireOptions | undefined,
  read: (record: Answer) => T,
): Promise<PromptResult<T>> {
  const parameters = promptParameters(options);
  const answer = await wire.submit('EditorInput', operation, parameters);
  return toPromptResult(answer, read);
}

/**
 * Build the editor proxy for one document.
 *
 * `options.onMessage` is the host's plugin message sink: `writeMessage` is
 * client-local, so without a sink the call refuses instead of pretending the text
 * went somewhere.
 */
export function createEditor(wire: CadApiWire, options: EditorOptions = {}): Editor {
  return {
    getPoint(options?: PromptPointOptions): Promise<PromptResult<Point3d>> {
      return ask(wire, 'editor.getPoint', options, (record) => {
        const point = requireAnswerObject(record['point'], 'bridge.answer.point');
        return point3d(
          requireNumber(point, 'x'),
          requireNumber(point, 'y'),
          requireNumber(point, 'z'),
        );
      });
    },
    getEntity(options?: PromptEntityOptions): Promise<PromptResult<ObjectId>> {
      return ask(wire, 'editor.getEntity', options, (record) =>
        objectId(record['objectId']),
      );
    },
    getSelection(options?: PromptSelectionOptions): Promise<PromptResult<SelectionSet>> {
      return ask(wire, 'editor.getSelection', options, (record) =>
        selectionSet(readObjectIds(record)),
      );
    },
    getString(options?: PromptStringOptions): Promise<PromptResult<string>> {
      return ask(wire, 'editor.getString', options, (record) =>
        requireString(record, 'value'),
      );
    },
    getDouble(options?: PromptDoubleOptions): Promise<PromptResult<number>> {
      return ask(wire, 'editor.getDouble', options, (record) =>
        requireNumber(record, 'value'),
      );
    },
    getInteger(options?: PromptIntegerOptions): Promise<PromptResult<number>> {
      return ask(wire, 'editor.getInteger', options, (record) => {
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
      return ask(wire, 'editor.getKeyword', options, (record) =>
        requireString(record, 'keyword'),
      );
    },
    async writeMessage(message: string): Promise<void> {
      if (typeof message !== 'string') throw invalidArgument('message');
      const sink = options.onMessage;
      if (sink === undefined) {
        // No funnel to write into: refusing is the only honest answer.
        throw new CadApiError(
          unsupportedReason('EditorInput', 'editor.writeMessage'),
          'this host has no plugin message sink',
        );
      }
      sink(message);
    },
    async selectAll(_filter?: SelectionFilter): Promise<SelectionSet> {
      // Immediate, but on the submit lane like every other EditorInput answer;
      // the body carries no arguments (the filter is not part of the wire yet).
      const answer = await wire.submit('EditorInput', 'editor.selectAll', {});
      const record = requireAnswerObject(answer, 'bridge.answer');
      if (record['status'] !== 'Ok') throw invalidArgument('bridge.answer.status');
      return selectionSet(readObjectIds(record));
    },
    async currentSelection(): Promise<SelectionSet> {
      const answer = await wire.call('EditorInput', 'editor.currentSelection', {});
      return selectionSet(readObjectIds(requireAnswerObject(answer, 'bridge.answer')));
    },
  };
}
