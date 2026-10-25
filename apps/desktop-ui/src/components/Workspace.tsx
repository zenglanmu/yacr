// Workspace: the central CAD partition plus the DOM chrome docked at its edges.
//
// The native CAD surface is composited by the host at exactly the rect this
// element reports, so `.native-input` holds **no** DOM children and nothing
// floats across it. Palettes dock to the left (shrinking the partition) and the
// navigation rail docks to the right.
//
// A `dom-canvas` host (ADR 0024 D3) carries its pixels inside this tree instead:
// an empty, input-transparent `<canvas>` fills the same rect and paints under
// `.native-input` by DOM order. It is a pixel surface only - no listeners, no
// document state - so `.native-input` stays the single input funnel.
import React from 'react';
import { NavRail } from './NavRail.tsx';
import { StartPanel } from './StartPanel.tsx';
import { LayersPalette } from './LayersPalette.tsx';
import { PropertiesPalette } from './PropertiesPalette.tsx';
import { DiagnosticsPalette } from './DiagnosticsPalette.tsx';
import { useI18n, useShell } from '../context.ts';
import { palettesOffered, SURFACE_MODE_DOM_CANVAS, surfaceModeFromShell } from '../surface.ts';

export function Workspace() {
  const { t } = useI18n();
  const shell = useShell();
  const showStart = !shell.documentVisible;
  // Whether the docked palettes exist on the reported carrier. A `dom-canvas`
  // host holds no host-side document state, so it has none of the projections they
  // read: they are not rendered at all rather than as empty panels. The carrier
  // rule is `palettesOffered`, the same helper the nav rail and the status bar
  // ask, so the panels and their toggles can never disagree. An absent flag (older
  // chrome state) keeps them, as today.
  const palettesAvailable = palettesOffered(shell);
  // The inspector is only meaningful with a drawing open; on the start screen the
  // start panel keeps the whole dock. Off by default so the startup partition
  // geometry is unchanged until the user opens it.
  const showProperties = palettesAvailable && shell.propertiesOpen && !showStart;
  const showDiagnostics = palettesAvailable && Boolean(shell.diagnosticsOpen);
  // The layers palette is a document-wide list, not a selection projection, so it
  // is offered on the start page too: with no session the host answers the empty
  // list and the panel says "no layers" rather than disappearing after `LAYER`.
  // Closed by default, so the startup partition geometry is unchanged.
  const showLayers = palettesAvailable && Boolean(shell.layersOpen);
  const leftOpen = showStart || showProperties || showDiagnostics || showLayers;
  // The host hides the system cursor over the CAD partition only while it is
  // drawing its own crosshair (a 2D view with a pointer on the surface). A 3D
  // view reports `false`, so the system arrow stays and the pointer is never
  // invisible. Missing field (older host): the arrow stays, never a guess.
  const crosshair = shell.status?.crosshair === true;
  // The host-reported pixel carrier (ADR 0024 D1/D3). Only an explicit
  // `dom-canvas` report mounts the in-DOM carrier; a missing field (older host)
  // and every other value stay `native-below`, i.e. the compositing this tree has
  // always used, with no extra node in the partition.
  const domCanvas = surfaceModeFromShell(shell) === SURFACE_MODE_DOM_CANVAS;
  return (
    <div className="region region--workspace workspace app-region">
      <div className={['workspace__left', leftOpen ? 'is-open' : ''].filter(Boolean).join(' ')}>
        {showStart ? <StartPanel /> : null}
        {showLayers ? <LayersPalette /> : null}
        {showProperties ? <PropertiesPalette /> : null}
        {showDiagnostics ? <DiagnosticsPalette /> : null}
      </div>
      <div className="workspace__canvas">
        {/* dom-canvas only: the in-DOM pixel carrier. It fills exactly the rect
            the host reports for `.native-input`, paints underneath it (later
            sibling wins) and takes no input, so the crosshair and pointer
            handling are unchanged. The wasm renderer paints its own clear
            colour, so the node deliberately carries no fill. */}
        {domCanvas ? (
          <canvas id="cad-canvas" className="cad-canvas" aria-hidden="true" />
        ) : null}
        {/* The native CAD partition. Keep this node childless. `tabIndex={-1}`
            makes it programmatically focusable only — the shortcut menu hands the
            keyboard back here on Escape/Tab — without adding it to the tab order. */}
        <div
          ref={shell.surfaceRef}
          tabIndex={-1}
          className={['native-input', crosshair ? 'is-crosshair' : ''].filter(Boolean).join(' ')}
          aria-label={t('a11y.canvas_label')}
        />
      </div>
      <NavRail />
    </div>
  );
}
