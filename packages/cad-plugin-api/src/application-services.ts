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
// `documents()` has no wire decoder yet (its row is still `pending`), so it is
// gated and refuses by name. `mdiActiveDocument()` is the first operation this
// build actually serves: it gates the row, calls the host, and decodes
// `{ document: null | { id, name, isReadOnly, database: { id, version } } }`.
// A malformed answer is `api.invalid_argument: bridge.answer.document.*`; it is
// never coerced into a document or into `null`.

import { unsupportedReason } from './abi.ts';
import { createDatabase } from './database-services.ts';
import type { Database } from './database-services.ts';
import { createEditor } from './editor-input.ts';
import type { Editor } from './editor-input.ts';
import { CadApiError, invalidArgument } from './errors.ts';
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
  /** Every open document. Still gated on a `pending` row, so it refuses by name. */
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
 * Decode the `mdiActiveDocument` payload's `document` member.
 *
 * Every failure names the field (`bridge.answer.document.id`, and so on) so a host bug is
 * diagnosable from the rejection alone; nothing is defaulted and nothing is cast.
 */
function decodeDocumentDescriptor(value: unknown): DocumentDescriptor {
  if (!isRecord(value)) throw invalidArgument('bridge.answer.document');
  const id = requireText(value['id'], 'bridge.answer.document.id');
  const name = requireText(value['name'], 'bridge.answer.document.name');
  const isReadOnly = value['isReadOnly'];
  if (typeof isReadOnly !== 'boolean') {
    throw invalidArgument('bridge.answer.document.isReadOnly');
  }
  const database = value['database'];
  if (!isRecord(database)) throw invalidArgument('bridge.answer.document.database');
  return {
    id,
    name,
    isReadOnly,
    database: {
      id: requireText(database['id'], 'bridge.answer.document.database.id'),
      version: requireText(database['version'], 'bridge.answer.document.database.version'),
    },
  };
}

/** A supported `documents` row needs a list decoder this client does not have yet. */
function noDocumentListDecoder(): CadApiError {
  return new CadApiError(
    unsupportedReason('ApplicationServices', 'documentManager.documents'),
    'this client has no document-list decoder yet; only the active document is served',
  );
}

/**
 * Materialize a document from a host descriptor.
 *
 * The document's own `id` names the transaction scope; `database.id` names the
 * database the `Database` proxy addresses (never a DXF handle).
 */
export function createDocument(wire: CadApiWire, descriptor: DocumentDescriptor): Document {
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
    editor: createEditor(wire),
    transactionManager: new TransactionManager(wire, documentId),
    async lockDocument(): Promise<void> {
      await wire.callVoid('ApplicationServices', 'document.lock', { document: documentId });
    },
  };
}

/**
 * Build the document manager for one host snapshot.
 *
 * `mdiActiveDocument()` is the one operation this build serves: gate, call, decode.
 * `documents()` gates too, but its row is `pending`, so it refuses by name and the
 * host is never asked.
 */
export function createDocumentManager(wire: CadApiWire): DocumentManager {
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
      return createDocument(wire, decodeDocumentDescriptor(document));
    },
    async documents(): Promise<readonly Document[]> {
      wire.assertSupported('ApplicationServices', 'documentManager.documents');
      throw noDocumentListDecoder();
    },
    async executeInApplicationContext<T>(work: () => T | Promise<T>): Promise<T> {
      if (typeof work !== 'function') throw invalidArgument('work');
      return await work();
    },
  };
}

/** Build the application singleton for one host snapshot. */
export function createApplication(wire: CadApiWire): Application {
  return { documentManager: createDocumentManager(wire) };
}
