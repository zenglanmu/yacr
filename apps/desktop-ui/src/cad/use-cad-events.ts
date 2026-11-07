// The shell's one status subscription (P3): a typed push event stream replaces
// the old `cad_status` poll.
//
// `App.tsx` is at the file-size ceiling, and this is exactly the kind of wiring
// the contract wanted extracted: transport selection, carrier bootstrap, gap
// recovery and the frame pump are all transport behaviour, not region layout.
//
// # The rules this effect owns
//
// * **One producer, two cadences.** A `status-changed` frame is semantic: it feeds
//   `setStatus` (the whole shell tree) and the leaf stores. The per-frame volatile
//   pair travels on its own tiny `cursor-changed` frame instead
//   (`cursorWorld` to the status-bar readout, `frameCount` to the FPS sampler via
//   `cad/status-cadence.ts`); it never feeds `setStatus`, the carrier, or the pump.
//   A transport failure feeds `setPollError` (the historical slot name is kept so
//   the banner precedence in `App.tsx` is unchanged). There is no timer that asks
//   for status in steady state.
// * **Carrier bootstrap, unchanged.** A `tauri` build subscribes through the
//   native event source first; its first snapshot carries
//   `hostProfile.surfaceMode`, which goes to `observeCarrier` once per status. If
//   that answer is `dom-canvas`, the route in `host/invoke.ts` changes and this
//   effect closes the native subscription and re-subscribes through the wasm
//   source. An `android` build starts on the same native route (its host runs the
//   native `CadSession` and answers `native-below`, so nothing flips); a `web`
//   build starts on the wasm route. Both are routed by the same table.
// * **Frames are pumped only by semantic events.** A per-frame `status-changed`
//   or `cursor-changed` event plus a pump would keep asking for frames forever;
//   frames are pumped by commands, selection, tool steps, document changes, host
//   errors and popups - never by the status/cursor cadence.
// * **FPS reads the volatile cadence.** `frameCount` arrives with every
//   `cursor-changed` (and the first status), so the sampler keeps its own
//   cadence; an unchanged counter is handled by `createFpsSampler`, and five
//   seconds without a status/cursor frame resets the reading.
import { useEffect, useRef } from 'react';
import { cadEventRoute, cadEventSource } from '../host/invoke.ts';
import { cadEventPumpsFrames } from './cad-events.ts';
import { observeCarrier } from '../host/runtime.ts';
import { createFpsSampler } from '../fps.ts';
import { reportedSurfaceMode } from './surface.ts';
import { sameStatusIgnoringVolatile } from './status-cadence.ts';
import { runVendorPass } from '../vendor/vendor-pass.ts';
import { dispatchCadStatus, ensurePluginRegistered } from '../plugins/host.ts';
import { recordVendorSettled } from '../plugins/plugin-manager.ts';
import { observeStatusError } from '../host/notice.ts';
import type { InvokeRoute } from '../host/invoke.ts';
import type { CadSubscription, CadEventFrame } from './cad-events.ts';
import type { CadStatus } from './cad-status.ts';

/** The state writes the subscription drives; `pump` is the page carrier's kick. */
export interface CadEventHandlers {
  readonly setStatus: (status: CadStatus | null) => void;
  readonly setPollError: (error: string) => void;
  /** Per-frame cursor slice: the status-bar leaf store, not the shell tree. */
  readonly setCursor: (cursorWorld: CadStatus['cursorWorld']) => void;
  /** Per-frame FPS slice: the diagnostics leaf store, not the shell tree. */
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
    // The last status the shell tree actually received. Only a semantic change
    // replaces it, so the per-frame volatile fields never churn React.
    let lastApplied: CadStatus | null = null;

    const resetFps = (): void => {
      clearTimeout(staleTimer);
      sampler.reset();
      live.current.setFps(null);
    };
    document.addEventListener('visibilitychange', resetFps);

    /** Publish the two volatile fields to their leaf stores (never the shell). */
    const publishVolatile = (
      cursorWorld: CadStatus['cursorWorld'],
      frameCount: number,
      active: boolean,
    ): void => {
      live.current.setCursor(cursorWorld);
      live.current.setFps(sampler.sample(frameCount, performance.now(), active));
      clearTimeout(staleTimer);
      if (active) staleTimer = setTimeout(resetFps, 5000);
    };

    const setStatus = (next: CadStatus): void => {
      live.current.setPollError('');
      // The renderer's pulled-copy fallback rides the ordinary `status.error`
      // string. Observe every semantic snapshot here (the one place they arrive)
      // so a token present on the first status or after a gap recovery reaches
      // the one-shot notice latch; the latch itself keeps this from re-opening.
      observeStatusError(next.error);
      // The two per-frame fields go to their leaf readers every status frame.
      publishVolatile(
        next.cursorWorld,
        next.frameCount,
        Boolean(next && !next.error && !document.hidden),
      );
      // Everything else is semantic state: apply it immediately, and skip the
      // full-shell re-render for a frame that differs only in the volatile pair.
      if (sameStatusIgnoringVolatile(lastApplied, next)) return;
      lastApplied = next;
      live.current.setStatus(next);
    };

    const onEvent = (frame: CadEventFrame): void => {
      if (disposed) return;
      if (frame.type === 'status-changed') {
        // The only carrier observation, exactly once per status snapshot.
        observeCarrier(reportedSurfaceMode({ status: frame.status }));
        setStatus(frame.status);
        // Every snapshot reaches the registered plugin providers verbatim; a
        // handler error is isolated inside the host and never fails the stream.
        dispatchCadStatus(frame.status);
        // Observe first: an X11 host's first answer moves the route to the wasm
        // adapter, and the live subscription must follow in the same turn.
        connect();
        return;
      }
      if (frame.type === 'cursor-changed') {
        // Volatile-only cadence: leaf stores and the stale timer, nothing else.
        // It never touches the shell status, the carrier route or the frame pump.
        publishVolatile(frame.cursorWorld, frame.frameCount, !document.hidden);
        return;
      }
      if (frame.type === 'vendor-pending') {
        // The host armed a pass for the document it just opened. Decoding runs
        // in the WebView plugin; a stale pass is rejected by the host token.
        void runVendorPass(frame).catch((error: unknown) => {
          const text = String(error);
          if (text.includes('vendor.stale_token') || text.includes('vendor.no_pending_pass')) return;
          if (!disposed) live.current.setPollError(`vendor-pass: ${text}`);
        });
        return;
      }
      if (frame.type === 'vendor-settled') {
        // The host settled the pass it armed. Remember applied/diagnostics for
        // the plugin manager's readers; this is an observation, not a pump.
        recordVendorSettled(frame.applied, [...frame.diagnostics]);
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

    // Load and register the vendor plugin once. Its optional `register` is the
    // only startup-provider channel (ADR 0030 D3); a missing module or a throwing
    // `register` is one diagnostic line, never a crash or a blank session.
    void ensurePluginRegistered().catch((error: unknown) => {
      const text = String(error);
      // A deliberately disabled decoder is a configuration, not a failure.
      if (text.includes('vendor.decoder_disabled')) return;
      if (!disposed) live.current.setPollError(`vendor-plugin: ${text}`);
    });

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
