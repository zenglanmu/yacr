// The ApplicationServices surface: the application, its documents, and the
// per-document wiring (database, editor, transaction manager).
//
// # Why the document manager is async
//
// AutoCAD exposes `DocumentManager.MdiActiveDocument` as a **property**. Here it
// is a method returning a `Promise`, because the value comes from the host over an
// asynchronous wire (`cad_api_call`). A synchronous accessor could only be honest
// by serving a stale cache, and a stale "which document am I editing" is exactly
// the kind of answer this package refuses to invent. So: `mdiActiveDocument()` and
// `documents()` are methods, `await` them, and a refusal is a rejection — never a
// silent `null`.
//
// Both document reads are served by this build: `mdiActiveDocument()` decodes
// `{ document: null | descriptor }` and `documents()` decodes
// `{ documents: [descriptor] }` (0 or 1 today; an empty list is the honest "no
// document is open"). A descriptor is
// `{ id, name, isReadOnly, database: { id, version } }`. A malformed answer is
// `api.invalid_argument: bridge.answer.document.*` / `.documents.*`; it is never
// coerced into a document, an empty list, or `null`.

import { createDatabase } from './database-services.ts';
import type { Database } from './database-services.ts';
import { createEditor } from './editor-input.ts';
import type { Editor, EditorOptions } from './editor-input.ts';
import { invalidArgument } from './errors.ts';
import type { CadApiWire } from './internal/capability.ts';
import { isRecord, requireAnswerObject } from './internal/capability.ts';
import { TransactionManager } from './transaction.ts';

/** The application singleton: what a plugin gets from `PluginContext.api`. */
export interface Application {
  readonly documentManager: DocumentManager;
}

/** The open drawing collection. */
export interface DocumentManager {
  /**
   * The document the user is working in, or `null` when none is open.
   *
   * Gated on `ApplicationServices.documentManager.mdiActiveDocument` (served by
   * `cad_api_call` in this build). `null` is a real answer — "a document manager
   * exists and holds no document" — not a placeholder; a host with **no snapshot
   * yet** refuses with `cad_waiting` instead, and that rejection reaches the caller
   * unchanged.
   */
  mdiActiveDocument(): Promise<Document | null>;
  /**
   * Every open document.
   *
   * Gated on `ApplicationServices.documentManager.documents` (served by
   * `cad_api_call` in this build). An empty list is a real answer — the document
   * manager exists and holds no document — not a placeholder; a host with no
   * snapshot yet refuses with `cad_waiting` instead.
   */
  documents(): Promise<readonly Document[]>;
  /**
   * Run a callback in application context.
   *
   * Local by design: the JS event loop *is* the application context for a plugin,
   * so this is a scheduling helper, not a host round-trip.
   */
  executeInApplicationContext<T>(work: () => T | Promise<T>): Promise<T>;
}

/** One open document and its per-document services. */
export interface Document {
  readonly name: string;
  readonly isReadOnly: boolean;
  readonly database: Database;
  readonly editor: Editor;
  readonly transactionManager: TransactionManager;
  /** Take a document lock for a long-running edit; gated on `document.lock`. */
  lockDocument(): Promise<void>;
}

/**
 * The host-supplied identity of a document; no data is derived from it here.
 *
 * `id` is what every commit envelope carries (`{ document, transactionId, ... }`),
 * so the manager, the database and the lock call all address the same document.
 * The database identity is its own `{ id, version }`: a database `id` is a host
 * id, **not** a DXF handle, and `version` is the host's decimal revision string.
 */
export interface DocumentDescriptor {
  readonly id: string;
  readonly name: string;
  readonly isReadOnly: boolean;
  readonly database: {
    readonly id: string;
    readonly version: string;
  };
}

function requireText(value: unknown, field: string): string {
  if (typeof value !== 'string' || value.trim().length === 0) throw invalidArgument(field);
  return value.trim();
}

/**
 * Decode one document descriptor.
 *
 * `prefix` is the wire path the value came from, so every failure names the field
 * (`bridge.answer.document.id`, `bridge.answer.documents.database.id`, ...) and a
 * host bug is diagnosable from the rejection alone. Nothing is defaulted and
 * nothing is cast.
 */
function decodeDocumentDescriptor(value: unknown, prefix: string): DocumentDescriptor {
  if (!isRecord(value)) throw invalidArgument(prefix);
  const id = requireText(value['id'], `${prefix}.id`);
  const name = requireText(value['name'], `${prefix}.name`);
  const isReadOnly = value['isReadOnly'];
  if (typeof isReadOnly !== 'boolean') throw invalidArgument(`${prefix}.isReadOnly`);
  const database = value['database'];
  if (!isRecord(database)) throw invalidArgument(`${prefix}.database`);
  return {
    id,
    name,
    isReadOnly,
    database: {
      id: requireText(database['id'], `${prefix}.database.id`),
      version: requireText(database['version'], `${prefix}.database.version`),
    },
  };
}

/**
 * Materialize a document from a host descriptor.
 *
 * The document's own `id` names the transaction scope; `database.id` names the
 * database the `Database` proxy addresses (never a DXF handle).
 */
export function createDocument(
  wire: CadApiWire,
  descriptor: DocumentDescriptor,
  editorOptions: EditorOptions = {},
): Document {
  if (typeof descriptor !== 'object' || descriptor === null) throw invalidArgument('descriptor');
  const documentId = requireText(descriptor.id, 'descriptor.id');
  const name = requireText(descriptor.name, 'descriptor.name');
  if (typeof descriptor.isReadOnly !== 'boolean') throw invalidArgument('descriptor.isReadOnly');
  const database = descriptor.database;
  if (typeof database !== 'object' || database === null) throw invalidArgument('descriptor.database');
  const databaseId = requireText(database.id, 'descriptor.database.id');
  const version = requireText(database.version, 'descriptor.database.version');
  return {
    name,
    isReadOnly: descriptor.isReadOnly,
    database: createDatabase(wire, { id: databaseId, version }),
    editor: createEditor(wire, editorOptions),
    transactionManager: new TransactionManager(wire, documentId),
    async lockDocument(): Promise<void> {
      await wire.callVoid('ApplicationServices', 'document.lock', { document: documentId });
    },
  };
}

/**
 * Build the document manager for one host snapshot.
 *
 * Both reads are served in this build: gate, call, decode. A refusal
 * (`cad_waiting`, `unsupported-runtime: ...`) or a malformed answer propagates; an
 * empty document list is the real "no document is open" answer.
 */
export function createDocumentManager(
  wire: CadApiWire,
  editorOptions: EditorOptions = {},
): DocumentManager {
  return {
    async mdiActiveDocument(): Promise<Document | null> {
      const answer = await wire.call(
        'ApplicationServices',
        'documentManager.mdiActiveDocument',
        {},
      );
      const record = requireAnswerObject(answer, 'bridge.answer');
      const document = record['document'];
      if (document === undefined) throw invalidArgument('bridge.answer.document');
      if (document === null) return null;
      return createDocument(
        wire,
        decodeDocumentDescriptor(document, 'bridge.answer.document'),
        editorOptions,
      );
    },
    async documents(): Promise<readonly Document[]> {
      const answer = await wire.call('ApplicationServices', 'documentManager.documents', {});
      const record = requireAnswerObject(answer, 'bridge.answer');
      const list = record['documents'];
      if (!Array.isArray(list)) throw invalidArgument('bridge.answer.documents');
      return list.map((entry) =>
        createDocument(
          wire,
          decodeDocumentDescriptor(entry, 'bridge.answer.documents'),
          editorOptions,
        ),
      );
    },
    async executeInApplicationContext<T>(work: () => T | Promise<T>): Promise<T> {
      if (typeof work !== 'function') throw invalidArgument('work');
      return await work();
    },
  };
}

/** Build the application singleton for one host snapshot. */
export function createApplication(
  wire: CadApiWire,
  editorOptions: EditorOptions = {},
): Application {
  return { documentManager: createDocumentManager(wire, editorOptions) };
}
