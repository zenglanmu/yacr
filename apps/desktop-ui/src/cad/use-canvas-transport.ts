// The `dom-canvas` transport effect, split out of `App.tsx` so the shell's
// composition file stays inside the 1000-line code gate.
//
// It is the same effect the shell always ran: on a page carrier, load the wasm
// module, create one transport over the CAD partition, drive its frame pump,
// re-report the partition rect on layout shifts, and publish the session's
// drawing identity back to the host through `canvas_session_status`. A module
// that cannot load is classified here and reported once as
// `canvas_runtime_failed`; the host owns the stable marker and the exit code.
//
// Nothing else on the page carrier is allowed to instantiate a second session
// (iron rule 8): this hook creates exactly one transport per mount, disposes it
// on unmount, and never falls back to a blank canvas.
import { useEffect } from 'react';
import type { MutableRefObject } from 'react';
import { invoke } from '../host/invoke.ts';
import {
  CANVAS_STEP_MODULE,
  canvasModuleUrl,
  createCanvasTransport,
  loadCanvasModule,
  watchCanvasRect,
} from './canvas-host.ts';
import type { CanvasFailure, CanvasTransport } from './canvas-host.ts';
import { createFrameLoop } from './frame-loop.ts';
import type { FrameLoop } from './frame-loop.ts';
import { rectFromElement } from './metrics.ts';
import {
  SURFACE_MODE_DOM_CANVAS,
  canvasSessionPayload,
  createCanvasSessionReporter,
} from './surface.ts';
import type { SurfaceModeReading } from './surface.ts';

/** What the transport effect needs from the shell. All setters are stable. */
export interface CanvasTransportHost {
  readonly surfaceMode: SurfaceModeReading;
  readonly surfaceNode: HTMLDivElement | null;
  readonly framesRef: MutableRefObject<FrameLoop | null>;
  readonly setCanvasLive: (live: boolean) => void;
  readonly setError: (message: string) => void;
}

/**
 * Mount the one `dom-canvas` transport while the page carrier is in force.
 *
 * On every other carrier (including an older host that reports `unknown`) this
 * is a no-op: the native host already owns the surface and its frame loop.
 */
export function useCanvasTransport({
  surfaceMode,
  surfaceNode,
  framesRef,
  setCanvasLive,
  setError,
}: CanvasTransportHost): void {
  useEffect(() => {
    if (surfaceMode !== SURFACE_MODE_DOM_CANVAS || !surfaceNode) return undefined;
    let cancelled = false;
    let transport: CanvasTransport | null = null;
    let frames: FrameLoop | null = null;
    let stopWatching: (() => void) | null = null;
    // The one channel that publishes *this* session's drawing identity to the
    // host. The native shell must not hold document state of its own (iron rule
    // 8), so the identity travels one way only: the session reports it, the host
    // stores the relay, the chrome reads the relay back.
    const reporter = createCanvasSessionReporter(report => {
      void invoke('canvas_session_status', { ...canvasSessionPayload(report) })
        .catch(reason => setError(String(reason)));
    });
    const fail = (failure: CanvasFailure) => {
      // The host owns the refusal: it maps the step to its stable marker line and
      // exit code (`webgl2` -> `yacr: unsupported-runtime: webgl2`, exit code 6).
      // This side forwards the machine-readable step and the session's own detail
      // only; it never invents user-facing copy.
      void invoke('canvas_runtime_failed', { step: failure.step, detail: failure.detail })
        .catch(reason => setError(String(reason)));
    };
    void loadCanvasModule(canvasModuleUrl()).then(module => {
      if (cancelled) return;
      const created = createCanvasTransport({
        module,
        element: surfaceNode,
        rect: rectFromElement(surfaceNode, window.devicePixelRatio || 1),
        onFailure: fail,
        onStatus: next => {
          reporter.report(next);
          // The first answered call is what makes the session live; the pointer
          // lane and the frame pump both wait for exactly this fact.
          setCanvasLive(true);
        },
      });
      transport = created;
      frames = createFrameLoop({ module });
      framesRef.current = frames;
      stopWatching = watchCanvasRect(surfaceNode, rect => {
        created.update(rect);
        frames?.pump();
      });
      frames.pump();
    }).catch(error => {
      if (cancelled) return;
      fail({ step: CANVAS_STEP_MODULE, detail: String(error), marker: null });
    });
    return () => {
      cancelled = true;
      stopWatching?.();
      frames?.dispose();
      if (framesRef.current === frames) framesRef.current = null;
      transport?.dispose();
      setCanvasLive(false);
    };
  }, [surfaceMode, surfaceNode, framesRef, setCanvasLive, setError]);
}
