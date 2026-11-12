// `@yacr/plugin-api` — the public surface.
//
// This is the **only** barrel in the package: every other module is imported by
// its concrete path (with the `.ts` extension), and nothing here is a
// convenience re-export of internals. `src/internal/capability.ts` is
// deliberately absent — the gated transport is not plugin surface.
//
// Everything exported here is either a frozen value (the contract table, the
// prompt/colour method maps, the creatable-type set), a pure value helper
// (Geometry, Colors, ids), an error type, a type, or a runtime entry point
// (`definePlugin`, `parsePluginManifest`, `createCadApi`).

export {
  API_NAMESPACES,
  API_OPERATIONS,
  API_VERSION,
  descriptorMissingReason,
  operationOf,
  pendingReason,
  unsupportedReason,
} from './abi.ts';
export type {
  CadApiNamespace,
  CadApiOperation,
  CadApiOperationStatus,
} from './abi.ts';

export { CadApiError, errorCodeOf, isCadApiError } from './errors.ts';
export type { CadApiErrorInit } from './errors.ts';

export type { CadApiHostInfo, CadApiRequest, CadHostBridge } from './host-bridge.ts';

export { createCadApi, definePlugin, parsePluginManifest } from './context.ts';
export type { CadApi, CadPlugin, PluginContext, PluginManifest } from './context.ts';

export {
  addPoints,
  createGeometryLibrary,
  distance3d,
  matrix3dIdentity,
  matrix3dRotateZ,
  matrix3dScale,
  matrix3dTranslate,
  planeFromOriginNormal,
  point2d,
  point3d,
  transformPoint,
  vector2d,
  vector3d,
} from './geometry.ts';
export type {
  GeometryLibrary,
  Matrix3d,
  Plane,
  Point2d,
  Point3d,
  Vector2d,
  Vector3d,
} from './geometry.ts';

export { aciToRgb, ColorMethod, colorFromAci, createColorLibrary } from './colors.ts';
export type { Color, ColorLibrary, EntityColor } from './colors.ts';

export { handle, objectId } from './database-services.ts';
export type {
  Arc,
  BlockTable,
  BlockTableRecord,
  Circle,
  Curve,
  Database,
  DatabaseDescriptor,
  DBObject,
  DBObjectBase,
  DBText,
  Entity,
  EntityTypeMap,
  EntityTypeName,
  Handle,
  LayerTable,
  LayerTableRecord,
  Line,
  LinetypeRef,
  MText,
  ObjectId,
  OpenMode,
  Polyline,
  SymbolTable,
  SymbolTableRecord,
  SymbolTables,
} from './database-services.ts';

export {
  MAX_TRANSACTION_DEPTH,
  SUPPORTED_NEW_ENTITY_TYPES,
  Transaction,
  TransactionManager,
} from './transaction.ts';
export type {
  CommitResult,
  NewlyCreatedDBObject,
  NewlyCreatedEntityType,
  TransactionEdit,
  TransactionOptions,
} from './transaction.ts';

export { PromptStatus, selectionSet } from './editor-input.ts';
export type {
  Editor,
  PromptDoubleOptions,
  PromptEntityOptions,
  PromptIntegerOptions,
  PromptKeywordOptions,
  PromptPointOptions,
  PromptResult,
  PromptSelectionOptions,
  PromptStringOptions,
  SelectionFilter,
  SelectionSet,
} from './editor-input.ts';

export type { Application, Document, DocumentManager } from './application-services.ts';
