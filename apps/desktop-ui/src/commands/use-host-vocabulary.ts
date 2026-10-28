// The host's command vocabulary and alias table, loaded once and reloaded when
// the carrier flips to `dom-canvas` (an X11 host's first native answer is its
// own table, which that carrier cannot execute).
//
// Extracted from `App.tsx` (file-size ceiling). Two facts matter to the caller:
//
// * `vocabulary` is the **in-flight promise**, awaited by admission, so a command
//   typed before the host answers is admitted once the answer arrives instead of
//   being refused as unknown;
// * a failed load leaves the catalog empty (every command then explicitly
//   unsupported) and reports the failure — the loader retries a transient
//   transport failure first, but never invents a vocabulary.
import { useEffect, useRef, useState } from 'react';
import { invoke } from '../host/invoke.ts';
import { subscribeCarrier, usesWasmCad } from '../host/runtime.ts';
import { setCommandAliases, setCommandCatalog } from './commands.ts';
import type { CommandAlias, CommandAliasInput } from './commands.ts';
import { loadCommandVocabulary } from './vocabulary.ts';

/** What the chrome reads: the two tables plus the promise admission awaits. */
export interface HostVocabulary {
  readonly catalogNames: readonly string[];
  readonly aliasPairs: readonly CommandAlias[];
  readonly vocabulary: React.MutableRefObject<Promise<unknown> | null>;
  /** Bumped when the observed carrier becomes `dom-canvas`. */
  readonly carrierEpoch: number;
}

/**
 * Track the observed carrier and load (or reload) the host's vocabulary.
 *
 * `onError` receives the final `cad_commands` failure, verbatim, exactly once per
 * failed load. The alias table is suggestion metadata: its failure is silent
 * because admission canonicalises whatever is typed — nothing is fabricated.
 */
export function useHostVocabulary(onError: (message: string) => void): HostVocabulary {
  const [catalogNames, setCatalogNames] = useState<readonly string[]>([]);
  const [aliasPairs, setAliasPairs] = useState<readonly CommandAlias[]>([]);
  const [carrierEpoch, setCarrierEpoch] = useState(0);
  const vocabulary = useRef<Promise<unknown> | null>(null);

  // A carrier flip to `dom-canvas` invalidates what the native host answered: a
  // vocabulary only its own CAD session could execute, and that carrier has none.
  useEffect(() => subscribeCarrier(() => {
    if (usesWasmCad()) setCarrierEpoch(epoch => epoch + 1);
  }), []);

  useEffect(() => {
    let cancelled = false;
    const pending = loadCommandVocabulary(() => invoke('cad_commands'));
    vocabulary.current = pending;
    pending.then(answer => {
      if (cancelled) return;
      // The vocabulary is validated by `setCommandCatalog` itself; the cast only
      // states what the host answers with.
      setCatalogNames(setCommandCatalog(answer as Iterable<unknown>));
    }).catch(reason => {
      if (cancelled) return;
      setCatalogNames(setCommandCatalog([]));
      onError(`cad_commands: ${String(reason)}`);
    });
    invoke('cad_command_aliases').then(answer => {
      if (cancelled) return;
      setAliasPairs(setCommandAliases(answer as Iterable<CommandAliasInput>));
    }).catch(() => {
      if (cancelled) return;
      setAliasPairs(setCommandAliases([]));
    });
    return () => { cancelled = true; };
  }, [carrierEpoch, onError]);

  return { catalogNames, aliasPairs, vocabulary, carrierEpoch };
}
