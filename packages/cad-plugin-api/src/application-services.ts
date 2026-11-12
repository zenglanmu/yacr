// The ApplicationServices surface: the application, its documents, and the
// per-document wiring (database, editor, transaction manager).
//
// The synchronous members here (`documentManager.mdiActiveDocument`,
// `documentManager.documents`) are the ones AutoCAD exposes as properties. They go
// through the same capability gate as everything else and never start I/O. In this
// build both rows are `pending`, so reading them refuses by name instead of
// answering `null`/`[]`: an empty list is a claim about the drawing, and this
// package has not been told anything about it.
//
// A host that advertises one of those rows as `supported` must also give this
// client a snapshot channel; there is none in the frozen host info yet, so the
// read refuses with a *client* gap (`unsupported-runtime: cad_api.<ns>.<op>`,
// message says so) — never with a caller error blaming the plugin for a host gap.
//
// Everything that fetches data is a `Promise` and is capability-gated.

import { unsupportedReason } from './abi.ts';
import type { CadApiNamespace } from './abi.ts';
import { createDatabase } from './database-services.ts';
import type { Database } from './database-services.ts';
import { createEditor } from './editor-input.ts';
import type { Editor } from './editor-input.ts';
import { CadApiError, invalidArgument } from './errors.ts';
import type { CadApiWire } from './internal/capability.ts';
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
   * Gated on `ApplicationServices.documentManager.mdiActiveDocument`. A host that
   * reports it `supported` must also publish a snapshot (not part of v1 host
   * info); until then the read refuses, because guessing `null` would be a claim.
   */
  readonly mdiActiveDocument: Document | null;
  /** Every open document; gated the same way as `mdiActiveDocument`. */
  readonly documents: readonly Document[];
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
 */
export interface DocumentDescriptor {
  readonly id: string;
  readonly name: string;
  readonly isReadOnly: boolean;
  readonly handle: string;
  readonly version: string;
}

/** The client-side gap a supported-but-snapshotless document read reports. */
function noSnapshotChannel(namespace: CadApiNamespace, operation: string): CadApiError {
  return new CadApiError(
    unsupportedReason(namespace, operation),
    'this client has no document snapshot channel yet, so it cannot answer a supported descriptor row',
  );
}

/**
 * Materialize a document from a host descriptor.
 *
 * Used by the reference client once a host serves a document snapshot; in this
 * build no host does, so nothing in the package calls this on a plugin's behalf.
 */
export function createDocument(wire: CadApiWire, descriptor: DocumentDescriptor): Document {
  if (typeof descriptor !== 'object' || descriptor === null) throw invalidArgument('descriptor');
  if (typeof descriptor.id !== 'string' || descriptor.id.trim().length === 0) {
    throw invalidArgument('descriptor.id');
  }
  if (typeof descriptor.name !== 'string' || descriptor.name.trim().length === 0) {
    throw invalidArgument('descriptor.name');
  }
  if (typeof descriptor.isReadOnly !== 'boolean') throw invalidArgument('descriptor.isReadOnly');
  const documentId = descriptor.id.trim();
  const database = createDatabase(wire, {
    handle: descriptor.handle,
    version: descriptor.version,
  });
  return {
    name: descriptor.name,
    isReadOnly: descriptor.isReadOnly,
    database,
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
 * The getters gate first, so on a `pending` carrier the refusal is the pending
 * code and the host is never asked. When the host claims support the read still
 * has no snapshot to answer from: that gap is reported as this client's
 * `unsupported-runtime` rather than as a plugin argument error.
 */
export function createDocumentManager(wire: CadApiWire): DocumentManager {
  return {
    get mdiActiveDocument(): Document | null {
      wire.assertSupported('ApplicationServices', 'documentManager.mdiActiveDocument');
      throw noSnapshotChannel('ApplicationServices', 'documentManager.mdiActiveDocument');
    },
    get documents(): readonly Document[] {
      wire.assertSupported('ApplicationServices', 'documentManager.documents');
      throw noSnapshotChannel('ApplicationServices', 'documentManager.documents');
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
