// Workspace: the central CAD partition plus the DOM chrome around it.
//
// The native CAD surface is composited by the host at exactly the rect this
// element reports, so `.native-input` holds **no** DOM children; overlays are
// siblings that paint above it without moving it. The left column only holds the
// start panel; every palette floats *above* the partition in `.palette-layer`
// (an absolute overlay that takes no layout space). Opening, closing, dragging
// or resizing a palette therefore never changes the `.native-input` rect, so the
// host gets no resize report and never re-presents/re-renders the CAD surface.
// The navigation rail stays docked at the right edge.
//
// A `dom-canvas` host (ADR 0024 D3) carries its pixels inside this tree instead:
// an empty, input-transparent `<canvas>` fills the same rect and paints under
// `.native-input` by DOM order. It is a pixel surface only - no listeners, no
// document state - so `.native-input` stays the single input funnel.
import React from 'react';
import { NavRail } from './NavRail.tsx';
import { StartPanel } from './StartPanel.tsx';
import { PaletteWindow } from './PaletteWindow.tsx';
import { LayersPalette } from './LayersPalette.tsx';
import { PropertiesDock } from './PropertiesDock.tsx';
import { PluginNoticeHost } from './PluginNoticeHost.tsx';
import { HostNoticeHost } from './HostNoticeHost.tsx';
import { UpdateNoticeHost } from './UpdateNoticeHost.tsx';
import { FileInfoPalette } from './FileInfoPalette.tsx';
import { DiagnosticsPalette } from './DiagnosticsPalette.tsx';
import { ReferencesPanel } from './ReferencesPanel.tsx';
import { FontsPanel } from './FontsPanel.tsx';
import { useI18n, useShell } from '../context.ts';
import { palettesOffered, SURFACE_MODE_DOM_CANVAS, surfaceModeFromShell } from '../cad/surface.ts';

export function Workspace() {
  const { t } = useI18n();
  const shell = useShell();
  const showStart = !shell.documentVisible;
  // Whether the desktop palettes exist on the reported carrier. A `dom-canvas`
  // host holds no host-side document state, so it has none of the projections they
  // read: they are not rendered at all rather than as empty panels. The carrier
  // rule is `palettesOffered`, the same helper the nav rail and the status bar
  // ask, so the panels and their toggles can never disagree. An absent flag (older
  // chrome state) keeps them, as today.
  const palettesAvailable = palettesOffered(shell);
  // The inspector is only meaningful with a drawing open; on the start screen the
  // start panel owns the left column. Off by default so the startup partition
  // geometry is unchanged until the user opens it.
  const showProperties = palettesAvailable && shell.propertiesOpen && !showStart;
  // The static File info panel and the live diagnostics panel are independent
  // windows behind the same carrier flag; either can be open without the other.
  const showFileInfo = palettesAvailable && Boolean(shell.fileInfoOpen);
  const showDiagnostics = palettesAvailable && Boolean(shell.diagnosticsOpen);
  // The references panel is a document-wide list like the layers palette: it
  // reads the host's `cad_status.references` rows and floats in the layer.
  const showReferences = palettesAvailable && Boolean(shell.referencesOpen);
  // The Font Manager is a document-wide read-only report like the references
  // panel, so it floats in the same layer and is gated by the same carrier flag.
  const showFonts = palettesAvailable && Boolean(shell.fontsOpen);
  // The layers palette is a document-wide list, not a selection projection, so it
  // is offered on the start page too: with no session the host answers the empty
  // list and the panel says "no layers" rather than disappearing after `LAYER`.
  // Closed by default, so the startup partition geometry is unchanged.
  const showLayers = palettesAvailable && Boolean(shell.layersOpen);
  // The left column is now the start screen's alone. Palettes live in the
  // floating layer below, so no palette flag may widen the partition.
  const leftOpen = showStart;
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
        {/* Floating palettes: an overlay sibling *after* `.native-input`, so it
            paints above the CAD surface and takes no layout space. The layer
            itself is pointer-transparent; each window re-enables input. DOM order
            is the open order below; z-index (owned by `PaletteWindow`) decides
            stacking after a press, with properties starting on top. */}
        <div className="palette-layer">
          {showProperties ? (
            <PaletteWindow windowKey="properties" initialIndex={0}>
              <PropertiesDock />
            </PaletteWindow>
          ) : null}
          {showLayers ? (
            <PaletteWindow windowKey="layers" initialIndex={1}>
              <LayersPalette />
            </PaletteWindow>
          ) : null}
          {showFileInfo ? (
            <PaletteWindow windowKey="fileinfo" initialIndex={2}>
              <FileInfoPalette />
            </PaletteWindow>
          ) : null}
          {showDiagnostics ? (
            <PaletteWindow windowKey="diagnostics" initialIndex={3}>
              <DiagnosticsPalette />
            </PaletteWindow>
          ) : null}
          {showReferences ? (
            <PaletteWindow windowKey="references" initialIndex={4}>
              <ReferencesPanel />
            </PaletteWindow>
          ) : null}
          {showFonts ? (
            <PaletteWindow windowKey="fonts" initialIndex={5}>
              <FontsPanel />
            </PaletteWindow>
          ) : null}
        </div>
      </div>
      <NavRail />
      <PluginNoticeHost />
      <HostNoticeHost />
      <UpdateNoticeHost />
    </div>
  );
}
