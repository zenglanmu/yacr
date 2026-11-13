// Transactions: the only write path in the API.
//
// Rules this file enforces **client-side** (the host enforces them again on its
// side of the wire; neither is optional):
//
//   * nesting is strictly LIFO for `commit`/`abort` — only the innermost open
//     transaction may close, otherwise `api.transaction.nesting_order`. Reads
//     (`getObject`) and buffered writes (`addNewlyCreatedDBObject`) are allowed at
//     any depth: opening or staging inside an inner transaction is not a close.
//   * at most `MAX_TRANSACTION_DEPTH` (8) open transactions, otherwise
//     `api.transaction.nesting_limit`;
//   * a transaction closes once — a second commit/abort is
//     `api.transaction.closed`;
//   * writes accumulate in a per-transaction buffer. A nested commit is a
//     **savepoint release**: it merges its edits into the parent and applies
//     nothing, so it reports `{ applied: false, revision: null, created: [] }` —
//     there is no revision to report and none is invented. A nested abort throws
//     its buffer away. Only the outermost commit **applies** anything, as one
//     request on the mutating lane (`wire.submit`) carrying
//     `{ document, transactionId, expectedRevision, edits }`.
//   * `expectedRevision` is **required**: an omitted precondition would be silent
//     last-writer-wins, so the caller reads `database.revision()` first and says
//     which base it edits.
//   * the commit is asynchronous because the wire is asynchronous, and it resolves
//     only after the host reported the outcome — never optimistically.
//   * optimistic concurrency: the host rejects with `api.revision-conflict` when
//     `expectedRevision` is stale, and that code (and any `details.actualRevision`
//     the host supplied) reaches the plugin unchanged.
//
// # What is local and what is not
//
// `transaction.start`, `transaction.addNewlyCreated` and `transaction.abort` are
// **client-local** operations: the host never mints a transaction handle, so
// opening, staging into, and discarding a transaction all happen in this module's
// own stack and buffers. They therefore have **no capability rows** and are not
// gated — refusing them would claim a host round-trip that does not exist.
//
// `getObject` is not local: it reads the **live session**, so it travels on the
// same mutating (submit/poll) lane, with `document` and `transactionId` in its
// parameters. `commit` (outermost) is the one operation that *changes* the drawing;
// a future host-minted transaction would add a row and a wire call back to
// start/abort; until then the local behaviour is the honest one.
//
// Buffers live in a module-local `WeakMap` rather than on the instance: no public
// member may add edits except `addNewlyCreatedDBObject`, and the manager performs
// the savepoint merge when a nested transaction is released.

import type { DBObject, ObjectId, OpenMode } from './database-services.ts';
import { decodeObject, objectId } from './database-services.ts';
import { CadApiError, invalidArgument } from './errors.ts';
import type { Vector3d } from './geometry.ts';
import type { CadApiWire } from './internal/capability.ts';
import { errorFromBridgeFailure, requireAnswerObject } from './internal/capability.ts';

/** The maximum number of simultaneously open transactions. */
export const MAX_TRANSACTION_DEPTH = 8;

/**
 * The entity types this build's commit envelope can create.
 *
 * The host's `NewEntity` is a closed set (`Line`/`Circle`/`Arc`); anything else is
 * refused locally with `api.invalid_argument: typeName` instead of being sent and
 * refused later.
 */
export type NewlyCreatedEntityType = 'Line' | 'Circle' | 'Arc';

/** The closed set of creatable entity types, in host order. */
export const SUPPORTED_NEW_ENTITY_TYPES: readonly NewlyCreatedEntityType[] = Object.freeze([
  'Line',
  'Circle',
  'Arc',
]);

const NEW_ENTITY_TYPES: ReadonlySet<string> = new Set<NewlyCreatedEntityType>(
  SUPPORTED_NEW_ENTITY_TYPES,
);

function isNewlyCreatedEntityType(value: string): value is NewlyCreatedEntityType {
  return NEW_ENTITY_TYPES.has(value);
}

/**
 * One buffered write.
 *
 * `insert` names the entity type, the target layer **name**, the target space
 * (a host-defined space id/name) and the host's `NewEntity` parameters
 * (`start`/`end`, `center`/`radius`, or `center`/`radius`/`startAngle`/`endAngle`).
 * `translate`/`delete` are part of the frozen wire shape; the host maps them to a
 * `SelectionRef { document, entity, instance, sub_element }` and refuses instanced
 * or sub-element references.
 */
export type TransactionEdit =
  | {
      readonly kind: 'insert';
      readonly typeName: NewlyCreatedEntityType;
      readonly layer: string;
      readonly space: string;
      readonly parameters: Readonly<Record<string, unknown>>;
    }
  | { readonly kind: 'translate'; readonly objectId: ObjectId; readonly displacement: Vector3d }
  | { readonly kind: 'delete'; readonly objectId: ObjectId };

/**
 * What a commit reports.
 *
 * `applied` is the load-bearing field: only the outermost commit applies anything.
 * `revision` is the host's new revision (never a locally invented number) and
 * `created` is the host's `ApiOutcome.created` — the ids of the entities this
 * commit actually created, in host order.
 */
export interface CommitResult {
  readonly applied: boolean;
  readonly revision: number | null;
  readonly created: readonly ObjectId[];
}

/** Options accepted by `TransactionManager.startTransaction`. */
export interface TransactionOptions {
  /**
   * The revision the caller believes is current. **Required** for the outermost
   * transaction: the host applies the commit only if it agrees, otherwise
   * `api.revision-conflict`. Read the current revision first (M2) instead of
   * omitting it — an absent precondition is silent last-writer-wins.
   */
  readonly expectedRevision: number;
}

/** An object a transaction is about to create (no id exists yet). */
export interface NewlyCreatedDBObject {
  readonly typeName: string;
  /** The target layer's **name**; the host resolves it to a layer id. */
  readonly layer: string;
  /** The target space (host-defined id/name); it must match the active space. */
  readonly space: string;
  /** The host's `NewEntity` parameters for `typeName`. */
  readonly parameters: Readonly<Record<string, unknown>>;
}

/**
 * Per-transaction wiring supplied by the manager. Not part of the plugin
 * surface: it exists so `Transaction` never reaches into manager internals.
 */
export interface TransactionWiring {
  readonly wire: CadApiWire;
  /** The document id the commit addresses (one manager per document). */
  readonly document: string;
  /** One commit correlation id, client-invented; not a host transaction handle. */
  readonly id: string;
  readonly depth: number;
  readonly parent: Transaction | null;
  readonly expectedRevision: number;
  isTop(transaction: Transaction): boolean;
  /** Pop the transaction; `commit` merges a nested buffer into its parent first. */
  release(transaction: Transaction, disposition: 'commit' | 'abort'): void;
}

/**
 * Edit buffers, keyed by transaction and owned by this module.
 *
 * Keeping them out of the instance is deliberate: there is no public member that
 * could push an edit, so every edit a plugin records went through
 * `addNewlyCreatedDBObject`.
 */
const EDIT_BUFFERS = new WeakMap<Transaction, TransactionEdit[]>();

function bufferOf(transaction: Transaction): TransactionEdit[] {
  const buffer = EDIT_BUFFERS.get(transaction);
  if (buffer === undefined) {
    // Only a `Transaction` built by the constructor is ever a key, so this is a
    // package bug, not a plugin error.
    throw new Error('transaction edit buffer missing');
  }
  return buffer;
}

/**
 * Push one edit, mirroring the host's duplicate rule.
 *
 * A single commit batch may not touch one entity twice (the host keeps a set of
 * edited entity ids), so a second `translate`/`delete` for the same `ObjectId` in
 * one transaction is `api.invalid_argument: edits.duplicate` — refused locally and
 * identically, before anything is sent.
 *
 * @internal Exported for the transaction suite; not part of the package surface.
 */
export function appendEdit(transaction: Transaction, edit: TransactionEdit): void {
  pushChecked(bufferOf(transaction), edit);
}

function pushChecked(buffer: TransactionEdit[], edit: TransactionEdit): void {
  if (edit.kind !== 'insert') {
    for (const existing of buffer) {
      if (existing.kind !== 'insert' && existing.objectId === edit.objectId) {
        throw new CadApiError(
          'api.invalid_argument: edits.duplicate',
          `one transaction may not edit ${edit.objectId} more than once`,
        );
      }
    }
  }
  buffer.push(edit);
}

function mergeEdits(target: TransactionEdit[], source: readonly TransactionEdit[]): void {
  for (const edit of source) pushChecked(target, edit);
}

function closedError(id: string): CadApiError {
  return new CadApiError('api.transaction.closed', `transaction ${id} is already closed`, {
    operation: 'DatabaseServices.transaction',
  });
}

function nestingOrderError(id: string): CadApiError {
  return new CadApiError(
    'api.transaction.nesting_order',
    `transaction ${id} is not the innermost open transaction`,
    { operation: 'DatabaseServices.transaction' },
  );
}

/**
 * Re-shape a `revision-conflict` rejection so the plugin sees the stable code and
 * whatever `actualRevision` the host supplied. Every other failure passes
 * through untouched.
 *
 * The Rust contract answers a stale commit with `CadError::StaleResult`, which
 * carries no actual revision: `details` is then simply absent, and M3 decides how
 * a revision is obtained.
 */
function mapCommitFailure(failure: unknown): CadApiError {
  const error = errorFromBridgeFailure(failure);
  if (error.code !== 'api.revision-conflict') return error;
  return new CadApiError('api.revision-conflict', error.message, {
    operation: 'DatabaseServices.transaction.commit',
    details: error.details,
    cause: error,
  });
}

function decodeCreated(value: unknown): readonly ObjectId[] {
  if (!Array.isArray(value)) throw invalidArgument('bridge.answer.created');
  return value.map((entry) => {
    if (typeof entry !== 'string' || entry.trim().length === 0) {
      throw invalidArgument('bridge.answer.created');
    }
    return objectId(entry);
  });
}

/** One open database transaction. Create it through `TransactionManager`. */
export class Transaction {
  readonly #wiring: TransactionWiring;
  #closed = false;

  /** @internal Use `TransactionManager.startTransaction`. */
  constructor(wiring: TransactionWiring) {
    this.#wiring = wiring;
    EDIT_BUFFERS.set(this, []);
  }

  /** 1 for the outermost transaction, increasing inwards. */
  get depth(): number {
    return this.#wiring.depth;
  }

  /** Whether this transaction has already been committed or aborted. */
  get closed(): boolean {
    return this.#closed;
  }

  /**
   * Open one object inside this transaction, at any depth.
   *
   * Goes over the **mutating** lane (`wire.submit`): the host reads the live
   * session rather than the reflected snapshot, so the answer is polled like a
   * mutation. The host's `typeName` tag picks the decoder: the result is a
   * discriminated union (`Line`/`Circle`/`Arc`/`Polyline`/`DBText`/`MText`), never
   * a caller-chosen type forced onto an unvalidated answer. `write`/`notify` do not
   * change the commit payload: edits are recorded explicitly, so opening an object
   * never implies a write.
   */
  async getObject(id: ObjectId, mode: OpenMode): Promise<DBObject> {
    if (this.#closed) throw closedError(this.#wiring.id);
    const identifier = objectId(id);
    if (mode !== 'read' && mode !== 'write' && mode !== 'notify') throw invalidArgument('mode');
    // A live-session read: the submit lane, carrying the document the client is
    // editing as well as the transaction correlation id.
    const answer = await this.#wiring.wire.submit('DatabaseServices', 'transaction.getObject', {
      document: this.#wiring.document,
      transactionId: this.#wiring.id,
      objectId: identifier,
      mode,
    });
    const record = requireAnswerObject(answer, 'bridge.answer');
    const object = record['object'];
    if (object === undefined || object === null) throw invalidArgument('bridge.answer.object');
    return decodeObject(requireAnswerObject(object, 'bridge.answer.object'));
  }

  /**
   * Buffer a newly created object so the commit creates it.
   *
   * `add === true` buffers the insert; `add === false` records nothing (the object
   * exists only in plugin memory). The type must be in the host's closed set
   * (`Line`/`Circle`/`Arc`), the layer is a layer **name**, and the space must be
   * the active space (the host refuses a mismatch). Allowed at any depth and
   * **client-local**: staging an edit is not a host operation, so nothing is gated
   * or sent until the outermost commit.
   */
  addNewlyCreatedDBObject(object: NewlyCreatedDBObject, add: boolean): void {
    if (this.#closed) throw closedError(this.#wiring.id);
    if (typeof object !== 'object' || object === null) throw invalidArgument('object');
    const typeName = object.typeName;
    if (typeof typeName !== 'string' || !isNewlyCreatedEntityType(typeName)) {
      throw invalidArgument('typeName');
    }
    if (typeof object.layer !== 'string' || object.layer.trim().length === 0) {
      throw invalidArgument('layer');
    }
    if (typeof object.space !== 'string' || object.space.trim().length === 0) {
      throw invalidArgument('space');
    }
    const parameters = object.parameters;
    if (typeof parameters !== 'object' || parameters === null || Array.isArray(parameters)) {
      throw invalidArgument('parameters');
    }
    if (typeof add !== 'boolean') throw invalidArgument('add');
    if (!add) return;
    appendEdit(this, {
      kind: 'insert',
      typeName,
      layer: object.layer.trim(),
      space: object.space.trim(),
      parameters,
    });
  }

  /**
   * Commit this transaction.
   *
   * An outermost commit applies one batch and resolves with the host's new
   * revision plus the ids it created; a nested commit is a savepoint release and
   * resolves `{ applied: false, revision: null, created: [] }`.
   */
  async commit(): Promise<CommitResult> {
    const wiring = this.#wiring;
    if (this.#closed) throw closedError(wiring.id);
    if (!wiring.isTop(this)) throw nestingOrderError(wiring.id);

    if (wiring.parent !== null) {
      // Savepoint release: merge into the parent, apply nothing, invent nothing.
      wiring.release(this, 'commit');
      return { applied: false, revision: null, created: [] };
    }

    // The capability refusal is local: it happens before the host is asked, so it
    // must not close a transaction that could still commit on a served carrier.
    wiring.wire.assertSupported('DatabaseServices', 'transaction.commit');

    let answer: unknown;
    try {
      // The mutating lane: the host queues and reports, so this resolves only
      // once the commit actually happened.
      answer = await wiring.wire.submit('DatabaseServices', 'transaction.commit', {
        document: wiring.document,
        transactionId: wiring.id,
        expectedRevision: wiring.expectedRevision,
        edits: [...bufferOf(this)],
      });
    } catch (failure) {
      this.#closed = true;
      wiring.release(this, 'abort');
      throw mapCommitFailure(failure);
    }

    this.#closed = true;
    let applied: { readonly revision: number; readonly created: readonly ObjectId[] };
    try {
      const record = requireAnswerObject(answer, 'bridge.answer');
      const revision = record['revision'];
      if (typeof revision !== 'number' || !Number.isFinite(revision) || revision < 0) {
        throw invalidArgument('bridge.answer.revision');
      }
      applied = { revision, created: decodeCreated(record['created']) };
    } catch (failure) {
      // The host answered, so the transaction is over even though the answer is
      // unusable: close it, then refuse. No revision is fabricated.
      wiring.release(this, 'abort');
      throw failure;
    }
    wiring.release(this, 'commit');
    return { applied: true, revision: applied.revision, created: applied.created };
  }

  /**
   * Abort this transaction: discard its buffer, close it, touch nothing remote.
   *
   * Local by design in this build — the host never minted a transaction handle, so
   * there is nothing remote to roll back and a remote abort would always refuse.
   * It has no capability row; a future host-minted transaction would add one and a
   * wire call here.
   */
  async abort(): Promise<void> {
    const wiring = this.#wiring;
    if (this.#closed) throw closedError(wiring.id);
    if (!wiring.isTop(this)) throw nestingOrderError(wiring.id);
    this.#closed = true;
    wiring.release(this, 'abort');
  }
}

/**
 * Owns the open-transaction stack for one document.
 *
 * A manager is per document (it belongs to a `Document`): the nesting order, the
 * depth ceiling and the commit's `document` field are scoped to one database.
 */
export class TransactionManager {
  readonly #wire: CadApiWire;
  readonly #document: string;
  readonly #prefix: string;
  #stack: Transaction[] = [];
  #expected: number[] = [];
  #nextId = 1;

  constructor(wire: CadApiWire, document: string, prefix: string = 'tx') {
    if (typeof document !== 'string' || document.trim().length === 0) {
      throw invalidArgument('document');
    }
    this.#wire = wire;
    this.#document = document.trim();
    this.#prefix = prefix;
  }

  /** The document every commit from this manager addresses. */
  get document(): string {
    return this.#document;
  }

  /** How many transactions are currently open. */
  get depth(): number {
    return this.#stack.length;
  }

  /**
   * Open a transaction. Synchronous and **client-local**: nothing crosses the wire.
   *
   * The host never mints a transaction handle, so opening one is this module's own
   * bookkeeping and is not gated (there is no `transaction.start` row to ask
   * about). The outermost call **requires** `expectedRevision`; a nested call
   * inherits the outer precondition (there is still one commit). Only `commit`
   * reaches the host, and only then does the carrier's capability matter.
   */
  startTransaction(options?: TransactionOptions): Transaction {
    if (this.#stack.length >= MAX_TRANSACTION_DEPTH) {
      throw new CadApiError(
        'api.transaction.nesting_limit',
        `at most ${MAX_TRANSACTION_DEPTH} transactions may be open at once`,
        { operation: 'DatabaseServices.transaction.start' },
      );
    }
    const parent = this.#stack.length > 0 ? this.#stack[this.#stack.length - 1] : null;
    const expectedRevision =
      parent === null ? readExpectedRevision(options) : this.#expected[this.#expected.length - 1];
    const id = `${this.#prefix}-${this.#nextId}`;
    this.#nextId += 1;

    const transaction = new Transaction({
      wire: this.#wire,
      document: this.#document,
      id,
      depth: this.#stack.length + 1,
      parent,
      expectedRevision,
      isTop: (candidate) => this.#stack[this.#stack.length - 1] === candidate,
      release: (candidate, disposition) => {
        const index = this.#stack.indexOf(candidate);
        if (index < 0) return;
        if (disposition === 'commit' && index > 0) {
          // Savepoint release: the parent absorbs the child's edits, under the
          // same duplicate rule the host will apply to the merged batch.
          mergeEdits(bufferOf(this.#stack[index - 1]), bufferOf(candidate));
        }
        this.#stack.splice(index, 1);
        this.#expected.splice(index, 1);
        EDIT_BUFFERS.delete(candidate);
      },
    });

    this.#stack.push(transaction);
    this.#expected.push(expectedRevision);
    return transaction;
  }
}

function readExpectedRevision(options: TransactionOptions | undefined): number {
  if (options === undefined || typeof options !== 'object' || options === null) {
    throw invalidArgument('expectedRevision');
  }
  const expected = options.expectedRevision;
  if (typeof expected !== 'number' || !Number.isSafeInteger(expected) || expected < 0) {
    throw invalidArgument('expectedRevision');
  }
  return expected;
}
