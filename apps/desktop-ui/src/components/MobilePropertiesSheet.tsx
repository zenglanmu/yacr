// The compact properties sheet: the desktop inspector as a bottom sheet.
//
// The compact layout does not render the desktop palette layer (`.palette-layer`
// is hidden by CSS), and the properties inspector is the one palette the layout
// keeps: this layer mounts the **same** `PropertiesDock` — one data source
// (`cad_status.selection`), one edit state machine, one write funnel — with
// `variant="floating"`, so the sheet is a placement and not a second panel.
//
// It follows the references/fonts/BIM sheets exactly: a fixed layer with a real
// backdrop button (a press closes it), the panel at the bottom edge, a 44px head
// with a 44px close, an internally scrolling body and the gesture-bar inset
// (`mobile.css`). The layer is mounted only while `propertiesOpen` is true, so it
// costs the drawing nothing when closed and never races a second instance: the
// desktop window is gated off on this layout (`Workspace`).
//
// The close paths all land on the one flag: the head's button runs
// `PROPERTIESCLOSE` (the same chrome-local selector the desktop uses), and the
// backdrop calls the setter directly. No second open state lives here.
import React from 'react';
import { useI18n, useShell } from '../context.ts';
import { PropertiesDock } from './PropertiesDock.tsx';

export function MobilePropertiesSheet() {
  const { t } = useI18n();
  const shell = useShell();
  return (
    <div className="properties-layer">
      <button
        type="button"
        className="properties-layer__backdrop"
        aria-label={t('properties.close')}
        onClick={() => shell.setPropertiesOpen(false)}
      />
      <PropertiesDock variant="floating" />
    </div>
  );
}
