// The DatabaseServices surface: object identity, the decoded object shapes, the
// named symbol tables, and the `Database` proxy.
//
// Two different things live here and the distinction matters:
//
//   * **Descriptions** (`DBObject`, `Entity`, `Line`, ...) are the shapes the
//     host returns from `transaction.getObject`. They are plain readonly data,
//     decoded and validated field by field; this package never constructs or
//     fabricates them.
//   * **Proxies** (`Database`) are live objects whose every member is
//     `Promise`-returning and goes through the capability gate *before* any
//     bridge call. In this build every one of those rows is `pending`, so the
//     proxy refuses by name — it never answers with a placeholder `Database`, an
//     empty symbol table, or a default layer.
//
// # Identity (never fabricate a handle)
//
// `ObjectId` and `Handle` are branded strings: opaque to a plugin, and mixing
// them up is a type error rather than a silent wrong lookup. `DBObject.handle` is
// the imported `source_handle` **provenance only** and is `null` for entities
// created through this API (ADR 0030 D1) — the decode accepts a missing/null
// handle and never synthesizes one.
//
// Layers and blocks are **keyed by name**. `cad-db` layers are `{ id, name,
// visible }` with no handle, and linetype is a symbolic/named concept; inventing
// a `Handle` for either would create a second identity system next to the
// database's own. So `Entity.layer` is a layer *name*, `Entity.linetype` is a
// `LinetypeRef` (byLayer / byBlock / named), and the symbol tables identify
// records by name with no handle at all.

import { invalidArgument } from './errors.ts';
import type { CadApiWire } from './internal/capability.ts';
import { isRecord, requireAnswerObject } from './internal/capability.ts';
import type { Color, EntityColor } from './colors.ts';
import type { Point3d } from './geometry.ts';
import { point3d } from './geometry.ts';

/** An opaque database object id (branded, so a plain string is not accepted). */
export type ObjectId = string & { readonly __objectId: unique symbol };

/** An opaque persistent handle (branded, distinct from `ObjectId`). */
export type Handle = string & { readonly __handle: unique symbol };

/** How a transaction opens an object. */
export type OpenMode = 'read' | 'write' | 'notify';

/**
 * How an entity resolves its linetype.
 *
 * Symbolic, like colour: a linetype is a named dash pattern, not a database
 * object with a handle. `named` carries the pattern name the host resolved.
 */
export type LinetypeRef =
  | { readonly method: 'byLayer' }
  | { readonly method: 'byBlock' }
  | { readonly method: 'named'; readonly name: string };

function requireIdentifier(value: unknown, field: string): string {
  if (typeof value !== 'string') throw invalidArgument(field);
  const trimmed = value.trim();
  if (trimmed.length === 0) throw invalidArgument(field);
  return trimmed;
}

/**
 * The runtime shape of the `ObjectId` brand: a validated, non-empty identifier.
 *
 * A declared type guard is how this module narrows without an `as` assertion —
 * the brand is a compile-time distinction over exactly this runtime contract.
 */
function isObjectId(value: string): value is ObjectId {
  return value.length > 0;
}

/** The runtime shape of the `Handle` brand (same contract, distinct type). */
function isHandle(value: string): value is Handle {
  return value.length > 0;
}

/** Brand a non-empty id string; nothing else is accepted. */
export function objectId(value: unknown): ObjectId {
  const text = requireIdentifier(value, 'objectId');
  if (!isObjectId(text)) throw invalidArgument('objectId');
  return text;
}

/** Brand a non-empty handle string; nothing else is accepted. */
export function handle(value: unknown): Handle {
  const text = requireIdentifier(value, 'handle');
  if (!isHandle(text)) throw invalidArgument('handle');
  return text;
}

/** The fields every database object shares. */
export interface DBObjectBase {
  readonly objectId: ObjectId;
  /**
   * The imported `source_handle`, or `null` when the object was created through
   * this API (or the host never had a handle for it). Never synthesized.
   */
  readonly handle: Handle | null;
  /** The host's type tag; concrete shapes narrow it to a literal. */
  readonly typeName: string;
}

/** A graphical object with the shared display attributes. */
export interface Entity extends DBObjectBase {
  /** The owning layer's **name** (not a handle; see the identity note above). */
  readonly layer: string;
  readonly color: EntityColor;
  readonly linetype: LinetypeRef;
}

/** An entity with a measurable length. */
export interface Curve extends Entity {
  readonly length: number;
}

/** A line segment. */
export interface Line extends Curve {
  readonly typeName: 'Line';
  readonly startPoint: Point3d;
  readonly endPoint: Point3d;
}

/** A full circle. */
export interface Circle extends Curve {
  readonly typeName: 'Circle';
  readonly center: Point3d;
  readonly radius: number;
}

/** A circular arc; angles are radians, counter-clockwise from +X. */
export interface Arc extends Curve {
  readonly typeName: 'Arc';
  readonly center: Point3d;
  readonly radius: number;
  readonly startAngle: number;
  readonly endAngle: number;
}

/** A lightweight polyline. */
export interface Polyline extends Curve {
  readonly typeName: 'Polyline';
  readonly numberOfVertices: number;
  readonly points: readonly Point3d[];
  readonly isClosed: boolean;
}

/** A single-line text object. */
export interface DBText extends Entity {
  readonly typeName: 'DBText';
  readonly textString: string;
  readonly position: Point3d;
  readonly height: number;
  readonly rotation: number;
}

/** A multiline text object. */
export interface MText extends Entity {
  readonly typeName: 'MText';
  readonly contents: string;
  readonly location: Point3d;
  readonly height: number;
}

/** The entity shapes this build can decode, keyed by the host's `typeName` tag. */
export interface EntityTypeMap {
  readonly Line: Line;
  readonly Circle: Circle;
  readonly Arc: Arc;
  readonly Polyline: Polyline;
  readonly DBText: DBText;
  readonly MText: MText;
}

/** One decodable entity tag (`'Line' | 'Circle' | ...`). */
export type EntityTypeName = keyof EntityTypeMap;

/** Any database object `transaction.getObject` can return in this build. */
export type DBObject = EntityTypeMap[EntityTypeName];

/**
 * A named record in a symbol table.
 *
 * Layers and blocks are keyed by name in `cad-db` terms and carry no handle, so
 * this base has none to inherit (see the identity note above).
 */
export interface SymbolTableRecord {
  readonly name: string;
}

/** A block definition, identified by name. */
export interface BlockTableRecord extends SymbolTableRecord {
  readonly isLayout: boolean;
  readonly origin: Point3d;
}

/** A layer definition, identified by name. */
export interface LayerTableRecord extends SymbolTableRecord {
  /**
   * The layer's resolved colour. `aci` is `null` for a true-colour layer — the
   * host reports no palette index there, and this type keeps it that way.
   */
  readonly color: Color;
  readonly visible: boolean;
  readonly locked: boolean;
  readonly frozen: boolean;
  /** The linetype **name** (a symbolic concept, not an object reference). */
  readonly linetype: string;
}

/** A symbol table keyed by record name. */
export interface SymbolTable<RecordType extends SymbolTableRecord = SymbolTableRecord> {
  readonly name: string;
  readonly records: readonly RecordType[];
  /** Every record name, in table order. */
  names(): readonly string[];
  /** One record by name, or `null` when the table has none — never a default record. */
  record(name: string): RecordType | null;
}

/** The block table. */
export interface BlockTable extends SymbolTable<BlockTableRecord> {
  readonly records: readonly BlockTableRecord[];
}

/** The layer table. */
export interface LayerTable extends SymbolTable<LayerTableRecord> {
  readonly records: readonly LayerTableRecord[];
}

/** The symbol tables reachable from one database. */
export interface SymbolTables {
  readonly blockTable: BlockTable;
  readonly layerTable: LayerTable;
}

/**
 * The live database proxy for one document.
 *
 * `id`/`version` are the host-supplied identity of the database (`id` is a host
 * database id, **not** a DXF handle; `version` is its decimal revision string);
 * every other member crosses the wire and is gated. `symbolTables` and `clayer`
 * share one round-trip (see `createDatabase`), because they are two views of the
 * same symbol-table snapshot.
 */
export interface Database {
  readonly id: string;
  readonly version: string;
  getObjectId(handle: Handle): Promise<ObjectId | null>;
  /**
   * The database's current revision, read from the host.
   *
   * Gated on `DatabaseServices.database.revision` (served by `cad_api_call` in
   * this build) and sent with `{ database: id }`. This is the value a commit's
   * `expectedRevision` is compared against, so it is read from the host every call
   * — never cached, and never derived from the `version` this proxy was built
   * with (that string is the snapshot the document was opened at).
   */
  revision(): Promise<number>;
  readonly symbolTables: Promise<SymbolTables>;
  /**
   * The current layer's **name**.
   *
   * The host may answer the empty string when its layer projection reports no
   * active layer: that is an honest "there is none", not a missing name, so it is
   * passed through rather than invented or refused.
   */
  readonly clayer: Promise<string>;
}

/** The host-supplied identity of a database (no data is derived from it here). */
export interface DatabaseDescriptor {
  /** The host database id (decimal), not a DXF handle. */
  readonly id: string;
  /** The database revision, as the host's decimal string. */
  readonly version: string;
}

interface SymbolTableSnapshot {
  readonly blockTable: BlockTable;
  readonly layerTable: LayerTable;
  readonly clayer: string;
}

function requireString(record: Readonly<Record<string, unknown>>, field: string): string {
  const value = record[field];
  if (typeof value !== 'string') throw invalidArgument(`bridge.answer.${field}`);
  return value;
}

/** A non-empty name (layer, block, linetype); blank is a malformed answer. */
function requireName(record: Readonly<Record<string, unknown>>, field: string): string {
  const value = requireString(record, field);
  if (value.trim().length === 0) throw invalidArgument(`bridge.answer.${field}`);
  return value;
}

function requireNumber(record: Readonly<Record<string, unknown>>, field: string): number {
  const value = record[field];
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw invalidArgument(`bridge.answer.${field}`);
  }
  return value;
}

function requireBoolean(record: Readonly<Record<string, unknown>>, field: string): boolean {
  const value = record[field];
  if (typeof value !== 'boolean') throw invalidArgument(`bridge.answer.${field}`);
  return value;
}

function requireRecord(
  record: Readonly<Record<string, unknown>>,
  field: string,
): Readonly<Record<string, unknown>> {
  const value = record[field];
  if (!isRecord(value)) throw invalidArgument(`bridge.answer.${field}`);
  return value;
}

function requireRecords(
  record: Readonly<Record<string, unknown>>,
  field: string,
): readonly Readonly<Record<string, unknown>>[] {
  const value = record[field];
  if (!Array.isArray(value)) throw invalidArgument(`bridge.answer.${field}`);
  return value.map((row) => requireAnswerObject(row, `bridge.answer.${field}`));
}

function requireComponent(value: unknown, field: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw invalidArgument(field);
  return value;
}

/**
 * Decode a 3D point.
 *
 * The wire uses a three-number array (`[x, y, z]` — the symbol-table `origin`
 * form) or an explicit `{ x, y, z }` record; both are unambiguous, and the entity
 * read has not fixed its own encoding yet, so both are accepted. A two-element
 * array, a fourth element, or a non-finite component is refused by field.
 */
function requirePoint(record: Readonly<Record<string, unknown>>, field: string): Point3d {
  const value = record[field];
  if (Array.isArray(value)) {
    if (value.length !== 3) throw invalidArgument(`bridge.answer.${field}`);
    return point3d(
      requireComponent(value[0], `bridge.answer.${field}[0]`),
      requireComponent(value[1], `bridge.answer.${field}[1]`),
      requireComponent(value[2], `bridge.answer.${field}[2]`),
    );
  }
  if (!isRecord(value)) throw invalidArgument(`bridge.answer.${field}`);
  return point3d(
    requireComponent(value['x'], `bridge.answer.${field}.x`),
    requireComponent(value['y'], `bridge.answer.${field}.y`),
    requireComponent(value['z'], `bridge.answer.${field}.z`),
  );
}

function requireEntityColor(
  record: Readonly<Record<string, unknown>>,
  field: string,
): EntityColor {
  const color = requireRecord(record, field);
  const method = color['method'];
  if (method === 'byLayer' || method === 'byBlock') return { method };
  if (method === 'byAci') {
    const aci = color['aci'];
    if (typeof aci !== 'number' || !Number.isSafeInteger(aci)) {
      throw invalidArgument(`bridge.answer.${field}.aci`);
    }
    return { method, aci };
  }
  if (method === 'byColor') {
    const rgb = color['rgb'];
    if (typeof rgb !== 'string') throw invalidArgument(`bridge.answer.${field}.rgb`);
    return { method, rgb };
  }
  throw invalidArgument(`bridge.answer.${field}.method`);
}

/**
 * Decode a layer's resolved colour: `{ aci, rgb }`.
 *
 * `aci` is a palette index or `null` for a true-colour layer. The host sends
 * `null` there and this decoder keeps it exactly; an absent `aci` means the same
 * thing ("no index") and becomes `null` — an invented index would be
 * indistinguishable from a real one. `rgb` is the host's `#rrggbb`.
 */
function requireLayerColor(
  record: Readonly<Record<string, unknown>>,
  field: string,
): Color {
  const color = requireRecord(record, field);
  const rawAci = color['aci'];
  let aci: number | null = null;
  if (rawAci !== null && rawAci !== undefined) {
    // The host carries `Option<u8>`; 0..256 is the ACI domain, so anything else
    // (or a non-integer) is a broken answer rather than a colour.
    if (
      typeof rawAci !== 'number' ||
      !Number.isSafeInteger(rawAci) ||
      rawAci < 0 ||
      rawAci > 256
    ) {
      throw invalidArgument(`bridge.answer.${field}.aci`);
    }
    aci = rawAci;
  }
  const rgb = color['rgb'];
  if (typeof rgb !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(rgb)) {
    throw invalidArgument(`bridge.answer.${field}.rgb`);
  }
  return { aci, rgb };
}

function requireLinetype(
  record: Readonly<Record<string, unknown>>,
  field: string,
): LinetypeRef {
  const linetype = requireRecord(record, field);
  const method = linetype['method'];
  if (method === 'byLayer' || method === 'byBlock') return { method };
  if (method === 'named') return { method, name: requireName(linetype, 'name') };
  throw invalidArgument(`bridge.answer.${field}.method`);
}

/** Shared identity: a missing or null `handle` stays `null`, it is never minted. */
function decodeIdentity(record: Readonly<Record<string, unknown>>): {
  readonly objectId: ObjectId;
  readonly handle: Handle | null;
} {
  const rawHandle = record['handle'];
  let decodedHandle: Handle | null = null;
  if (rawHandle !== undefined && rawHandle !== null) {
    if (typeof rawHandle !== 'string' || rawHandle.trim().length === 0) {
      throw invalidArgument('bridge.answer.handle');
    }
    decodedHandle = handle(rawHandle);
  }
  return { objectId: objectId(requireString(record, 'objectId')), handle: decodedHandle };
}

function decodeEntityFields(record: Readonly<Record<string, unknown>>): {
  readonly objectId: ObjectId;
  readonly handle: Handle | null;
  readonly layer: string;
  readonly color: EntityColor;
  readonly linetype: LinetypeRef;
} {
  return {
    ...decodeIdentity(record),
    layer: requireName(record, 'layer'),
    color: requireEntityColor(record, 'color'),
    linetype: requireLinetype(record, 'linetype'),
  };
}

function decodeLine(record: Readonly<Record<string, unknown>>): Line {
  return {
    ...decodeEntityFields(record),
    typeName: 'Line',
    startPoint: requirePoint(record, 'startPoint'),
    endPoint: requirePoint(record, 'endPoint'),
    length: requireNumber(record, 'length'),
  };
}

function decodeCircle(record: Readonly<Record<string, unknown>>): Circle {
  return {
    ...decodeEntityFields(record),
    typeName: 'Circle',
    center: requirePoint(record, 'center'),
    radius: requireNumber(record, 'radius'),
    length: requireNumber(record, 'length'),
  };
}

function decodeArc(record: Readonly<Record<string, unknown>>): Arc {
  return {
    ...decodeEntityFields(record),
    typeName: 'Arc',
    center: requirePoint(record, 'center'),
    radius: requireNumber(record, 'radius'),
    startAngle: requireNumber(record, 'startAngle'),
    endAngle: requireNumber(record, 'endAngle'),
    length: requireNumber(record, 'length'),
  };
}

function decodePolyline(record: Readonly<Record<string, unknown>>): Polyline {
  const points = requireRecords(record, 'points').map((point, index) =>
    requirePoint(point, `points[${index}]`),
  );
  return {
    ...decodeEntityFields(record),
    typeName: 'Polyline',
    numberOfVertices: requireNumber(record, 'numberOfVertices'),
    points,
    isClosed: requireBoolean(record, 'isClosed'),
    length: requireNumber(record, 'length'),
  };
}

function decodeDBText(record: Readonly<Record<string, unknown>>): DBText {
  return {
    ...decodeEntityFields(record),
    typeName: 'DBText',
    textString: requireString(record, 'textString'),
    position: requirePoint(record, 'position'),
    height: requireNumber(record, 'height'),
    rotation: requireNumber(record, 'rotation'),
  };
}

function decodeMText(record: Readonly<Record<string, unknown>>): MText {
  return {
    ...decodeEntityFields(record),
    typeName: 'MText',
    contents: requireString(record, 'contents'),
    location: requirePoint(record, 'location'),
    height: requireNumber(record, 'height'),
  };
}

/**
 * Decode one host object by its `typeName` tag.
 *
 * There is no caller-chosen type parameter and no cast: the host's tag picks the
 * decoder, and an unknown or missing tag is `api.invalid_argument:
 * bridge.answer.typeName`. A plugin narrows the returned union on `typeName`.
 */
export function decodeObject(record: Readonly<Record<string, unknown>>): DBObject {
  const typeName = record['typeName'];
  if (typeof typeName !== 'string' || typeName.length === 0) {
    throw invalidArgument('bridge.answer.typeName');
  }
  switch (typeName) {
    case 'Line':
      return decodeLine(record);
    case 'Circle':
      return decodeCircle(record);
    case 'Arc':
      return decodeArc(record);
    case 'Polyline':
      return decodePolyline(record);
    case 'DBText':
      return decodeDBText(record);
    case 'MText':
      return decodeMText(record);
    default:
      throw invalidArgument('bridge.answer.typeName');
  }
}

function decodeLayerTableRecord(record: Readonly<Record<string, unknown>>): LayerTableRecord {
  return {
    name: requireName(record, 'name'),
    color: requireLayerColor(record, 'color'),
    visible: requireBoolean(record, 'visible'),
    locked: requireBoolean(record, 'locked'),
    frozen: requireBoolean(record, 'frozen'),
    linetype: requireName(record, 'linetype'),
  };
}

function decodeBlockTableRecord(record: Readonly<Record<string, unknown>>): BlockTableRecord {
  return {
    name: requireName(record, 'name'),
    isLayout: requireBoolean(record, 'isLayout'),
    origin: requirePoint(record, 'origin'),
  };
}

function keyedTable<RecordType extends SymbolTableRecord>(
  name: string,
  records: readonly RecordType[],
): SymbolTable<RecordType> {
  const frozen = Object.freeze([...records]);
  return {
    name,
    records: frozen,
    names: () => frozen.map((row) => row.name),
    // A missing name is `null`, never a default record: a fabricated layer would
    // be indistinguishable from a real one.
    record: (key: string) => frozen.find((row) => row.name === key) ?? null,
  };
}

function decodeSymbolTable<RecordType extends SymbolTableRecord>(
  record: Readonly<Record<string, unknown>>,
  decodeRecord: (row: Readonly<Record<string, unknown>>) => RecordType,
): SymbolTable<RecordType> {
  return keyedTable(requireName(record, 'name'), requireRecords(record, 'records').map(decodeRecord));
}

/**
 * Build the `Database` proxy for one document handle.
 *
 * `symbolTables` and `clayer` share a single `database.symbolTables` answer,
 * cached per proxy. The cache holds the host's decode of that answer — never a
 * locally invented table — so a second read cannot quietly disagree with the
 * first, and a refused answer is not cached (the next read asks again).
 */
export function createDatabase(wire: CadApiWire, descriptor: DatabaseDescriptor): Database {
  if (typeof descriptor !== 'object' || descriptor === null) throw invalidArgument('descriptor');
  const databaseId = requireIdentifier(descriptor.id, 'descriptor.id');
  if (typeof descriptor.version !== 'string' || descriptor.version.trim().length === 0) {
    throw invalidArgument('descriptor.version');
  }

  let snapshot: Promise<SymbolTableSnapshot> | undefined;

  const loadSnapshot = (): Promise<SymbolTableSnapshot> => {
    if (snapshot !== undefined) return snapshot;
    const pending = (async (): Promise<SymbolTableSnapshot> => {
      const answer = await wire.call('DatabaseServices', 'database.symbolTables', {
        database: databaseId,
      });
      const record = requireAnswerObject(answer, 'bridge.answer');
      return {
        blockTable: decodeSymbolTable(requireRecord(record, 'blockTable'), decodeBlockTableRecord),
        layerTable: decodeSymbolTable(requireRecord(record, 'layerTable'), decodeLayerTableRecord),
        clayer: requireString(record, 'clayer'),
      };
    })();
    snapshot = pending.catch((failure: unknown) => {
      snapshot = undefined;
      throw failure;
    });
    return snapshot;
  };

  return {
    id: databaseId,
    version: descriptor.version.trim(),
    async getObjectId(value: Handle): Promise<ObjectId | null> {
      const answer = await wire.call('DatabaseServices', 'database.getObjectId', {
        handle: value,
      });
      const record = requireAnswerObject(answer, 'bridge.answer');
      const id = record['objectId'];
      if (id === null) return null;
      if (typeof id !== 'string') throw invalidArgument('bridge.answer.objectId');
      return objectId(id);
    },
    async revision(): Promise<number> {
      const answer = await wire.call('DatabaseServices', 'database.revision', {
        database: databaseId,
      });
      const record = requireAnswerObject(answer, 'bridge.answer');
      const revision = record['revision'];
      if (
        typeof revision !== 'number' ||
        !Number.isFinite(revision) ||
        !Number.isInteger(revision) ||
        revision < 0
      ) {
        throw invalidArgument('bridge.answer.revision');
      }
      return revision;
    },
    get symbolTables(): Promise<SymbolTables> {
      return loadSnapshot().then(({ blockTable, layerTable }) => ({ blockTable, layerTable }));
    },
    get clayer(): Promise<string> {
      return loadSnapshot().then(({ clayer }) => clayer);
    },
  };
}
