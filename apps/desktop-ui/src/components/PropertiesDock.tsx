// The properties dock: one slot, two possible owners.
//
// A plugin panel is display-only derived data for a **single** selected entity
// (ADR 0030 D4). It is shown only while the live selection still carries the
// panel's `handle`; any other selection falls back to the ordinary
// `PropertiesPalette`. The stale panel is cleared on that fallback, so simply
// reselecting the same entity does not resurrect a panel the user never asked
// for again — `PROPERTIES` is the one request path.
//
// The dock is placement-agnostic: `PaletteWindow` wraps it on the desktop band and
// `MobilePropertiesSheet` mounts it as the compact layout's bottom sheet. Both
// children therefore receive the same `variant`, so the sheet's surface, head and
// dialog roles are chosen in one place instead of being re-derived here.
import React, { useEffect } from 'react';
import { useShell } from '../context.ts';
import { clearPluginPanel, usePluginPanel } from '../plugins/host.ts';
import { PluginPropertiesPalette } from './PluginPropertiesPalette.tsx';
import { PropertiesPalette } from './PropertiesPalette.tsx';
import type { PropertiesPaletteVariant } from './PropertiesPalette.tsx';
import type { CadSelection } from '../cad/cad-status.ts';

/** The live single-selection provenance handle, or `null` when unavailable. */
function selectedHandle(selection: CadSelection | null): string | null {
  if (!selection || selection.count !== 1) return null;
  const row = selection.rows.find(candidate => candidate.key === 'handle');
  // Source handles are uppercase hex in the importer, but a test/fixture may
  // store a lower-case spelling; compare case-insensitively so the panel is
  // tied to the entity, not to the formatting of one row.
  return row && !row.mixed && typeof row.value === 'string' ? row.value.toUpperCase() : null;
}

export interface PropertiesDockProps {
  /** The desktop palette window, or the compact bottom sheet. */
  readonly variant?: PropertiesPaletteVariant;
}

export function PropertiesDock({ variant = 'window' }: PropertiesDockProps) {
  const shell = useShell();
  const panel = usePluginPanel();
  const selection = (shell.status?.selection ?? null) as CadSelection | null;
  const matches = panel !== null && selectedHandle(selection) === panel.handle.toUpperCase();

  // The panel belongs to one request and one handle. When the selection moves
  // away (or disappears) the window returns to the default palette and the stale
  // panel is dropped; a new `PROPERTIES` re-asks the providers.
  useEffect(() => {
    if (panel !== null && !matches) clearPluginPanel();
  }, [panel, matches]);

  if (panel !== null && matches) {
    return (
      <PluginPropertiesPalette
        panel={panel}
        variant={variant}
        onClose={() => {
          clearPluginPanel();
          shell.runCommand('PROPERTIESCLOSE');
        }}
        onShowDefault={() => { clearPluginPanel(); }}
      />
    );
  }
  return <PropertiesPalette variant={variant} />;
}
