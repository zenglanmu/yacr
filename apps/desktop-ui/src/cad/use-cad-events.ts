// The shell's one status subscription (P3): a typed push event stream replaces
// the old `cad_status` poll.
//
// `App.tsx` is at the file-size ceiling, and this is exactly the kind of wiring
// the contract wanted extracted: transport selection, carrier bootstrap, gap
// recovery and the frame pump are all transport behaviour, not region layout.
//
// # The rules this effect owns
//
// * **One producer.** Every `status-changed` frame feeds `setStatus`; a transport
//   failure feeds `setPollError` (the historical slot name is kept so the banner
//   precedence in `App.tsx` is unchanged). There is no timer that asks for status
//   in steady state.
// * **Carrier bootstrap, unchanged.** A `tauri` build subscribes through the
//   native event source first; its first snapshot carries
//   `hostProfile.surfaceMode`, which goes to `observeCarrier` once per status. If
//   that answer is `dom-canvas`, the route in `host/invoke.ts` changes and this
//   effect closes the native subscription and re-subscribes through the wasm
//   source. An `android` build starts on the same native route (its host runs the
//   native `CadSession` and answers `native-below`, so nothing flips); a `web`
//   build starts on the wasm route. Both are routed by the same table.
// * **Frames are pumped only by semantic events.** A per-frame `status-changed`
//   event plus a pump would keep asking for frames forever; frames are pumped by
//   commands, selection, tool steps, document changes, host errors and popups -
//   never by a status frame.
// * **FPS reads the snapshot.** `status.frameCount` arrives with every frame's
//   status, so the sampler keeps its own cadence; an unchanged counter is handled
//   by `createFpsSampler`, and five seconds without a status resets the reading.
import { useEffect, useRef } from 'react';
import { cadEventRoute, cadEventSource } from '../host/invoke.ts';
import { cadEventPumpsFrames } from './cad-events.ts';
import { observeCarrier } from '../host/runtime.ts';
import { createFpsSampler } from '../fps.ts';
import { reportedSurfaceMode } from './surface.ts';
import type { InvokeRoute } from '../host/invoke.ts';
import type { CadSubscription, CadEventFrame } from './cad-events.ts';
import type { CadStatus } from './cad-status.ts';

/** The state writes the subscription drives; `pump` is the page carrier's kick. */
export interface CadEventHandlers {
  readonly setStatus: (status: CadStatus | null) => void;
  readonly setPollError: (error: string) => void;
  readonly setFps: (fps: number | null) => void;
  readonly pump: () => void;
}

/**
 * Subscribe to the host's typed event stream for the life of the shell.
 *
 * One effect, one subscription (replaced only when the observed carrier moves the
 * transport). Handlers live in a ref so a re-render never resubscribes.
 */
export function useCadEvents(handlers: CadEventHandlers): void {
  const live = useRef(handlers);
  useEffect(() => { live.current = handlers; }, [handlers]);

  useEffect(() => {
    const sampler = createFpsSampler();
    let disposed = false;
    let subscription: CadSubscription | null = null;
    let transport: InvokeRoute | null = null;
    let connecting = false;
    let staleTimer: ReturnType<typeof setTimeout> | undefined;

    const resetFps = (): void => {
      clearTimeout(staleTimer);
      sampler.reset();
      live.current.setFps(null);
    };
    document.addEventListener('visibilitychange', resetFps);

    const setStatus = (next: CadStatus): void => {
      live.current.setStatus(next);
      live.current.setPollError('');
      clearTimeout(staleTimer);
      live.current.setFps(sampler.sample(
        next.frameCount,
        performance.now(),
        Boolean(next && !next.error && !document.hidden),
      ));
      if (!next.error && !document.hidden) staleTimer = setTimeout(resetFps, 5000);
    };

    const onEvent = (frame: CadEventFrame): void => {
      if (disposed) return;
      if (frame.type === 'status-changed') {
        // The only carrier observation, exactly once per status snapshot.
        observeCarrier(reportedSurfaceMode({ status: frame.status }));
        setStatus(frame.status);
        // Observe first: an X11 host's first answer moves the route to the wasm
        // adapter, and the live subscription must follow in the same turn.
        connect();
        return;
      }
      if (cadEventPumpsFrames(frame.type)) live.current.pump();
    };

    const onError = (error: string): void => {
      if (!disposed) live.current.setPollError(error);
    };

    function connect(): void {
      if (disposed || connecting) return;
      const route = cadEventRoute();
      if (route === transport && subscription !== null) return;
      subscription?.close();
      transport = route;
      connecting = true;
      subscription = cadEventSource(route).subscribe(onEvent, onError);
      connecting = false;
      // A snapshot delivered synchronously during `subscribe` may already have
      // changed the route; follow it now instead of waiting for the next frame.
      if (!disposed && cadEventRoute() !== transport) connect();
    }

    connect();
    return () => {
      disposed = true;
      subscription?.close();
      subscription = null;
      clearTimeout(staleTimer);
      document.removeEventListener('visibilitychange', resetFps);
    };
  }, []);
}
